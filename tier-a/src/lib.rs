//! nixbox-tier-a: wraps the `tvix-eval` (a.k.a. snix-eval) Nix language
//! evaluator, plus a `derivation`/`derivationStrict` builtin built on
//! `nix-compat`, for use from a Web Worker via wasm-bindgen.
//!
//! Productionized from `spikes/snix-wasm/` (see `docs/spikes/A2-snix.md`).
//! The public surface (`eval_json`) matches `EvalRequest` / `EvalResult` in
//! `lib/runtime/types.ts`, serialized as JSON so the worker glue can stay
//! backend-agnostic.
//!
//! See `nixbox/README.md` for the vendoring rationale, the `tvix-eval`
//! patch (a few `ErrorKind` variants now carry the offending value's
//! printed text) and the CppNix error-message mapping this file implements.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde::{Deserialize, Serialize};
use tvix_eval::{EvalIO, EvalMode, Evaluation, FileType};
use wasm_bindgen::prelude::*;

mod cppnix;
mod pkgs_builtins;

/// Request shape, mirrors `EvalRequest` in lib/runtime/types.ts.
#[derive(Deserialize)]
struct EvalRequest {
    files: HashMap<String, String>,
    entry: String,
    #[allow(dead_code)]
    #[serde(rename = "timeoutMs")]
    timeout_ms: Option<u64>,
    strict: Option<bool>,
}

/// Response shape, mirrors `EvalResult` (EvalSuccess | EvalFailure).
#[derive(Serialize)]
struct EvalResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    printed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    json: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "errorClass")]
    error_class: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    col: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    trace: Option<Vec<String>>,
}

/// An in-memory virtual filesystem implementing tvix-eval's `EvalIO` trait.
/// Paths are normalized relative to the virtual root ("./x.nix" == "x.nix"
/// == "/x.nix") so nothing ever touches the host filesystem.
pub(crate) struct VfsIo {
    files: RefCell<HashMap<String, Vec<u8>>>,
}

fn normalize(path: &Path) -> String {
    let mut out: Vec<&str> = Vec::new();
    for comp in path.components() {
        match comp {
            std::path::Component::Normal(s) => {
                if let Some(s) = s.to_str() {
                    out.push(s);
                }
            }
            std::path::Component::ParentDir => {
                out.pop();
            }
            _ => {}
        }
    }
    out.join("/")
}

impl VfsIo {
    /// `entry` is aliased in as a workaround for a real tvix-eval quirk on
    /// wasm32: `Compiler::new` decides whether a `location` PathBuf is a
    /// directory or a file via `std::fs::Path::is_file()` to decide whether
    /// to pop the last segment before resolving relative imports — but on
    /// wasm32 there is no real filesystem, so `is_file()` is always false
    /// and the entry file's own path is used, unpopped, as the import
    /// root. A one-file `import ./sibling.nix` from `entry` therefore
    /// resolves to `"<entry>/sibling.nix"` instead of `"sibling.nix"`. We
    /// work around it by also serving every file at `"<entry>/<name>"`, one
    /// level deep. See docs/spikes/A2-snix.md ("import + virtual FS").
    fn new(files: HashMap<String, String>, entry: &str) -> Self {
        let entry_key = normalize(Path::new(entry));
        let mut normalized: HashMap<String, Vec<u8>> = HashMap::new();
        for (k, v) in files {
            let key = normalize(Path::new(&k));
            let bytes = v.into_bytes();
            if key != entry_key {
                normalized.insert(format!("{entry_key}/{key}"), bytes.clone());
            }
            normalized.insert(key, bytes);
        }
        VfsIo {
            files: RefCell::new(normalized),
        }
    }
}

impl EvalIO for VfsIo {
    fn path_exists(&self, path: &Path) -> io::Result<bool> {
        let key = normalize(path);
        let files = self.files.borrow();
        Ok(files.contains_key(&key) || files.keys().any(|k| k.starts_with(&format!("{key}/"))))
    }

