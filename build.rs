/// Build script: compiles GSettings schemas and GResources for the `gui` feature.
///
/// 1. Copies GSettings schema XML to `target/share/glib-2.0/schemas/`
///    and runs `glib-compile-schemas` if available.
/// 2. Compiles the missing RelaxNG schema into a GResource so that
///    GtkSourceView can load language-spec `.lang` files on Windows/MSYS2
///    (where the bundled `language-specs.gresource` is missing `language2.rng`).
use std::path::PathBuf;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(gresource_available)");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap_or_default());

    // ── GSettings schema ──────────────────────────────────────────────
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
