# Meld-rs

**Meld-rs** is a complete rewrite of the [Meld](https://gitlab.gnome.org/GNOME/meld) visual diff
and merge tool in **Rust** using **gtk-rs** (GTK 4). It provides side-by-side file and directory
comparison, 3-way merge, and version control integration (Git, SVN, Mercurial).

> [!WARNING]
> This project is in an **early experimental phase** and under active development.
> Expect bugs, incomplete features, and breaking changes. Use with caution —
> do not rely on it for production or critical workflows yet.

## Features

- 2-way and 3-way file comparison with syntax highlighting
- Recursive directory comparison with filters
- 3-way merge with conflict resolution
- Version control integration (Git, SVN, Mercurial)
- Syntax highlighting via GtkSourceView
- Diff map sidebar for visual overview
- In-file search (find bar)
- Dark mode support
- Session management with multiple tabs
- Command-line integration (`meld-rs <file1> <file2> [file3]`)

## Requirements

- **Rust** 1.81 or later
- **GTK 4** runtime (4.16+ recommended)
- **GtkSourceView 5**
- **libadwaita**

### Platform-specific setup

**Linux (Debian/Ubuntu):**
```bash
sudo apt install libgtk-4-dev libgtksourceview-5-dev libadwaita-1-dev
```

**Linux (Fedora):**
```bash
sudo dnf install gtk4-devel gtksourceview5-devel libadwaita-devel
```

**Windows:**
Install GTK 4 via [MSYS2](https://www.msys2.org/):
```bash
pacman -S mingw-w64-x86_64-gtk4 mingw-w64-x86_64-gtksourceview5 mingw-w64-x86_64-libadwaita
```

**macOS:**
```bash
brew install gtk4 gtksourceview5 libadwaita
```

## Building

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Or use the provided scripts
./scripts/build.sh        # Linux/macOS
./scripts/run.sh          # Linux/macOS (build & run)
scripts\build.ps1         # Windows (PowerShell, detects MSYS2)
scripts\build.bat         # Windows (CMD)
scripts\run.ps1           # Windows (PowerShell, build & run)
```

## Running

```bash
# Compare two files
meld-rs file_a.txt file_b.txt

# Compare two directories
meld-rs dir_a/ dir_b/

# 3-way comparison
meld-rs left.txt middle.txt right.txt

# 3-way auto-merge
meld-rs base.txt local.txt remote.txt --auto-merge -o merged.txt

# Open a new tab in an existing instance
meld-rs --newtab file_a.txt file_b.txt

# Auto-compare all differing files in directories
meld-rs dir_a/ dir_b/ --auto-compare

# Open version control view for a repository
meld-rs /path/to/repo/

# Show help
meld-rs --help
```

When running from source without installing:

```bash
cargo run -- <file1> <file2>
```

## Running Tests

The `gui` feature is enabled by default, which links against GTK 4 and requires
the GTK runtime libraries to be available. To run the diff engine and integration
tests without GUI dependencies:

```bash
cargo test --no-default-features
```

To also run GUI-related tests (requires GTK 4 runtime installed):

```bash
cargo test
```

## License

Meld-rs is licensed under the GNU General Public License v2.0 or later (GPL-2.0-or-later),
the same license as the original Meld.

See [LICENSE](LICENSE) for the full text.

## Contributing

Contributions are welcome. Please ensure your code:
- Passes `cargo fmt` and `cargo clippy`
- Passes `cargo test --no-default-features`
- Includes tests for new functionality
- Follows the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)

See `scripts/test.sh` for a quick test runner.
