//! Undo/redo system for text buffer operations.
//!
//! Replaces the Python `UndoSequence`/`GroupAction` from the original Meld.
//! Mirrors `meld/undo.py`: a cursor-based action list with per-buffer
//! checkpoints that drive each buffer's modified state.

use std::cell::RefCell;
use std::collections::HashMap;

use gtk4::prelude::*;
use sourceview5 as gsv;

/// Errors related to undo operations.
#[derive(Debug, thiserror::Error)]
pub enum UndoError {
    #[error("Nothing to undo")]
    NothingToUndo,
    #[error("Nothing to redo")]
    NothingToRedo,
}

/// A single reversible action tied to a pane (identified by its index).
pub trait UndoAction {
    /// Pane index this action operates on.
    fn key(&self) -> usize;
    /// Reverse the action.
    fn undo(&self);
    /// Re-apply the action.
    fn redo(&self);
}

/// Combines several actions into one logical action (a "group").
struct GroupAction {
    key: usize,
    actions: Vec<Box<dyn UndoAction>>,
}

impl GroupAction {
    fn new(actions: Vec<Box<dyn UndoAction>>) -> Self {
        let key = actions.first().map(|a| a.key()).unwrap_or(0);
        Self { key, actions }
    }
}

impl UndoAction for GroupAction {
    fn key(&self) -> usize {
        self.key
    }

    fn undo(&self) {
        for action in self.actions.iter().rev() {
            action.undo();
        }
    }

    fn redo(&self) {
        for action in &self.actions {
            action.redo();
        }
    }
}

/// A checkpoint: `(start, end)` positions within the action list.
/// `None` start means "no checkpoint"; `None` end means "end of list".
type Checkpoint = (Option<usize>, Option<usize>);

type CheckpointedCallback = Box<dyn Fn(usize, bool)>;
type CanUndoCallback = Box<dyn Fn(bool)>;
type CanRedoCallback = Box<dyn Fn(bool)>;

/// Manages undo/redo across one or more text buffers (panes).
///
/// Unlike a stack-based design, this keeps a flat `actions` list plus a
/// `next_redo` cursor. Each pane has a checkpoint marking its "saved"
/// position; crossing it emits `checkpointed`, which callers use to update
/// the buffer's modified state.
pub struct UndoSequence {
    actions: RefCell<Vec<Box<dyn UndoAction>>>,
    next_redo: RefCell<usize>,
    checkpoints: RefCell<HashMap<usize, Checkpoint>>,
    group_stack: RefCell<Vec<Vec<Box<dyn UndoAction>>>>,
    group_level: RefCell<usize>,
    busy: RefCell<bool>,
    checkpointed_callbacks: RefCell<Vec<CheckpointedCallback>>,
    can_undo_callbacks: RefCell<Vec<CanUndoCallback>>,
    can_redo_callbacks: RefCell<Vec<CanRedoCallback>>,
}

impl std::fmt::Debug for UndoSequence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UndoSequence")
            .field("actions", &self.actions.borrow().len())
            .field("next_redo", &*self.next_redo.borrow())
            .finish()
    }
}

impl UndoSequence {
    /// Create a sequence tracking checkpoints for the given pane indices.
    pub fn new(keys: &[usize]) -> Self {
        let checkpoints = keys.iter().map(|&k| (k, (Some(0), None))).collect();
        Self {
            actions: RefCell::new(Vec::new()),
            next_redo: RefCell::new(0),
            checkpoints: RefCell::new(checkpoints),
            group_stack: RefCell::new(Vec::new()),
            group_level: RefCell::new(0),
            busy: RefCell::new(false),
            checkpointed_callbacks: RefCell::new(Vec::new()),
            can_undo_callbacks: RefCell::new(Vec::new()),
            can_redo_callbacks: RefCell::new(Vec::new()),
        }
    }

