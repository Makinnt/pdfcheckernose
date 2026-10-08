fn main() {
    slint_build::compile("ui/app.slint").unwrap();

    // ponytail: copia plana junto al exe, sin crate de assets; build de instalador si hace falta
    let manifest =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let lib = if cfg!(target_os = "windows") {
        "pdfium.dll"
    } else {
        "libpdfium.so"
    };
    let src = manifest.join("assets/pdfium").join(lib);
    println!("cargo:rerun-if-changed=assets/pdfium/{lib}");

    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    if let Some(profile_dir) = out.ancestors().nth(3) {
        let _ = std::fs::copy(&src, profile_dir.join(lib));
        // diccionarios junto al exe para que el binario suelto los encuentre
        let dicts = profile_dir.join("dicts");
        let _ = std::fs::create_dir_all(&dicts);
        for f in ["es_ES.aff", "es_ES.dic", "en_US.aff", "en_US.dic"] {
            println!("cargo:rerun-if-changed=assets/dicts/{f}");
            let _ = std::fs::copy(manifest.join("assets/dicts").join(f), dicts.join(f));
        }
    }
}
