#![cfg(feature = "gui")]
//! Message area widget using a simple label with CSS styling.
//! GTK4 `InfoBar` API changed; this provides a simplified version.

use gtk4 as gtk;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgType {
    Info,
    Warning,
    Error,
}

/// Optional action callback shown as a button next to the message.
type ActionCallback = Rc<RefCell<Option<Box<dyn Fn()>>>>;

pub struct MsgArea {
    container: gtk::Box,
    label: gtk::Label,
    action_button: gtk::Button,
    action: ActionCallback,
}

impl MsgArea {
    pub fn new() -> Self {
        let container = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        container.set_visible(false);
        container.set_hexpand(true);
        container.set_margin_start(6);
        container.set_margin_end(6);
        container.set_margin_top(2);
        container.set_margin_bottom(2);
        container.set_css_classes(&["toolbar", "meld-msgarea"]);

        let label = gtk::Label::new(None);
        label.set_wrap(true);
        label.set_xalign(0.0);
        label.set_hexpand(true);
        container.append(&label);

        // Optional action button shown next to the message (mirrors the
        // original Meld's action messages, e.g. "Reload" on file changes).
        let action_button = gtk::Button::with_label("");
        action_button.set_visible(false);
        action_button.set_focus_on_click(false);
        container.append(&action_button);

        let action: ActionCallback = Rc::new(RefCell::new(None));
        {
            let action_cb = Rc::clone(&action);
            let container_cb = container.clone();
            let button_cb = action_button.clone();
            action_button.connect_clicked(move |_| {
                let callback = action_cb.borrow_mut().take();
                if let Some(callback) = callback {
                    callback();
                }
                button_cb.set_visible(false);
                container_cb.set_visible(false);
            });
        }

        Self {
            container,
            label,
            action_button,
            action,
        }
    }

    pub fn widget(&self) -> &gtk::Widget {
        self.container.upcast_ref()
    }

    pub fn show_info(&self, msg: &str) {
        self.show_msg(msg);
    }
    pub fn show_warning(&self, msg: &str) {
        self.show_msg(msg);
    }
    pub fn show_error(&self, msg: &str) {
        self.show_msg(msg);
    }

    /// Show a message with an action button (e.g. "Reload").  Clicking
    /// the button invokes `on_action` and hides the message.
    pub fn show_warning_action(
        &self,
        msg: &str,
        action_label: &str,
        on_action: impl Fn() + 'static,
    ) {
        *self.action.borrow_mut() = Some(Box::new(on_action));
        self.action_button.set_label(action_label);
        self.action_button.set_visible(true);
        self.show_msg(msg);
    }

    pub fn hide(&self) {
        *self.action.borrow_mut() = None;
        self.action_button.set_visible(false);
        self.container.set_visible(false);
    }

    fn show_msg(&self, msg: &str) {
        self.label.set_text(msg);
        // Plain messages have no associated action.
        self.action_button.set_visible(false);
        self.container.set_visible(true);
    }
}

impl Default for MsgArea {
    fn default() -> Self {
        Self::new()
    }
}