    /// Register a callback invoked whenever a pane's checkpointed state changes.
    pub fn connect_checkpointed<F: Fn(usize, bool) + 'static>(&self, f: F) {
        self.checkpointed_callbacks.borrow_mut().push(Box::new(f));
    }

    /// Register a callback invoked whenever undo availability changes.
    pub fn connect_can_undo<F: Fn(bool) + 'static>(&self, f: F) {
        self.can_undo_callbacks.borrow_mut().push(Box::new(f));
    }

    /// Register a callback invoked whenever redo availability changes.
    pub fn connect_can_redo<F: Fn(bool) + 'static>(&self, f: F) {
        self.can_redo_callbacks.borrow_mut().push(Box::new(f));
    }

    /// Returns `true` if there is an action to undo.
    pub fn can_undo(&self) -> bool {
        *self.next_redo.borrow() > 0
    }

    /// Returns `true` if there is an action to redo.
    pub fn can_redo(&self) -> bool {
        *self.next_redo.borrow() < self.actions.borrow().len()
    }

    /// Returns `true` if `key` is currently at (or after) its checkpoint.
    pub fn checkpointed(&self, key: usize) -> bool {
        let checkpoints = self.checkpoints.borrow();
        let Some(&(start, end)) = checkpoints.get(&key) else {
            return false;
        };
        let Some(start) = start else {
            return false;
        };
        let end = end.unwrap_or(self.actions.borrow().len());
        let next_redo = *self.next_redo.borrow();
        start <= next_redo && next_redo <= end
    }

    /// Mark the current position as the checkpoint (saved state) for `key`.
    pub fn checkpoint(&self, key: usize) {
        let (start, end) = {
            let actions = self.actions.borrow();
            let next_redo = *self.next_redo.borrow();

            let mut start = next_redo;
            while start > 0 && actions[start - 1].key() != key {
                start -= 1;
            }

            let mut end = next_redo;
            while end < actions.len().saturating_sub(1) && actions[end + 1].key() != key {
                end += 1;
            }
            let end = if end == actions.len() {
                None
            } else {
                Some(end)
            };
            (start, end)
        };

        self.checkpoints
            .borrow_mut()
            .insert(key, (Some(start), end));
        self.notify_checkpointed(key, true);
    }

    /// Record a reversible action (or add it to the active group).
    pub fn add_action(&self, action: Box<dyn UndoAction>) {
        if *self.busy.borrow() {
            return;
        }

        if *self.group_level.borrow() > 0 {
            self.group_stack
                .borrow_mut()
                .last_mut()
                .expect("group stack underflow")
                .push(action);
            return;
        }

        let key = action.key();

        if self.checkpointed(key) {
            if let Some(cp) = self.checkpoints.borrow_mut().get_mut(&key) {
                cp.1 = Some(*self.next_redo.borrow());
            }
            self.notify_checkpointed(key, false);
        } else {
            let mut checkpoints = self.checkpoints.borrow_mut();
            if let Some((Some(start), _)) = checkpoints.get(&key) {
                if *start > *self.next_redo.borrow() {
                    checkpoints.insert(key, (None, None));
                }
            }
        }

        let could_undo = self.can_undo();
        let could_redo = self.can_redo();

        {
            let mut actions = self.actions.borrow_mut();
            actions.truncate(*self.next_redo.borrow());
            actions.push(action);
        }
        *self.next_redo.borrow_mut() += 1;

        if !could_undo {
            self.notify_can_undo(true);
        }
        if could_redo {
            self.notify_can_redo(false);
        }
    }

    /// Record a text insertion so it can be undone.
    pub fn record_insert(&self, key: usize, buffer: &gsv::Buffer, offset: usize, text: &str) {
        self.add_action(Box::new(BufferInsertionAction::new(
            key, buffer, offset, text,
        )));
    }

    /// Record a text deletion so it can be undone.
    pub fn record_delete(&self, key: usize, buffer: &gsv::Buffer, offset: usize, text: &str) {
        self.add_action(Box::new(BufferDeletionAction::new(
            key, buffer, offset, text,
        )));
    }

    /// Undo the action at the current cursor position.
    pub fn undo(&self) -> Result<(), UndoError> {
        if !self.can_undo() {
            return Err(UndoError::NothingToUndo);
        }
        *self.busy.borrow_mut() = true;

        let key = {
            let actions = self.actions.borrow();
            actions[*self.next_redo.borrow() - 1].key()
        };
        if self.checkpointed(key) {
            self.notify_checkpointed(key, false);
        }

        let could_redo = self.can_redo();
        *self.next_redo.borrow_mut() -= 1;

        {
            let actions = self.actions.borrow();
            actions[*self.next_redo.borrow()].undo();
        }

        *self.busy.borrow_mut() = false;

        if !self.can_undo() {
            self.notify_can_undo(false);
        }
        if !could_redo {
            self.notify_can_redo(true);
        }
        if self.checkpointed(key) {
            self.notify_checkpointed(key, true);
        }

        Ok(())
    }

    /// Redo the action just after the current cursor position.
    pub fn redo(&self) -> Result<(), UndoError> {
        if !self.can_redo() {
            return Err(UndoError::NothingToRedo);
        }
        *self.busy.borrow_mut() = true;

        let key = {
            let actions = self.actions.borrow();
            actions[*self.next_redo.borrow()].key()
        };
        if self.checkpointed(key) {
            self.notify_checkpointed(key, false);
        }

        let could_undo = self.can_undo();
        *self.next_redo.borrow_mut() += 1;

        {
            let actions = self.actions.borrow();
            actions[*self.next_redo.borrow() - 1].redo();
        }

        *self.busy.borrow_mut() = false;

        if !could_undo {
            self.notify_can_undo(true);
        }
        if !self.can_redo() {
            self.notify_can_redo(false);
        }
        if self.checkpointed(key) {
            self.notify_checkpointed(key, true);
        }

        Ok(())
    }

    /// Begin grouping subsequent actions. Nested groups are supported.
    pub fn begin_group(&self) {
        if *self.busy.borrow() {
            return;
        }
        *self.group_level.borrow_mut() += 1;
        self.group_stack.borrow_mut().push(Vec::new());
    }

    /// End the current group, collapsing it into a single logical action.
    pub fn end_group(&self) {
        if *self.busy.borrow() {
            return;
        }
        if *self.group_level.borrow() == 0 {
            log::warn!("Tried to end a non-existent undo group");
            return;
        }
        *self.group_level.borrow_mut() -= 1;

        let group = self.group_stack.borrow_mut().pop().unwrap_or_default();
        match group.len() {
            0 => {}
            1 => {
                let action = group.into_iter().next().expect("length checked");
                self.add_action(action);
            }
            _ => {
                self.add_action(Box::new(GroupAction::new(group)));
            }
        }
    }

    /// Discard the currently grouped actions without undoing them.
    pub fn abort_group(&self) {
        if *self.busy.borrow() {
            return;
        }
        if *self.group_level.borrow() == 0 {
            log::warn!("Tried to abort a non-existent undo group");
            return;
        }
        *self.group_level.borrow_mut() -= 1;
        self.group_stack.borrow_mut().pop();
    }

    /// Returns `true` while actions are being grouped.
    pub fn in_grouped_action(&self) -> bool {
        *self.group_level.borrow() > 0
    }

    /// Clear all undo/redo history and reset checkpoints.
    pub fn clear(&self) {
        let could_undo = self.can_undo();
        let could_redo = self.can_redo();

        self.actions.borrow_mut().clear();
        *self.next_redo.borrow_mut() = 0;
        for cp in self.checkpoints.borrow_mut().values_mut() {
            *cp = (Some(0), None);
        }
        self.group_stack.borrow_mut().clear();
        *self.group_level.borrow_mut() = 0;
        *self.busy.borrow_mut() = false;

        if could_undo {
            self.notify_can_undo(false);
        }
        if could_redo {
            self.notify_can_redo(false);
        }
    }

    fn notify_checkpointed(&self, key: usize, checkpointed: bool) {
        for cb in self.checkpointed_callbacks.borrow().iter() {
            cb(key, checkpointed);
        }
    }

    fn notify_can_undo(&self, can_undo: bool) {
        for cb in self.can_undo_callbacks.borrow().iter() {
            cb(can_undo);
        }
    }

    fn notify_can_redo(&self, can_redo: bool) {
        for cb in self.can_redo_callbacks.borrow().iter() {
            cb(can_redo);
        }
    }
}

