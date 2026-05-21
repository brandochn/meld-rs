#![cfg(feature = "gui")]
//! Background task scheduling.
//!
//! Provides a lightweight task queue for running background operations
//! (e.g., diff computation, file scanning) without blocking the UI thread.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// A task that can be run by the scheduler.
pub type Task = Box<dyn FnOnce() + Send + 'static>;

/// A simple LIFO (last-in, first-out) task scheduler.
///
/// Tasks are accumulated and executed one at a time from the GTK main
/// loop via `glib::idle_add`.
pub struct LifoScheduler {
    tasks: Arc<Mutex<VecDeque<Task>>>,
    pending: Arc<Mutex<bool>>,
}

impl LifoScheduler {
    /// Create a new empty scheduler.
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(Mutex::new(VecDeque::new())),
            pending: Arc::new(Mutex::new(false)),
        }
    }

    /// Add a task to the scheduler.
    pub fn add_task<F>(&self, task: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.tasks.lock().unwrap().push_back(Box::new(task));

        let mut pending = self.pending.lock().unwrap();
        if !*pending {
            *pending = true;
            self.schedule_next();
        }
    }

    /// Returns `true` if there are still tasks waiting to run.
    pub fn tasks_pending(&self) -> bool {
        !self.tasks.lock().unwrap().is_empty()
    }

    /// Run the next available task.
    ///
    /// Returns `true` if there are more tasks to process, `false` otherwise.
    pub fn run_next(&self) -> bool {
        let task = self.tasks.lock().unwrap().pop_front();

        if let Some(task) = task {
            task();
            let more = !self.tasks.lock().unwrap().is_empty();
            if more {
                self.schedule_next();
            } else {
                *self.pending.lock().unwrap() = false;
            }
            more
        } else {
            *self.pending.lock().unwrap() = false;
            false
        }
    }

    fn schedule_next(&self) {
        let tasks = Arc::clone(&self.tasks);
        let pending = Arc::clone(&self.pending);
        glib::idle_add_local(move || {
            let task = tasks.lock().unwrap().pop_front();
            if let Some(task) = task {
                task();
                let more = !tasks.lock().unwrap().is_empty();
                if !more {
                    *pending.lock().unwrap() = false;
                }
                glib::ControlFlow::from(more)
            } else {
                *pending.lock().unwrap() = false;
                glib::ControlFlow::Break
            }
        });
    }
}

impl Default for LifoScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_scheduler() {
        let sched = LifoScheduler::new();
        assert!(!sched.tasks_pending());
    }

    #[test]
    fn test_add_task() {
        let sched = LifoScheduler::new();
        assert!(!sched.tasks_pending());
        // Tasks run on idle, so they just queue
        sched.add_task(|| {
            let _x = 42;
        });
    }
}
