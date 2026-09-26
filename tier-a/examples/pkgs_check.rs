use std::collections::HashMap;
use std::fs;
use std::path::Path;

fn walk(dir: &Path, base: &str, out: &mut HashMap<String, String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let name = entry.file_name().into_string().unwrap();
        let rel = if base.is_empty() { name.clone() } else { format!("{base}/{name}") };
        if path.is_dir() {
            walk(&path, &rel, out);
        } else if name.ends_with(".nix") {
            out.insert(format!("nixpkgs/{rel}"), fs::read_to_string(&path).unwrap());
        }
    }
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = HashMap::new();
    walk(&root.join("nixpkgs-lib/lib"), "lib", &mut files);
    files.insert(
        "nixpkgs/default.nix".to_string(),
        fs::read_to_string(root.join("pkgs/default.nix")).unwrap(),
    );
    files.insert(
        "entry.nix".to_string(),
        std::env::args().nth(1).unwrap_or_else(|| {
            "(import <nixpkgs> {}).hello.drvPath".to_string()
        }),
    );

    let req = serde_json::json!({
        "files": files,
        "entry": "entry.nix",
        "timeoutMs": 20000,
        "strict": true,
    });
    let raw = nixbox_tier_a::eval_json(&req.to_string());
    println!("{raw}");
}