impl Default for UndoSequence {
    fn default() -> Self {
        Self::new(&[])
    }
}

/// Deletes a range of text from `buffer`, using character offsets.
fn delete_range(buffer: &gsv::Buffer, offset: usize, text: &str) {
    let mut start = buffer.iter_at_offset(offset as i32);
    let mut end = buffer.iter_at_offset((offset + text.chars().count()) as i32);
    buffer.delete(&mut start, &mut end);
    buffer.place_cursor(&end);
}

/// Inserts `text` into `buffer` at the given character offset.
fn insert_range(buffer: &gsv::Buffer, offset: usize, text: &str) {
    let mut start = buffer.iter_at_offset(offset as i32);
    buffer.place_cursor(&start);
    buffer.insert(&mut start, text);
}

/// Undo/redo state for a single text insertion.
struct BufferInsertionAction {
    key: usize,
    buffer: gsv::Buffer,
    offset: usize,
    text: String,
}

impl BufferInsertionAction {
    fn new(key: usize, buffer: &gsv::Buffer, offset: usize, text: &str) -> Self {
        Self {
            key,
            buffer: buffer.clone(),
            offset,
            text: text.to_owned(),
        }
    }
}

impl UndoAction for BufferInsertionAction {
    fn key(&self) -> usize {
        self.key
    }

