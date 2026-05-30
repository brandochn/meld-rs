#![cfg(feature = "gui")]
use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use crate::diff::engine::{Chunk, DiffOp, Differ};
use crate::diff::movement::MoveMap;
use crate::diff::similarity::SimilarityMap;

pub struct DiffResult {
    pub chunks: Vec<Chunk>,
    pub similarity: SimilarityMap,
    pub movement: MoveMap,
    pub text_a: Vec<String>,
    pub text_b: Vec<String>,
    pub is_empty: bool,
    pub is_identical: bool,
}

pub struct DiffState {
    cancel_flag: Option<std::sync::Arc<AtomicBool>>,
    poll_source: Option<glib::SourceId>,
    debounce_source: Option<glib::SourceId>,
    generation: Rc<Cell<u64>>,
}

impl DiffState {
    pub fn new() -> Self {
        Self {
            cancel_flag: None,
            poll_source: None,
            debounce_source: None,
            generation: Rc::new(Cell::new(0)),
        }
    }

    pub fn cancel_all(&mut self) {
        if let Some(flag) = &self.cancel_flag {
            flag.store(true, Ordering::SeqCst);
        }
        self.cancel_flag = None;
        if let Some(src) = self.debounce_source.take() {
            src.remove();
        }
        if let Some(src) = self.poll_source.take() {
            src.remove();
        }
    }

    pub fn schedule_diff(
        &mut self,
        text_a: Vec<String>,
        text_b: Vec<String>,
        on_complete: Box<dyn FnOnce(DiffResult) + 'static>,
    ) {
        self.cancel_all();

        if text_a.is_empty() && text_b.is_empty() {
            return;
        }

        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let gen = self.generation.get() + 1;
        self.generation.set(gen);

        self.cancel_flag = Some(std::sync::Arc::clone(&cancel));

        let (tx, rx) = mpsc::channel::<Option<DiffResult>>();
        let debounce_cancel = std::sync::Arc::clone(&cancel);

        let mut text_a_opt = Some(text_a);
        let mut text_b_opt = Some(text_b);

        let debounce_id = glib::timeout_add_local(Duration::from_millis(150), move || {
            if debounce_cancel.load(Ordering::SeqCst) {
                return glib::ControlFlow::Break;
            }

            let text_a = text_a_opt.take().unwrap_or_default();
            let text_b = text_b_opt.take().unwrap_or_default();

            if text_a.is_empty() && text_b.is_empty() {
                return glib::ControlFlow::Break;
            }

            let cancel_clone = std::sync::Arc::clone(&debounce_cancel);
            let tx_clone = tx.clone();

            std::thread::spawn(move || {
                if cancel_clone.load(Ordering::SeqCst) {
                    let _ = tx_clone.send(None);
                    return;
                }

                let differ = Differ::new(text_a.clone(), text_b.clone());
                let result = match differ.compare_with_cancel(&cancel_clone) {
                    Some(r) => r,
                    None => {
                        let _ = tx_clone.send(None);
                        return;
                    }
                };

                let merged =
                    crate::diff::engine::merge_adjacent_replace_chunks(&result.chunks);

                if cancel_clone.load(Ordering::SeqCst) {
                    let _ = tx_clone.send(None);
                    return;
                }

                let mut matched_left = std::collections::HashSet::new();
                let mut matched_right = std::collections::HashSet::new();
                for chunk in &merged {
                    if chunk.op == DiffOp::Equal {
                        for i in chunk.start_a..chunk.end_a {
                            matched_left.insert(i);
                        }
                        for i in chunk.start_b..chunk.end_b {
                            matched_right.insert(i);
                        }
                    }
                }

                if cancel_clone.load(Ordering::SeqCst) {
                    let _ = tx_clone.send(None);
                    return;
                }

                let similarity = SimilarityMap::build(
                    &text_a,
                    &text_b,
                    &matched_left,
                    &matched_right,
                    0.25,
                    50,
                    &cancel_clone,
                );

                if cancel_clone.load(Ordering::SeqCst) {
                    let _ = tx_clone.send(None);
                    return;
                }

                let movement = MoveMap::build(
                    &text_a,
                    &text_b,
                    &matched_left,
                    &matched_right,
                    0.6,
                    2,
                    &cancel_clone,
                );

                if cancel_clone.load(Ordering::SeqCst) {
                    let _ = tx_clone.send(None);
                    return;
                }

                let is_empty = text_a.is_empty() && text_b.is_empty();
                let is_identical = merged.iter().all(|c| c.op == DiffOp::Equal);

                let _ = tx_clone.send(Some(DiffResult {
                    chunks: merged,
                    similarity,
                    movement,
                    text_a,
                    text_b,
                    is_empty,
                    is_identical,
                }));
            });

            glib::ControlFlow::Break
        });
        self.debounce_source = Some(debounce_id);

        let gen_cell = Rc::clone(&self.generation);
        let mut on_complete_opt = Some(on_complete);

        let poll_id = glib::timeout_add_local(Duration::from_millis(16), move || {
            match rx.try_recv() {
                Ok(Some(diff_result)) => {
                    if gen_cell.get() != gen {
                        return glib::ControlFlow::Break;
                    }
                    if let Some(cb) = on_complete_opt.take() {
                        cb(diff_result);
                    }
                    glib::ControlFlow::Break
                }
                Ok(None) => glib::ControlFlow::Break,
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            }
        });
        self.poll_source = Some(poll_id);
    }
}
