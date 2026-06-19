#![cfg(feature = "gui")]
//! Path label widget.
//!
//! Ported from the original `meld/ui/pathlabel.py`.  A flat `MenuButton`
//! showing `[icon] shortened-path`; clicking it opens a popover with the full
//! path and "Copy Path" / "Open Containing Folder" actions.

use gtk4 as gtk;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

/// A label that displays a file path, shortened intelligently (home-relative)
/// and ellipsized, with a popover exposing the full path and actions.
pub struct PathLabel {
    button: gtk::MenuButton,
    icon: gtk::Image,
    label: gtk::Label,
    entry: gtk::Entry,
    full_path: Rc<RefCell<String>>,
    home_dir: Option<String>,
}

impl PathLabel {
    /// Create a new path label.
    pub fn new() -> Self {
        let button = gtk::MenuButton::new();
        button.add_css_class("flat");
        button.set_focus_on_click(false);

        // ── Visible content: [icon] shortened-path ──
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let icon = gtk::Image::from_icon_name("text-x-generic-symbolic");
        let label = gtk::Label::new(None);
        label.set_ellipsize(pango::EllipsizeMode::Start);
        label.set_xalign(0.0);
        label.set_max_width_chars(40);
        content.append(&icon);
        content.append(&label);
        button.set_child(Some(&content));

        // ── Popover: full path + actions ──
        let popover = gtk::Popover::new();
        let pbox = gtk::Box::new(gtk::Orientation::Vertical, 6);
        pbox.set_margin_start(6);
        pbox.set_margin_end(6);
        pbox.set_margin_top(6);
        pbox.set_margin_bottom(6);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let path_caption = gtk::Label::new(Some("Path"));
        path_caption.add_css_class("dim-label");
        let entry = gtk::Entry::new();
        entry.set_editable(false);
        entry.set_width_chars(60);
        entry.add_css_class("flat");
        row.append(&path_caption);
        row.append(&entry);
        pbox.append(&row);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        actions.set_halign(gtk::Align::End);
        actions.add_css_class("linked");
        let copy_btn = gtk::Button::with_label("Copy Path");
        copy_btn.set_tooltip_text(Some("Copy the full path"));
        copy_btn.set_focus_on_click(false);
        let open_btn = gtk::Button::with_label("Open Containing Folder");
        open_btn.set_tooltip_text(Some("View the folder in the file manager"));
        open_btn.set_focus_on_click(false);
        actions.append(&copy_btn);
        actions.append(&open_btn);
        pbox.append(&actions);

        popover.set_child(Some(&pbox));
        button.set_popover(Some(&popover));

        let full_path = Rc::new(RefCell::new(String::new()));

        // Copy Path → clipboard
        {
            let full_path = Rc::clone(&full_path);
            copy_btn.connect_clicked(move |b| {
                let text = full_path.borrow().clone();
                if !text.is_empty() {
                    b.clipboard().set_text(&text);
                }
            });
        }

        // Open Containing Folder → file manager
        {
            let full_path = Rc::clone(&full_path);
            open_btn.connect_clicked(move |_| {
                let path = full_path.borrow().clone();
                if path.is_empty() {
                    return;
                }
                let file = gio::File::for_path(&path);
                let launcher = gtk::FileLauncher::new(Some(&file));
                launcher.open_containing_folder(
                    None::<&gtk::Window>,
                    gio::Cancellable::NONE,
                    |_| {},
                );
            });
        }

        let home = dirs::home_dir().map(|p| p.to_string_lossy().into_owned());

        Self {
            button,
            icon,
            label,
            entry,
            full_path,
            home_dir: home,
        }
    }

    /// Underlying widget.
    pub fn widget(&self) -> &gtk::Widget {
        self.button.upcast_ref()
    }

    /// Set the displayed path.
    pub fn set_path(&self, path: &str) {
        self.full_path.replace(path.to_owned());
        self.entry.set_text(path);
        self.label.set_text(&self.shorten(path));
        self.button.set_tooltip_text(Some(path));
    }

    /// Override the leading icon (e.g. a folder icon for directory panes).
    pub fn set_icon_name(&self, name: &str) {
        self.icon.set_icon_name(Some(name));
    }

    /// Get the full (unshortened) path.
    pub fn full_path(&self) -> String {
        self.full_path.borrow().clone()
    }

    fn shorten(&self, path: &str) -> String {
        if let Some(ref home) = self.home_dir {
            if let Ok(relative) = Path::new(path).strip_prefix(home) {
                return format!("~/{}", relative.to_string_lossy());
            }
        }
        path.to_owned()
    }
}

impl Default for PathLabel {
    fn default() -> Self {
        Self::new()
    }
}