    fn open(&self, path: &Path) -> io::Result<Box<dyn io::Read>> {
        let key = normalize(path);
        let files = self.files.borrow();
        if let Some(bytes) = files.get(&key) {
            return Ok(Box::new(Cursor::new(bytes.clone())));
        }
        // Workaround: tvix-eval's `import` builtin decides whether to
        // append "default.nix" to a directory import via
        // `std::path::Path::is_dir()`, which always returns false on
        // wasm32-unknown-unknown (there is no real filesystem to check).
        // `import ./some-dir` is therefore silently broken upstream on
        // wasm — see docs/spikes/A2-snix.md ("import + virtual FS"). We
        // paper over it here: if `key` isn't itself a file but
        // `key/default.nix` exists, serve that.
        let default_key = format!("{key}/default.nix");
        if let Some(bytes) = files.get(&default_key) {
            return Ok(Box::new(Cursor::new(bytes.clone())));
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no such virtual file: {key}"),
        ))
    }

    fn file_type(&self, path: &Path) -> io::Result<FileType> {
        let key = normalize(path);
        let files = self.files.borrow();
        if files.contains_key(&key) {
            Ok(FileType::Regular)
        } else if files.keys().any(|k| k.starts_with(&format!("{key}/"))) {
            Ok(FileType::Directory)
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "no such virtual path"))
        }
    }

    fn read_dir(&self, path: &Path) -> io::Result<Vec<(bytes::Bytes, FileType)>> {
        let key = normalize(path);
        let prefix = format!("{key}/");
        let files = self.files.borrow();
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for k in files.keys() {
            if let Some(rest) = k.strip_prefix(&prefix) {
                let name = rest.split('/').next().unwrap_or(rest);
                if seen.insert(name.to_string()) {
                    let is_dir = rest.contains('/');
                    out.push((
                        bytes::Bytes::from(name.to_string()),
                        if is_dir { FileType::Directory } else { FileType::Regular },
                    ));
                }
            }
        }
        Ok(out)
    }

    fn import_path(&self, path: &Path) -> io::Result<PathBuf> {
        // Virtual FS: nothing to copy into a store, return as-is.
        Ok(path.to_path_buf())
    }

    fn store_dir(&self) -> Option<String> {
        Some(nix_compat::store_path::STORE_DIR.to_string())
    }
}

/// The well-known `corepkgs/derivation.nix` wrapper that real Nix evaluates
/// `derivation` as: it calls the native `derivationStrict` builtin (which
/// we implement in `pkgs_builtins` on top of `nix-compat`, matching real
/// Nix's store-path hashing bit for bit) and reshapes its `{ drvPath;
/// <output> = <path>; }` result into the full per-output attrset Nix
/// programs expect (`outPath`, `drvPath`, `type = "derivation"`,
/// `outputName`, `all`, and the original attributes). This is the standard
/// upstream Nix implementation (see e.g.
/// https://github.com/NixOS/nix/blob/master/src/libexpr/primops/derivation.nix),
/// not NixQuest's own design, reproduced here because tvix-eval (unlike
/// C++ Nix) doesn't ship it.
const DERIVATION_NIX: &str = r#"
drvAttrs @ { outputs ? [ "out" ], ... }:

let

  strict = derivationStrict drvAttrs;

  commonAttrs = drvAttrs // (builtins.listToAttrs outputsList) //
    { all = map (x: x.value) outputsList;
      inherit drvAttrs;
    };

  outputToAttrListElement = outputName:
    { name = outputName;
      value = commonAttrs // {
        outPath = builtins.getAttr outputName strict;
        drvPath = strict.drvPath;
        type = "derivation";
        inherit outputName;
      };
    };

  outputsList = map outputToAttrListElement outputs;

in (builtins.head outputsList).value
"#;

/// Evaluates a small, self-contained expression string (an assert
/// condition's operand, always closed over nothing but literals in the
/// beginner-error cases this is used for) in the same VFS as the main
/// evaluation, strictly, returning the resulting `Value` on success. Used
/// only for CppNix's `assert X == Y;` operand-value wording (see
/// `cppnix::assert_condition_parts`); `None` on any error, so callers fall
/// back to tvix's generic message.
fn eval_subexpr(io: Rc<dyn EvalIO>, code: &str) -> Option<tvix_eval::Value> {
    let eval = Evaluation::builder(io).mode(EvalMode::Strict).build();
    let result = eval.evaluate(code.to_string(), None);
    if !result.errors.is_empty() {
        return None;
    }
    match result.value {
        Some(tvix_eval::Value::Catchable(_)) | None => None,
        Some(v) => Some(v),
    }
}

