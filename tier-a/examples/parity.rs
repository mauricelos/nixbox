//! Native (non-wasm) parity runner for fast iteration: runs
//! `spikes/parity/snippets/*` through `eval_json` directly in-process (no
//! wasm, no worker) and compares against the `.expected` files. Mirrors
//! `spikes/snix-wasm/parity/lib.mjs`'s comparison rules. The real parity
//! suite (vitest, against the committed wasm build) lives elsewhere; this
//! is a development-only tool.
use std::collections::HashMap;
use std::fs;
use std::path::Path;

fn primary_error_message(expected: &str) -> Option<String> {
    let mut last = None;
    for line in expected.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("error: ") {
            last = Some(rest.to_string());
        }
    }
    last
}

fn main() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spikes/parity/snippets");
    let mut entries: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap()).collect();
    entries.sort_by_key(|e| e.file_name());

    let mut pass = 0;
    let mut total = 0;
    let mut fails = Vec::new();

    for entry in entries {
        let path = entry.path();
        let file_name = entry.file_name().into_string().unwrap();
        let (name, files, nix_entry, expected) = if path.is_dir() {
            let mut files = HashMap::new();
            let mut expected = None;
            for f in fs::read_dir(&path).unwrap() {
                let f = f.unwrap();
                let fname = f.file_name().into_string().unwrap();
                if fname.ends_with(".nix") {
                    files.insert(fname.clone(), fs::read_to_string(f.path()).unwrap());
                } else if fname.ends_with(".expected") {
                    expected = Some(fs::read_to_string(f.path()).unwrap());
                }
            }
            (file_name.clone(), files, "default.nix".to_string(), expected.unwrap())
        } else if file_name.ends_with(".nix") {
            let name = file_name.trim_end_matches(".nix").to_string();
            let expected_path = dir.join(format!("{name}.expected"));
            if !expected_path.exists() {
                continue;
            }
            let mut files = HashMap::new();
            files.insert(file_name.clone(), fs::read_to_string(&path).unwrap());
            (
                name,
                files,
                file_name.clone(),
                fs::read_to_string(expected_path).unwrap(),
            )
        } else {
            continue;
        };

        total += 1;
        let req = serde_json::json!({
            "files": files,
            "entry": nix_entry,
            "timeoutMs": 5000,
            "strict": true,
        });
        let raw = nixbox_tier_a::eval_json(&req.to_string());
        let result: serde_json::Value = serde_json::from_str(&raw).unwrap();

        let trimmed_expected = expected.trim_end_matches('\n');
        let is_error_expected = trimmed_expected.starts_with("error:")
            || expected.contains("\nerror: ")
            || expected.starts_with("error");

        let (ok, detail) = if is_error_expected {
            if result["ok"].as_bool().unwrap_or(false) {
                (
                    false,
                    format!("expected error, got ok printed={:?}", result.get("printed")),
                )
            } else {
                let want = primary_error_message(&expected);
                let got = result.get("message").and_then(|m| m.as_str()).map(|s| s.to_string());
                let ok = want.is_some() && want == got;
                (
                    ok,
                    if ok {
                        String::new()
                    } else {
                        format!("expected primary message {want:?}, got {got:?}")
                    },
                )
            }
        } else if !result["ok"].as_bool().unwrap_or(false) {
            (false, format!("expected ok, got error: {:?}", result.get("message")))
        } else {
            let got = result.get("printed").and_then(|p| p.as_str()).unwrap_or("");
            let ok = got == trimmed_expected;
            (
                ok,
                if ok {
                    String::new()
                } else {
                    format!("expected {trimmed_expected:?}, got {got:?}")
                },
            )
        };

        if ok {
            pass += 1;
            println!("PASS  {name}");
        } else {
            println!("FAIL  {name}  -- {detail}");
            fails.push(name);
        }
    }

    println!("\n{pass}/{total} passed");
    if !fails.is_empty() {
        println!("failing: {}", fails.join(", "));
    }
}
