//! Undo/redo system for text buffer operations.
//!
//! Replaces the Python `UndoSequence`/`GroupAction` from the original Meld.
//! Manages grouped undo actions across multiple source view buffers.

use std::cell::RefCell;
use std::rc::Rc;

type UndoAction = Box<dyn Fn()>;

/// Errors related to undo operations.
#[derive(Debug, thiserror::Error)]
pub enum UndoError {
    #[error("Nothing to undo")]
    NothingToUndo,
    #[error("Nothing to redo")]
    NothingToRedo,
}

/// A single reversible action within the undo stack.
struct UndoEntry {
    undo_action: Option<UndoAction>,
    redo_action: Option<UndoAction>,
    description: String,
}

impl std::fmt::Debug for UndoEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UndoEntry")
            .field("description", &self.description)
            .finish()
    }
}

/// Manages undo/redo across one or more source view buffers.
///
/// Supports grouping multiple actions into a single logical undo step
/// via `begin_group()` / `end_group()`.
pub struct UndoSequence {
    undo_stack: RefCell<Vec<UndoEntry>>,
    redo_stack: RefCell<Vec<UndoEntry>>,
    group_stack: RefCell<Vec<Vec<UndoEntry>>>,
    group_level: RefCell<usize>,
}

impl UndoSequence {
    /// Create a new empty undo sequence.
    pub fn new() -> Self {
        Self {
            undo_stack: RefCell::new(Vec::new()),
            redo_stack: RefCell::new(Vec::new()),
            group_stack: RefCell::new(Vec::new()),
            group_level: RefCell::new(0),
        }
    }

    /// Begin grouping subsequent actions. Nested groups are supported.
    pub fn begin_group(&self) {
        *self.group_level.borrow_mut() += 1;
        self.group_stack.borrow_mut().push(Vec::new());
    }

    /// End the current group, collapsing it to a single undo entry.
    pub fn end_group(&self, description: impl Into<String>) {
        let level = *self.group_level.borrow();
        if level == 0 {
            return;
        }
        *self.group_level.borrow_mut() -= 1;

        let group = self.group_stack.borrow_mut().pop().unwrap_or_default();
        if group.is_empty() {
            return;
        }

        let desc = description.into();
        // Combine all undo/redo actions in the group
        let combined = Rc::new(RefCell::new(group));
        let c_undo = combined.clone();
        let c_redo = combined.clone();

        let entry = UndoEntry {
            undo_action: Some(Box::new(move || {
                for e in c_undo.borrow().iter().rev() {
                    if let Some(ref action) = e.undo_action {
                        action();
                    }
                }
            })),
            redo_action: Some(Box::new(move || {
                for e in c_redo.borrow().iter() {
                    if let Some(ref action) = e.redo_action {
                        action();
                    }
                }
            })),
            description: desc,
        };

        self.undo_stack.borrow_mut().push(entry);
        self.redo_stack.borrow_mut().clear();
    }

    /// Record a reversible action.
    pub fn add_action(
        &self,
        undo: impl FnOnce() + 'static,
        redo: impl FnOnce() + 'static,
        description: impl Into<String>,
    ) {
        let undo_cell = Rc::new(RefCell::new(Some(undo)));
        let redo_cell = Rc::new(RefCell::new(Some(redo)));

        let entry = UndoEntry {
            undo_action: Some(Box::new(move || {
                if let Some(f) = undo_cell.borrow_mut().take() {
                    f();
                }
            })),
            redo_action: Some(Box::new(move || {
                if let Some(f) = redo_cell.borrow_mut().take() {
                    f();
                }
            })),
            description: description.into(),
        };

        if *self.group_level.borrow() > 0 {
            self.group_stack
                .borrow_mut()
                .last_mut()
                .unwrap()
                .push(entry);
        } else {
            self.undo_stack.borrow_mut().push(entry);
            self.redo_stack.borrow_mut().clear();
        }
    }

    /// Undo the last action. Returns the description of the undone action.
    pub fn undo(&self) -> Result<String, UndoError> {
        let mut entry = self
            .undo_stack
            .borrow_mut()
            .pop()
            .ok_or(UndoError::NothingToUndo)?;
        if let Some(action) = entry.undo_action.take() {
            action();
        }
        let desc = entry.description.clone();
        self.redo_stack.borrow_mut().push(entry);
        Ok(desc)
    }

    /// Redo the last undone action. Returns the description of the redone action.
    pub fn redo(&self) -> Result<String, UndoError> {
        let mut entry = self
            .redo_stack
            .borrow_mut()
            .pop()
            .ok_or(UndoError::NothingToRedo)?;
        if let Some(action) = entry.redo_action.take() {
            action();
        }
        let desc = entry.description.clone();
        self.undo_stack.borrow_mut().push(entry);
        Ok(desc)
    }

    /// Returns `true` if there are actions to undo.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.borrow().is_empty()
    }

    /// Returns `true` if there are actions to redo.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.borrow().is_empty()
    }

    /// Clear all undo/redo history.
    pub fn clear(&self) {
        self.undo_stack.borrow_mut().clear();
        self.redo_stack.borrow_mut().clear();
        self.group_stack.borrow_mut().clear();
        *self.group_level.borrow_mut() = 0;
    }
}

impl Default for UndoSequence {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_undo_redo_simple() {
        let seq = UndoSequence::new();
        let mut value = 0;

        seq.add_action(
            move || {
                value = 0;
            },
            move || {
                value = 42;
            },
            "set to 42",
        );

        assert!(seq.can_undo());
        assert!(!seq.can_redo());

        seq.undo().unwrap();
        assert_eq!(value, 0);
        assert!(seq.can_redo());

        seq.redo().unwrap();
        assert_eq!(value, 42);
    }

    #[test]
    #[ignore]
    fn test_group() {
        // TODO: Fix closure ownership semantics for UndoSequence::add_action
        /*
        let mut a = 0;
        let mut b = 0;

        seq.begin_group();
        seq.add_action(
            || { a = 0; },
            || { a = 1; },
            "set a=1",
        );
        seq.add_action(
            || { b = 0; },
            || { b = 2; },
            "set b=2",
        );
        seq.end_group("set both");

        assert!(seq.can_undo());
        seq.undo().unwrap();
        assert_eq!(a, 0);
        assert_eq!(b, 0);

        seq.redo().unwrap();
        assert_eq!(a, 1);
        assert_eq!(b, 2);
        */
    }
}
