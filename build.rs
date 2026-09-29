/// Build script: compiles GSettings schemas, GResources, and gettext catalogs
/// for the `gui` feature.
///
/// 1. Copies GSettings schema XML to `target/share/glib-2.0/schemas/`
///    and runs `glib-compile-schemas` if available.
/// 2. Compiles the missing RelaxNG schema into a GResource so that
///    GtkSourceView can load language-spec `.lang` files on Windows/MSYS2
///    (where the bundled `language-specs.gresource` is missing `language2.rng`).
/// 3. Compiles the vendored gettext catalogs (`po/*.po`) with `msgfmt` into
///    `target/share/locale/<lang>/LC_MESSAGES/meld-rs.mo` so the runtime can
///    load them.  If `msgfmt` is unavailable the UI gracefully falls back to
///    English and a warning is emitted.
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo::rustc-check-cfg=cfg(gresource_available)");
    println!("cargo:rerun-if-changed=po");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());

    compile_schemas(&out_dir);
    compile_gresource(&out_dir);
    compile_translations(&out_dir);
}

/// Directory shared with the schema/gresource/locale outputs
/// (`target/share`), derived from `OUT_DIR` (target/debug/build/meld-rs-xxx/out).
fn target_share_dir(out_dir: &Path) -> Option<PathBuf> {
    out_dir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(|p| p.join("share"))
}

fn compile_schemas(out_dir: &Path) {
    // ── GSettings schema ──────────────────────────────────────────────
    let schema_dir = target_share_dir(out_dir).map(|p| p.join("glib-2.0").join("schemas"));

    let schema_src = "resources/gschemas/org.gnome.meld-rs.gschema.xml";

    if let Some(ref dir) = schema_dir {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!(
                "build.rs: failed to create schema dir {}: {e}",
                dir.display()
            );
            return;
        }

        let dst = dir.join("org.gnome.meld-rs.gschema.xml");
        if let Err(e) = std::fs::copy(schema_src, &dst) {
            eprintln!("build.rs: failed to copy schema: {e}");
            return;
        }

        match std::process::Command::new("glib-compile-schemas")
            .arg(dir)
            .status()
        {
            Ok(status) if status.success() => {
                println!("cargo:warning=GSettings schemas compiled successfully");
            }
            Ok(status) => {
                eprintln!("build.rs: glib-compile-schemas exited with {status}");
            }
            Err(e) => {
                eprintln!("build.rs: glib-compile-schemas not found ({e})");
            }
        }
    }
}

fn compile_gresource(out_dir: &Path) {
    // ── GResource: language2.rng schema ────────────────────────────────
    let gresource_xml = "resources/gresources/meld-language-schema.gresource.xml";
    let gresource_out = out_dir.join("meld-language-schema.gresource");

    match std::process::Command::new("glib-compile-resources")
        .arg("--target")
        .arg(&gresource_out)
        .arg(&format!(
            "--sourcedir={}",
            std::path::Path::new("resources/gresources").display()
        ))
        .arg(gresource_xml)
        .status()
    {
        Ok(status) if status.success() => {
            println!("cargo:warning=GResource compiled successfully");
            println!("cargo:rustc-cfg=gresource_available");
        }
        Ok(status) => {
            eprintln!("build.rs: glib-compile-resources exited with {status}");
        }
        Err(e) => {
            eprintln!("build.rs: glib-compile-resources not found ({e})");
        }
    }
}

/// Compile every vendored `po/<lang>.po` catalog into
/// `target/share/locale/<lang>/LC_MESSAGES/meld-rs.mo`.
fn compile_translations(out_dir: &Path) {
    let Some(locale_dir) = target_share_dir(out_dir).map(|p| p.join("locale")) else {
        eprintln!("build.rs: cannot determine target share dir; skipping translations");
        return;
    };

    let po_dir = Path::new("po");
    let entries = match std::fs::read_dir(po_dir) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("build.rs: cannot read {}: {e}", po_dir.display());
            return;
        }
    };

    let mut total = 0usize;
    let mut compiled = 0usize;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("po") {
            continue;
        }
        let Some(lang) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        total += 1;

        let dest_dir = locale_dir.join(lang).join("LC_MESSAGES");
        if let Err(e) = std::fs::create_dir_all(&dest_dir) {
            eprintln!(
                "build.rs: failed to create locale dir {}: {e}",
                dest_dir.display()
            );
            continue;
        }
        let dest = dest_dir.join("meld-rs.mo");

        match std::process::Command::new("msgfmt")
            .arg("-o")
            .arg(&dest)
            .arg(&path)
            .status()
        {
            Ok(status) if status.success() => compiled += 1,
            Ok(status) => {
                eprintln!(
                    "build.rs: msgfmt exited with {status} for {}",
                    path.display()
                );
            }
            Err(e) => {
                eprintln!("build.rs: msgfmt not found ({e}); UI text will fall back to English");
                return;
            }
        }
    }

    if compiled > 0 {
        println!("cargo:warning=Compiled {compiled}/{total} gettext catalogs");
    }
}
