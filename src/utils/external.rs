//! Launch files in the user's editor.
//!
//! Mirrors the original Meld `externalhelpers.make_custom_editor_command`:
//! a configured custom editor command is used (with `{file}` / `{line}`
//! placeholders), otherwise the operating system's default handler opens
//! the file.

use crate::config::settings::MeldSettings;

/// Open `path` with the configured external editor, or the OS default
/// handler when no custom editor is set.
///
/// `line` is the 1-based cursor line, substituted for `{line}` when the
/// custom command uses it.
pub fn open_with_editor(path: &str, line: Option<i32>) {
    let settings = MeldSettings::load().unwrap_or_default();
    if !settings.use_system_editor && !settings.custom_editor_command.trim().is_empty() {
        let argv =
            make_custom_editor_command(&settings.custom_editor_command, path, line.unwrap_or(0));
        if let Err(e) = spawn(&argv) {
            log::error!("Failed to launch custom editor {:?}: {}", argv, e);
        }
        return;
    }

    // System default handler.
    let result = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", path])
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(path).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(path).spawn()
    };
    if let Err(e) = result {
        log::error!("Failed to open '{}': {}", path, e);
    }
}

/// Open each path using the appropriate handler: directories with the system
/// file manager, regular files with the configured editor or system default.
/// Mirrors the original Meld `open_files_external`.
pub fn open_files_external(paths: &[String]) {
    for path in paths {
        if std::path::Path::new(path).is_dir() {
            open_directory(path);
        } else {
            open_with_editor(path, None);
        }
    }
}

/// Open a directory in the system file manager.
pub fn open_directory(path: &str) {
    let result = if cfg!(target_os = "windows") {
        std::process::Command::new("explorer").arg(path).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(path).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(path).spawn()
    };
    if let Err(e) = result {
        log::error!("Failed to open directory '{}': {}", path, e);
    }
}

/// Expand `{file}` / `{line}` in the custom editor command and split it
/// into argv, mirroring Meld's `make_custom_editor_command`.
fn make_custom_editor_command(command: &str, path: &str, line: i32) -> Vec<String> {
    if command.contains("{file}") || command.contains("{line}") {
        let substituted = command
            .replace("{file}", &quote(path))
            .replace("{line}", &line.to_string());
        split_command(&substituted)
    } else {
        // Legacy behaviour: the whole string is the executable and the path
        // is appended as a single argument.
        vec![command.to_string(), path.to_string()]
    }
}

/// Minimal double-quote wrapper (equivalent in practice to `shlex.quote`
/// for the common editor-command cases).
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\\\""))
}

/// Split a command line on whitespace, honouring double quotes.
fn split_command(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => in_quotes = !in_quotes,
            '\\' if in_quotes => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            c if c.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

fn spawn(argv: &[String]) -> std::io::Result<()> {
    let Some(program) = argv.first() else {
        return Err(std::io::Error::other("empty editor command"));
    };
    std::process::Command::new(program)
        .args(&argv[1..])
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_command_simple() {
        assert_eq!(split_command("gedit"), vec!["gedit"]);
        assert_eq!(
            split_command("code -g --reuse-window"),
            vec!["code", "-g", "--reuse-window"]
        );
    }

    #[test]
    fn test_split_command_quotes() {
        assert_eq!(
            split_command("\"C:/Program Files/My Editor/editor.exe\" file.txt"),
            vec!["C:/Program Files/My Editor/editor.exe", "file.txt"]
        );
    }

    #[test]
    fn test_make_command_with_placeholders() {
        let argv = make_custom_editor_command("code -g {file}:{line}", "/a b.txt", 42);
        assert_eq!(argv, vec!["code", "-g", "/a b.txt:42"]);
    }

    #[test]
    fn test_make_command_legacy() {
        let argv = make_custom_editor_command("gedit", "/a.txt", 7);
        assert_eq!(argv, vec!["gedit", "/a.txt"]);
    }
}
