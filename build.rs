// Embeds every logo in assets/logos into the binary as a sorted
// (name, contents) table, so lookups are a binary search and the
// program needs no data files at runtime.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").unwrap();
    let dir = Path::new(&manifest).join("assets").join("logos");
    println!("cargo:rerun-if-changed=assets/logos");

    let mut names: Vec<String> = fs::read_dir(&dir)
        .expect("assets/logos is missing")
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            name.strip_suffix(".txt").map(str::to_owned)
        })
        .filter(|n| n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .collect();
    names.sort();

    let mut out = String::from("pub static LOGOS: &[(&str, &str)] = &[\n");
    for name in &names {
        out.push_str(&format!(
            "    ({name:?}, include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/assets/logos/{name}.txt\"))),\n"
        ));
    }
    out.push_str("];\n");

    let dest = Path::new(&env::var("OUT_DIR").unwrap()).join("logos.rs");
    fs::write(dest, out).unwrap();
}
