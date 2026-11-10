//! Compiles the bundled connector packages (`integrations/connectors/<id>/`, docs/adr/0007-connectors.md) into the
//! server: `OUT_DIR/bundled_connectors.rs` lists every file of every package with `include_str!`, read by
//! `connectors::registry` at startup.

use std::path::{Path, PathBuf};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        let hidden = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.'));
        if hidden {
            continue;
        }
        if path.is_dir() {
            files(&path, out);
        } else if std::fs::read_to_string(&path).is_ok() {
            out.push(path);
        }
    }
}

fn main() {
    let root = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../../integrations/connectors");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut all = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        dirs.sort();
        for dir in dirs {
            files(&dir, &mut all);
        }
    }
    let mut code = String::from("/// (package directory, path in the package, contents) of every bundled connector file.\npub const FILES: &[(&str, &str, &str)] = &[\n");
    for path in &all {
        let rel = path.strip_prefix(&root).unwrap();
        let mut parts = rel.components();
        let package = parts.next().unwrap().as_os_str().to_string_lossy().to_string();
        let inner = parts.as_path().to_string_lossy().replace('\\', "/");
        let abs = path.canonicalize().unwrap();
        code.push_str(&format!("    ({package:?}, {inner:?}, include_str!({:?})),\n", abs.display().to_string()));
    }
    code.push_str("];\n");
    std::fs::write(Path::new(&std::env::var("OUT_DIR").unwrap()).join("bundled_connectors.rs"), code).unwrap();
}
