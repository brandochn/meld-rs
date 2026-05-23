# Meld-rs Architecture

## Component Mapping (Original Python → Rust)

| Componente         | Archivo(s) original(es)                     | Archivo(s) Rust            | Propósito                                     |
|-------------------|---------------------------------------------|----------------------------|-----------------------------------------------|
| Entry point        | `bin/meld`, `meld/meldapp.py`               | `src/main.rs`, `src/app.rs`| Inicio de la aplicación, parseo CLI, GTK App   |
| Main Window        | `meld/meldwindow.py`                        | `src/window.rs`            | Ventana principal con pestañas (GtkNotebook)   |
| File Diff          | `meld/filediff.py`, `meld/diffgrid.py`       | `src/diff/filediff.rs`, `src/ui/diff_view.rs` | Comparación lado a lado de archivos |
| Directory Diff     | `meld/dirdiff.py`                           | `src/diff/dirdiff.rs`, `src/ui/dir_view.rs` | Comparación de directorios              |
| Version Control    | `meld/vcview.py`, `meld/vc/`                | `src/vc/`, `src/ui/vc_view.rs` | Integración con Git, SVN, Mercurial    |
| Merge View         | `meld/mergeview.py`                         | `src/ui/merge_view.rs`    | Vista de merge de 3 vías                      |
| Diff Engine        | `meld/matchers/diffutil.py`, `meld/matchers/`| `src/diff/engine.rs`, `src/diff/matchers.rs` | Algoritmos de diff y matching    |
| Settings           | `meld/settings.py`                          | `src/config/settings.rs`  | Preferencias y configuración (GSettings)       |
| Recent Files       | `meld/recent.py`                            | `src/config/recent.rs`    | Archivos recientes                             |
| Task Runner        | `meld/task.py`                              | `src/utils/task.rs`       | Tareas en background                           |
| Filters            | `meld/filters.py`                           | `src/utils/`              | Filtros para comparación de directorios        |
| Encoding           | `meld/iohelpers.py`, `meld/meldbuffer.py`   | `src/utils/encoding.rs`   | Manejo de encodings de archivos                |
| Resources          | `data/icons/`, `data/styles/`               | `resources/`              | Iconos, CSS, recursos GTK                     |
| Preferences Dialog | `meld/preferences.py`                       | `src/ui/preferences.rs`   | Diálogo de preferencias                       |
| Find Bar           | `meld/ui/findbar.py`                        | `src/ui/find_bar.rs`      | Barra de búsqueda en archivos                 |
| Diff Map           | `meld/chunkmap.py`                          | `src/ui/diff_map.rs`      | Mapa de diferencias lateral (overview)         |
| Notebook Label     | `meld/ui/notebooklabel.py`                  | `src/ui/tab_manager.rs`   | Gestor de etiquetas de pestañas               |

## Dependency Mapping

| Python Module            | Rust Crate Equivalent |
|-------------------------|----------------------|
| `gi.repository.Gtk`     | `gtk4`               |
| `gi.repository.Gio`     | `gio`                |
| `gi.repository.GLib`    | `glib`               |
| `gi.repository.Gdk`     | `gdk4`               |
| `gi.repository.GtkSource`| `sourceview5`       |
| `gi.repository.Pango`   | `pango`              |
| `gi.repository.GObject` | `glib::Object`       |
| Python `difflib`        | `similar`            |
| Python `re`             | `regex`              |
| Python `json`           | `serde_json`         |
| Python `os.path`        | `std::path`          |
| Python threading        | `tokio` / `glib::spawn`|

## Key Design Decisions

1. **GTK 4 over GTK 3**: Uses modern GTK 4 APIs with `libadwaita` for adaptive UI
2. **similar crate for diffing**: Replaces Python's `difflib` with Rust's `similar`
3. **GSettings for config**: Uses GIO's GSettings for persistent configuration, same as original
4. **std::process::Command for VCS**: Executes git/svn/hg commands as subprocesses
5. **tokio for async**: Background tasks use tokio runtime where possible, with glib::spawn for GTK integration
6. **thiserror for error types**: Idiomatic Rust error handling
7. **serde for serialization**: For saved comparison files (.meldcmp) and config