fn run_eval(req: EvalRequest) -> EvalResponse {
    let strict = req.strict.unwrap_or(true);
    let io = Rc::new(VfsIo::new(req.files, &req.entry)) as Rc<dyn EvalIO>;
    let code = match io.open(Path::new(&req.entry)) {
        Ok(mut r) => {
            let mut s = String::new();
            if std::io::Read::read_to_string(&mut r, &mut s).is_err() {
                return EvalResponse::failure(format!(
                    "entry file is not valid UTF-8: {}",
                    req.entry
                ));
            }
            s
        }
        Err(_) => {
            return EvalResponse::failure(format!("entry file not found: {}", req.entry));
        }
    };

    let mode = if strict { EvalMode::Strict } else { EvalMode::default() };
    let io_for_assert = io.clone();

    let eval = Evaluation::builder(io)
        .enable_import()
        .add_builtins(pkgs_builtins::builtins())
        .add_src_builtin("derivation", DERIVATION_NIX)
        // `<nixpkgs>` / `<nixpkgs/lib>`: an *absolute* virtual path, not a
        // relative one — `NixSearchPath::resolve`'s relative-path branch
        // calls `std::env::current_dir()`, which is unavailable on
        // wasm32-unknown-unknown (see nixbox/README.md and the "probably
        // panics in wasm" comment in the vendored
        // src/nix_search_path.rs). An absolute path skips that entirely.
        .nix_path(Some("nixpkgs=/nixpkgs".to_string()))
        .mode(mode)
        .build();

    let result = eval.evaluate(code, Some(PathBuf::from(&req.entry)));

    if let Some(err) = result.errors.first() {
        let source = err.source.clone();
        let leaf = cppnix::leaf_error(err);

        // Return line/col of the *innermost* error, like CppNix does,
        // unless the message mapper (syntax errors, see cppnix.rs) found a
        // more accurate one from the parser's own token range.
        let (default_line, default_col) = (
            source.get_line(leaf.span) as u32,
            source.codemap().look_up_span(leaf.span).begin.column as u32 + 1,
        );
        let (mut message, override_loc) = cppnix::message_for(&source, leaf);
        let (line, col) = override_loc.unwrap_or((default_line, default_col));
        let error_class = cppnix::classify(&leaf.kind);

        // Trace frames ("… while evaluating …"), outermost last: still
        // extracted from the diagnostic-rendering path (fancy_format_str),
        // which walks the NativeError/BytecodeError chain the same way
        // `tvix-cli` does. Not part of primary-message parity (see
        // spikes/parity/README.md) but useful for a future error-explainer
        // trace view.
        let fancy = err.fancy_format_str();
        let mut trace: Vec<String> = Vec::new();
        for line in fancy.lines() {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix('…') {
                trace.push(rest.trim().to_string());
            }
        }

        // CppNix's special "an integer with value '1' is not equal to an
        // integer with value '2'" wording for a failed `assert X == Y;` /
        // `assert X != Y;` — see cppnix::assert_condition_parts. Only
        // attempted for plain assertion failures (not e.g. a `throw`
        // inside the condition), and silently falls back to tvix's
        // generic "assertion failed" if the operands don't evaluate
        // cleanly (e.g. the condition wasn't a simple `==`/`!=`).
        if error_class == "assertion" {
            let condition_text = source.source_slice(leaf.span).to_string();
            if let Some((lhs, op, rhs)) = cppnix::assert_condition_parts(&condition_text) {
                if let (Some(lv), Some(rv)) = (
                    eval_subexpr(io_for_assert.clone(), &lhs),
                    eval_subexpr(io_for_assert.clone(), &rhs),
                ) {
                    let verb = if op == "==" { "is not equal to" } else { "is equal to" };
                    message = format!(
                        "{} {verb} {}",
                        cppnix::assert_operand_text(&lv),
                        cppnix::assert_operand_text(&rv)
                    );
                }
            }
        }

        return EvalResponse {
            ok: false,
            printed: None,
            json: None,
            message: Some(message),
            error_class: Some(error_class),
            line: Some(line),
            col: Some(col),
            trace: if trace.is_empty() { None } else { Some(trace) },
        };
    }

    match result.value {
        Some(tvix_eval::Value::Catchable(c)) => EvalResponse::failure(format!("{c}")),
        Some(value) => {
            let printed = format!("{value}");
            let json = cppnix::to_json(&value);
            EvalResponse {
                ok: true,
                printed: Some(printed),
                json,
                message: None,
                error_class: None,
                line: None,
                col: None,
                trace: None,
            }
        }
        None => EvalResponse::failure("evaluation produced no value and no error".to_string()),
    }
}

impl EvalResponse {
    fn failure(message: String) -> Self {
        EvalResponse {
            ok: false,
            printed: None,
            json: None,
            message: Some(message),
            error_class: None,
            line: None,
            col: None,
            trace: None,
        }
    }
}

// Runs automatically once the wasm module is instantiated (wasm-bindgen's
// `start` hook); not meant to be called directly. Named distinctly from
// wasm-bindgen's own generated default-export loader (also conventionally
// called `init`) to avoid confusing the two.
#[wasm_bindgen(start)]
fn set_panic_hook() {
    console_error_panic_hook::set_once();
}

/// Evaluate a request given as a JSON string (`EvalRequest`), returning a
/// JSON string (`EvalResult`). Kept as a single string<->string boundary so
/// the worker only needs `JSON.parse`/`JSON.stringify`, no wasm-bindgen
/// object marshalling.
#[wasm_bindgen]
pub fn eval_json(input: &str) -> String {
    let req: EvalRequest = match serde_json::from_str(input) {
        Ok(r) => r,
        Err(e) => {
            let resp = EvalResponse::failure(format!("bad request: {e}"));
            return serde_json::to_string(&resp).unwrap();
        }
    };
    let resp = run_eval(req);
    serde_json::to_string(&resp).unwrap_or_else(|_| {
        "{\"ok\":false,\"message\":\"failed to serialize result\"}".to_string()
    })
}