    fn undo(&self) {
        delete_range(&self.buffer, self.offset, &self.text);
    }

    fn redo(&self) {
        insert_range(&self.buffer, self.offset, &self.text);
    }
}

/// Undo/redo state for a single text deletion.
struct BufferDeletionAction {
    key: usize,
    buffer: gsv::Buffer,
    offset: usize,
    text: String,
}

impl BufferDeletionAction {
    fn new(key: usize, buffer: &gsv::Buffer, offset: usize, text: &str) -> Self {
        Self {
            key,
            buffer: buffer.clone(),
            offset,
            text: text.to_owned(),
        }
    }
}

impl UndoAction for BufferDeletionAction {
    fn key(&self) -> usize {
        self.key
    }

    fn undo(&self) {
        insert_range(&self.buffer, self.offset, &self.text);
    }

    fn redo(&self) {
        delete_range(&self.buffer, self.offset, &self.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy)]
    struct DummyAction {
        key: usize,
    }

    impl UndoAction for DummyAction {
        fn key(&self) -> usize {
            self.key
        }

        fn undo(&self) {}

        fn redo(&self) {}
    }

    #[test]
    fn test_undo_redo_cursor() {
        let seq = UndoSequence::new(&[0]);
        seq.add_action(Box::new(DummyAction { key: 0 }));
        assert!(seq.can_undo());
        assert!(!seq.can_redo());

        seq.undo().unwrap();
        assert!(!seq.can_undo());
        assert!(seq.can_redo());

        seq.redo().unwrap();
        assert!(seq.can_undo());
        assert!(!seq.can_redo());
    }

    #[test]
    fn test_checkpoint_marks_modified() {
        let seq = UndoSequence::new(&[0]);
        assert!(seq.checkpointed(0));

        seq.add_action(Box::new(DummyAction { key: 0 }));
        assert!(!seq.checkpointed(0));

        seq.checkpoint(0);
        assert!(seq.checkpointed(0));
    }

    #[test]
    fn test_undo_back_to_checkpoint() {
        let seq = UndoSequence::new(&[0]);
        seq.add_action(Box::new(DummyAction { key: 0 }));
        seq.checkpoint(0);
        seq.add_action(Box::new(DummyAction { key: 0 }));

        assert!(!seq.checkpointed(0));
        seq.undo().unwrap();
        assert!(seq.checkpointed(0));
    }

    #[test]
    fn test_redo_truncates_after_new_action() {
        let seq = UndoSequence::new(&[0]);
        seq.add_action(Box::new(DummyAction { key: 0 }));
        seq.add_action(Box::new(DummyAction { key: 0 }));
        seq.undo().unwrap();

        assert!(seq.can_redo());

        seq.add_action(Box::new(DummyAction { key: 0 }));
        assert!(!seq.can_redo());
        assert!(seq.can_undo());
    }

    #[test]
    fn test_group_collapses_to_single_action() {
        let seq = UndoSequence::new(&[0]);
        seq.begin_group();
        seq.add_action(Box::new(DummyAction { key: 0 }));
        seq.add_action(Box::new(DummyAction { key: 0 }));
        seq.end_group();

        assert!(seq.can_undo());
        seq.undo().unwrap();
        assert!(!seq.can_undo());
        assert!(seq.can_redo());
    }

    #[test]
    fn test_clear_resets_checkpoint() {
        let seq = UndoSequence::new(&[0]);
        seq.add_action(Box::new(DummyAction { key: 0 }));
        assert!(!seq.checkpointed(0));

        seq.clear();
        assert!(!seq.can_undo());
        assert!(!seq.can_redo());
        assert!(seq.checkpointed(0));
    }
}
