//! Build script: embeds the Hub UI (`ui/`) and the fixture catalog into the exe, and on Windows
//! adds the FastKiwi icon and version resource.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("read ui dir")
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(root, &p, out);
        } else {
            let rel = p
                .strip_prefix(root)
                .expect("under root")
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, p));
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let ui = manifest.join("ui");
    println!("cargo:rerun-if-changed=ui");
    println!("cargo:rerun-if-changed=catalog");
    println!("cargo:rerun-if-changed=assets/fastkiwi.ico");

    let mut files = Vec::new();
    walk(&ui, &ui, &mut files);
    let mut src = String::from("/// Every file under `ui/`, as (path relative to `ui/`, bytes).\n");
    src.push_str("pub static UI_FILES: &[(&str, &[u8])] = &[\n");
    for (rel, path) in &files {
        writeln!(
            src,
            "    ({rel:?}, include_bytes!({:?})),",
            path.to_string_lossy()
        )
        .expect("write");
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir")).join("ui_files.rs");
    std::fs::write(out, src).expect("write ui_files.rs");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/fastkiwi.ico");
        res.set("ProductName", "FastKiwi");
        res.set("FileDescription", "FastKiwi Hub (prototype)");
        res.set("CompanyName", "FastKiwi");
        res.set("LegalCopyright", "FastKiwi");
        res.set("OriginalFilename", "FastKiwi.exe");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icon resource not added: {e}");
        }
    }
}
