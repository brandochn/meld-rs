/// Build script: compiles GSettings schemas for the `gui` feature.
///
/// Copies the schema XML to `target/share/glib-2.0/schemas/` and runs
/// `glib-compile-schemas` if available. This is non-fatal — if the tool
/// is not present, the schema is still copied and can be compiled later
/// by the packaging step.
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());

    // Navigate from OUT_DIR (target/debug/build/meld-rs-xxx/out) up to target/
    let schema_dir = out_dir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(|p| p.join("share").join("glib-2.0").join("schemas"));

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

        // Non-fatal: glib-compile-schemas may not be on PATH
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
                eprintln!("build.rs: glib-compile-schemas not found ({e}) — schemas not compiled");
            }
        }
    }
}
