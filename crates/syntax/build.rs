use std::fmt::Write;
use std::path::{Path, PathBuf};
use syntect::parsing::{SyntaxDefinition, SyntaxSet};

fn main() {
    let mut builder = SyntaxSet::load_defaults_newlines().into_builder();
    for name in ["HolyC", "FreeBASIC"] {
        let grammar = format!("src/grammars/{name}.sublime-syntax");
        println!("cargo:rerun-if-changed={grammar}");
        let source = std::fs::read_to_string(grammar).expect("read bundled syntax");
        builder.add(SyntaxDefinition::load_from_str(&source, true, Some(name)).expect("bundled syntax must be valid"));
    }
    let syntaxes = builder.build();
    assert!(syntaxes.find_unlinked_contexts().is_empty());
    let output = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo build output"));
    syntect::dumps::dump_to_uncompressed_file(&syntaxes, output.join("syntaxes.packdump"))
        .expect("write bundled syntaxes");
    embed_languages(&output);
}

/// `assets/languages/<id>/**` as a table of `(id, [(relative path, bytes)])`, sorted, so the
/// bundled languages need no files at run time.
fn embed_languages(output: &Path) {
    // read when the script runs, not when it was compiled: one build script may serve checkouts
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets the package folder");
    let root = Path::new(&manifest).join("../../assets/languages");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut table = String::from("pub static BUNDLED: &[(&str, super::Embedded)] = &[\n");
    let mut folders: Vec<PathBuf> = std::fs::read_dir(&root)
        .map(|entries| entries.flatten().map(|e| e.path()).filter(|p| p.join("language.json").is_file()).collect())
        .unwrap_or_default();
    folders.sort();
    for folder in folders {
        let id = folder.file_name().unwrap().to_str().expect("UTF-8 language folder name");
        let mut files = Vec::new();
        collect(&folder, &folder, &mut files);
        files.sort();
        writeln!(table, "    ({id:?}, &[").unwrap();
        for (relative, path) in files {
            let path = path.canonicalize().expect("bundled language file");
            writeln!(table, "        ({relative:?}, include_bytes!({:?})),", path.display().to_string()).unwrap();
        }
        table.push_str("    ]),\n");
    }
    table.push_str("];\n");
    std::fs::write(output.join("languages.rs"), table).expect("write bundled language table");
}

fn collect(root: &Path, dir: &Path, files: &mut Vec<(String, PathBuf)>) {
    for entry in std::fs::read_dir(dir).expect("read bundled language folder").flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            collect(root, &path, files);
        } else if entry.file_type().is_ok_and(|t| t.is_file()) {
            let relative = path.strip_prefix(root).unwrap().to_str().expect("UTF-8 file name").replace('\\', "/");
            files.push((relative, path));
        }
    }
}
