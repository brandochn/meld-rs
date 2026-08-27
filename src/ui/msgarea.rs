#![cfg(feature = "gui")]
//! Message area widget built on `AdwBanner` (the modern replacement for
//! `GtkInfoBar`), matching the original Meld's banner: an icon, coloured
//! background, prominent title, and an optional action/dismiss button.

use gtk4 as gtk;
use gtk4::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;

/// Optional action callback shown as the banner's button.
type ActionCallback = Rc<RefCell<Option<Box<dyn Fn()>>>>;

pub struct MsgArea {
    banner: adw::Banner,
    action: ActionCallback,
}

impl MsgArea {
    pub fn new() -> Self {
        let banner = adw::Banner::new("");
        banner.set_revealed(false);
        banner.set_hexpand(true);

        let action: ActionCallback = Rc::new(RefCell::new(None));
        {
            let action_cb = Rc::clone(&action);
            let banner_cb = banner.clone();
            banner.connect_button_clicked(move |_| {
                let callback = action_cb.borrow_mut().take();
                if let Some(callback) = callback {
                    callback();
                }
                banner_cb.set_button_label(None);
                banner_cb.set_revealed(false);
            });
        }

        Self { banner, action }
    }

    pub fn widget(&self) -> &gtk::Widget {
        self.banner.upcast_ref()
    }

    pub fn show_info(&self, msg: &str) {
        self.show_msg(msg, None);
    }
    pub fn show_warning(&self, msg: &str) {
        self.show_msg(msg, None);
    }
    pub fn show_error(&self, msg: &str) {
        self.show_msg(msg, None);
    }

    /// Show an informational message with a "Hide" button, mirroring Meld's
    /// dismissable "Files are identical" message.
    pub fn show_info_dismissable(&self, msg: &str) {
        *self.action.borrow_mut() = None;
        self.show_msg(msg, Some("Hide"));
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
        self.show_msg(msg, Some(action_label));
    }

    pub fn hide(&self) {
        *self.action.borrow_mut() = None;
        self.banner.set_button_label(None);
        self.banner.set_revealed(false);
    }

    fn show_msg(&self, msg: &str, action_label: Option<&str>) {
        self.banner.set_title(msg);
        match action_label {
            Some(label) => self.banner.set_button_label(Some(label)),
            None => self.banner.set_button_label(None),
        }
        self.banner.set_revealed(true);
    }
}

impl Default for MsgArea {
    fn default() -> Self {
        Self::new()
    }
}
