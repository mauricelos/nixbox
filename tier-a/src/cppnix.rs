//! Maps tvix-eval's `Error`/`ErrorKind` to CppNix 2.28's exact primary
//! error-message wording (the last `error: …` line `nix-instantiate --eval
//! --strict` prints — see `spikes/parity/README.md`), and to the
//! `NixErrorClass` the error explainer (`lib/runtime/explain.ts`) matches
//! on.
//!
//! tvix-eval's own `ErrorKind` messages are semantically equivalent but
//! worded differently (e.g. "variable not found" vs. CppNix's "undefined
//! variable 'y'"), and several don't carry the runtime `Value` CppNix
//! includes in its message at all — `NotCoercibleToString`, `NotCallable`
//! and `TypeError` were patched upstream (see nixbox/README.md) to carry
//! it. The rest of the gap (which function a missing/unexpected argument
//! belongs to, whether an `AttributeNotFound` is a plain `.attr` miss or a
//! function-pattern destructure) is recovered here from the error's span
//! and the original source text, since tvix-eval's compiler discards that
//! context once compiled to bytecode.

use bstr::ByteSlice;
use tvix_eval::{Error, ErrorKind, SourceCode, Value};

/// Walk down `NativeError`/`BytecodeError` wrapper frames to the innermost
/// `Error`, which carries both the real `ErrorKind` and (per the "Return
/// line/col of the innermost error like CppNix does" requirement) the
/// accurate span.
pub(crate) fn leaf_error(err: &Error) -> &Error {
    match &err.kind {
        ErrorKind::NativeError { err: inner, .. } => leaf_error(inner),
        ErrorKind::BytecodeError(inner) => leaf_error(inner),
        _ => err,
    }
}

/// CppNix's noun phrase for a tvix `Value::type_of()` string, e.g. `"int"`
/// -> `"an integer"`, `"bool"` -> `"a Boolean"`. Verified against real Nix
/// 2.28.3 (`nix-instantiate --eval --strict -E 'if 1 then 2 else 3'` ->
/// "expected a Boolean but found an integer: 1", etc.)
fn noun(type_of: &str) -> String {
    match type_of {
        "null" => "null".to_string(),
        "bool" => "a Boolean".to_string(),
        "int" => "an integer".to_string(),
        "float" => "a float".to_string(),
        "string" => "a string".to_string(),
        "path" => "a path".to_string(),
        "set" => "a set".to_string(),
        "list" => "a list".to_string(),
        "lambda" => "a function".to_string(),
        other => format!("a {other}"),
    }
}

/// The exact substring of `source` a span covers.
fn span_text(source: &SourceCode, span: codemap::Span) -> String {
    source.source_slice(span).to_string()
}

/// Local (per-file) byte offset of `pos`, for scanning the raw file text.
fn local_offset(file: &codemap::File, pos: codemap::Pos) -> usize {
    (pos - file.span.low()) as usize
}

/// True when the nearest non-whitespace character before byte offset
/// `from` in `full` is `.` — i.e. `from` is the target of a plain `x.attr`
/// selection, as opposed to a name inside a `{ a, b }:` function pattern
/// (always preceded by `{`, `,`, or whitespace/newline, never `.`).
fn preceded_by_dot(full: &str, from: usize) -> bool {
    let bytes = full.as_bytes();
    let mut i = from.min(bytes.len());
    while i > 0 && (bytes[i - 1] as char).is_whitespace() {
        i -= 1;
    }
    i > 0 && bytes[i - 1] == b'.'
}

/// Scans backward from byte offset `before` (the start of a `{` formals
/// pattern) for a lexical binder `NAME = ` immediately preceding it, e.g.
/// the `f` in `let f = { a, b }: …`. Returns `None` when the lambda isn't
/// directly the right-hand side of a binding (an anonymous lambda, per
/// CppNix's own "anonymous lambda" wording — verified against real Nix).
fn enclosing_binder_name(full: &str, before: usize) -> Option<String> {
    let bytes = full.as_bytes();
    let mut i = before.min(bytes.len());
    while i > 0 && (bytes[i - 1] as char).is_whitespace() {
        i -= 1;
    }
    if i == 0 || bytes[i - 1] != b'=' {
        return None;
    }
    i -= 1;
    while i > 0 && (bytes[i - 1] as char).is_whitespace() {
        i -= 1;
    }
    let end = i;
    while i > 0 {
        let c = bytes[i - 1] as char;
        if c.is_alphanumeric() || c == '_' || c == '\'' || c == '-' {
            i -= 1;
        } else {
            break;
        }
    }
    if i == end {
        return None;
    }
    let ident = &full[i..end];
    let first = ident.chars().next()?;
    if !(first.is_alphabetic() || first == '_') {
        return None;
    }
    Some(ident.to_string())
}

/// Scans backward from byte offset `from` for the nearest unmatched `{`,
/// tracking brace depth. Used to find a formals pattern's own opening
/// brace starting from one of its (failing) parameter names, since
/// `ErrorKind::AttributeNotFound` for a missing formal only carries the
/// parameter's own span, not the pattern's.
fn enclosing_brace_offset(full: &str, from: usize) -> Option<usize> {
    let bytes = full.as_bytes();
    let mut depth: i32 = 0;
    let mut i = from.min(bytes.len());
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b'}' => depth += 1,
            b'{' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

fn function_name(full: &str, formals_open_brace: usize) -> String {
    enclosing_binder_name(full, formals_open_brace).unwrap_or_else(|| "anonymous lambda".to_string())
}

/// CppNix's name for an rnix `SyntaxKind` when it appears in an "expecting
/// X" clause — only the handful of keywords/punctuation that show up in
/// the beginner errors this targets (PLAN.md 4.13); anything else is a
/// known gap (see `message_for`'s ParseErrors arm).
fn expecting_name(kind: rnix::SyntaxKind) -> Option<&'static str> {
    use rnix::SyntaxKind::*;
    Some(match kind {
        TOKEN_ELSE => "ELSE",
        TOKEN_THEN => "THEN",
        TOKEN_IN => "IN",
        TOKEN_SEMICOLON => "\";\"",
        TOKEN_R_BRACE => "\"}\"",
        TOKEN_R_PAREN => "\")\"",
        TOKEN_R_BRACK => "\"]\"",
        TOKEN_ASSIGN => "\"=\"",
        _ => return None,
    })
}

/// The exact source text a parser `TextRange` covers, given the file it
/// belongs to (rnix ranges are local byte offsets into that one file's
/// text, unlike `codemap::Span`, which is a global offset across every
/// file tvix-eval has loaded).
fn range_text(file: &codemap::File, range: rowan::TextRange) -> &str {
    let full = file.source();
    &full[usize::from(range.start())..usize::from(range.end()).min(full.len())]
}

/// 1-indexed (line, col) of a byte offset into `file`'s source, computed
/// the same way `codemap` does it elsewhere in this module (count
/// newlines, then chars on the final line) — rnix's `TextRange` offsets
/// aren't `codemap::Pos`, so `SourceCode`/`File`'s own line/col helpers
/// don't apply directly.
fn line_col_at(file: &codemap::File, byte_offset: usize) -> (u32, u32) {
    let full = file.source();
    let offset = byte_offset.min(full.len());
    let mut line = 1u32;
    let mut last_newline = 0usize;
    for (i, b) in full.as_bytes()[..offset].iter().enumerate() {
        if *b == b'\n' {
            line += 1;
            last_newline = i + 1;
        }
    }
    let col = full[last_newline..offset].chars().count() as u32 + 1;
    (line, col)
}

/// CppNix-style rendering of an rnix parse error: the exact source text at
/// the reported `TextRange` for the "unexpected X" part (so it's right
/// whatever the token is — an identifier, a keyword, or a single
/// punctuation character), and a small keyword table (`expecting_name`)
/// for the "expecting X" clause on the "reached end of file" forms.
/// Returns the message plus the (1-indexed line, col) CppNix would report
/// — the location of the *unexpected* token/position, which for a parse
/// error is not necessarily where `Error::span` points (that's often the
/// span of the whole enclosing node tvix-eval's compiler was building).
fn syntax_message(
    file: &codemap::File,
    errors: &[rnix::parser::ParseError],
) -> (String, u32, u32) {
    use rnix::parser::ParseError as PE;
    match errors.first() {
        Some(PE::Unexpected(range)) | Some(PE::UnexpectedExtra(range)) => {
            let text = range_text(file, *range);
            let (line, col) = line_col_at(file, usize::from(range.start()));
            (format!("syntax error, unexpected '{text}'"), line, col)
        }
        Some(PE::UnexpectedWanted(_got, range, wanted)) => {
            let text = range_text(file, *range);
            let (line, col) = line_col_at(file, usize::from(range.start()));
            let expecting = wanted.first().copied().and_then(expecting_name);
            let msg = match expecting {
                Some(w) => format!("syntax error, unexpected '{text}', expecting {w}"),
                None => format!("syntax error, unexpected '{text}'"),
            };
            (msg, line, col)
        }
        Some(PE::UnexpectedDoubleBind(range)) => {
            let text = range_text(file, *range);
            let (line, col) = line_col_at(file, usize::from(range.start()));
            (format!("syntax error, duplicate formal function argument '{text}'"), line, col)
        }
        Some(PE::UnexpectedEOF) => {
            let (line, col) = line_col_at(file, file.source().len());
            ("syntax error, unexpected end of file".to_string(), line, col)
        }
        Some(PE::UnexpectedEOFWanted(wanted)) => {
            let (line, col) = line_col_at(file, file.source().len());
            let expecting = wanted.first().copied().and_then(expecting_name);
            let msg = match expecting {
                Some(w) => format!("syntax error, unexpected end of file, expecting {w}"),
                None => "syntax error, unexpected end of file".to_string(),
            };
            (msg, line, col)
        }
        Some(PE::DuplicatedArgs(range, name)) => {
            let (line, col) = line_col_at(file, usize::from(range.start()));
            (format!("attribute '{name}' already defined"), line, col)
        }
        Some(PE::RecursionLimitExceeded) | None => ("syntax error".to_string(), 1, 1),
        Some(_) => ("syntax error".to_string(), 1, 1),
    }
}

/// Build the CppNix-worded primary message for a leaf tvix-eval error.
/// Falls back to tvix's own (differently-worded but same-class) message
/// when no CppNix-exact mapping is implemented — see nixbox/README.md's
/// parity table for what's covered.
pub(crate) fn message_for(source: &SourceCode, err: &Error) -> (String, Option<(u32, u32)>) {
    let message = match &err.kind {
        ErrorKind::ParseErrors(errors) => {
            let file = source.get_file(err.span);
            let (msg, line, col) = syntax_message(&file, errors);
            return (msg, Some((line, col)));
        }
        ErrorKind::ImportParseError { errors, file, .. } => {
            let (msg, line, col) = syntax_message(file, errors);
            return (msg, Some((line, col)));
        }

        ErrorKind::UnknownStaticVariable => {
            let name = span_text(source, err.span);
            format!("undefined variable '{name}'")
        }
        ErrorKind::UnknownDynamicVariable(name) => {
            format!("undefined variable '{name}'")
        }

        ErrorKind::AttributeNotFound { name } => {
            let file = source.get_file(err.span);
            let local = local_offset(&file, err.span.low());
            let full = file.source();
            if preceded_by_dot(full, local) {
                format!("attribute '{name}' missing")
            } else {
                let fname = enclosing_brace_offset(full, local)
                    .map(|brace| function_name(full, brace))
                    .unwrap_or_else(|| "anonymous lambda".to_string());
                format!("function '{fname}' called without required argument '{name}'")
            }
        }

        ErrorKind::UnexpectedArgumentFormals { arg, formals_span } => {
            let file = source.get_file(*formals_span);
            let local = local_offset(&file, formals_span.low());
            let fname = function_name(file.source(), local);
            format!(
                "function '{fname}' called with unexpected argument '{}'",
                arg.to_str_lossy()
            )
        }

        ErrorKind::NotCoercibleToString { from, value, .. } => {
            format!("cannot coerce {} to a string: {value}", noun(from))
        }

        ErrorKind::NotCallable { type_name, value } => {
            let what = if type_name.starts_with("set") {
                "a set".to_string()
            } else {
                noun(type_name)
            };
            format!("attempt to call something which is not a function but {what}: {value}")
        }

        ErrorKind::TypeError {
            expected,
            actual,
            value,
        } => {
            format!(
                "expected {} but found {}: {value}",
                noun(expected),
                noun(actual)
            )
        }

        ErrorKind::DivisionByZero => "division by zero".to_string(),

        ErrorKind::InfiniteRecursion { .. } => "infinite recursion encountered".to_string(),

        ErrorKind::CatchableError(cek) => match cek {
            tvix_eval::CatchableErrorKind::Throw(msg) => {
                format!("{}", msg.to_str_lossy())
            }
            tvix_eval::CatchableErrorKind::AssertionFailed => "assertion failed".to_string(),
            other => format!("{other}"),
        },

        ErrorKind::Abort(msg) => format!("evaluation aborted with the following error message: '{msg}'"),

        other => other.to_string(),
    };
    (message, None)
}

/// Classify a leaf `ErrorKind` into the `NixErrorClass` the error explainer
/// (`lib/runtime/explain.ts`) and parity suite key off. Independent of
/// whatever CppNix wording was achieved above — every class here matches
/// tvix's error *kind* directly, which is reliable even when the message
/// text isn't (yet) verbatim.
pub(crate) fn classify(kind: &ErrorKind) -> &'static str {
    match kind {
        ErrorKind::ParseErrors(_) | ErrorKind::ImportParseError { .. } => "syntax",
        ErrorKind::UnknownStaticVariable | ErrorKind::UnknownDynamicVariable(_) => {
            "undefined-variable"
        }
        ErrorKind::AttributeNotFound { .. } => "missing-attribute",
        ErrorKind::UnexpectedArgumentFormals { .. } | ErrorKind::UnexpectedArgumentBuiltin(_) => {
            "unexpected-argument"
        }
        ErrorKind::NotCoercibleToString { .. } => "coercion",
        ErrorKind::NotCallable { .. } => "type",
        ErrorKind::TypeError { .. } => "type",
        ErrorKind::DivisionByZero => "division-by-zero",
        ErrorKind::InfiniteRecursion { .. } => "infinite-recursion",
        ErrorKind::CatchableError(tvix_eval::CatchableErrorKind::AssertionFailed) => "assertion",
        ErrorKind::CatchableError(tvix_eval::CatchableErrorKind::Throw(_)) => "throw",
        ErrorKind::Abort(_) => "abort",
        _ => "other",
    }
}

/// CppNix special-cases a failed `assert LHS == RHS;`/`assert LHS != RHS;`
/// to report both operands' values ("an integer with value '1' is not
/// equal to an integer with value '2'") rather than a bare "assertion
/// failed". tvix-eval doesn't carry operand info in
/// `CatchableErrorKind::AssertionFailed` itself, but `Op::AssertFail`'s
/// span (`compile_assert` in the vendored compiler) is exactly the
/// condition expression's own span — `err.span`/`leaf.span` for this error
/// *is* `1 == 2`'s source range. This extracts `("LHS", "==", "RHS")` (or
/// `!=`) from that raw condition text; `lib.rs` then re-evaluates each side
/// (in the same VFS) to get their printed values, since forcing happened
/// deep inside the VM and isn't available here.
pub(crate) fn assert_condition_parts(statement_text: &str) -> Option<(String, &'static str, String)> {
    // `err.span` for `CatchableErrorKind::AssertionFailed` covers the whole
    // `assert COND; BODY` statement (empirically — not just `COND`), so
    // strip the `assert ` keyword and the top-level `;` that ends the
    // condition ourselves.
    let after_kw = statement_text.trim().strip_prefix("assert")?.trim_start();
    let semi = find_top_level(after_kw, ";")?;
    let inner = after_kw[..semi].trim();
    for op in ["==", "!="] {
        if let Some(idx) = find_top_level(inner, op) {
            let lhs = inner[..idx].trim().to_string();
            let rhs = inner[idx + op.len()..].trim().to_string();
            if lhs.is_empty() || rhs.is_empty() {
                continue;
            }
            return Some((lhs, op, rhs));
        }
    }
    None
}

/// Finds `op` in `s` outside of any `(...)`/`[...]`/`{...}` nesting (so
/// `(a == b) == c` splits on the outer `==`, not the one inside the
/// parens). Good enough for the simple conditions beginner lessons write.
fn find_top_level(s: &str, op: &str) -> Option<usize> {
    let mut depth = 0i32;
    let bytes = s.as_bytes();
    let op_bytes = op.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        if depth == 0 && s[i..].as_bytes().starts_with(op_bytes) {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Formats one side of a failed `==`/`!=` assertion the way CppNix does:
/// `{noun(type)} with value '{printed}'`.
pub(crate) fn assert_operand_text(value: &Value) -> String {
    format!("{} with value '{value}'", noun(value.type_of()))
}

/// `builtins.toJSON`-shaped conversion of an already-fully-forced `Value`
/// into `serde_json::Value`, for `EvalResult.json`. tvix's own `Value` has
/// no `serde::Serialize` impl (its `toJSON` builtin is a VM generator, not
/// reachable outside a running VM — see docs/spikes/A2-snix.md, "What
/// doesn't work"), so this walks the already-forced tree directly instead.
/// Functions and other non-JSON-representable values yield `None`, per
/// `EvalSuccess.json`'s "when representable" contract in
/// lib/runtime/types.ts.
pub(crate) fn to_json(value: &Value) -> Option<serde_json::Value> {
    use serde_json::Value as J;
    match value {
        Value::Null => Some(J::Null),
        Value::Bool(b) => Some(J::Bool(*b)),
        Value::Integer(i) => Some(J::Number((*i).into())),
        Value::Float(f) => serde_json::Number::from_f64(*f).map(J::Number),
        Value::String(s) => Some(J::String(s.to_str_lossy().into_owned())),
        Value::Path(p) => Some(J::String(p.display().to_string())),
        Value::List(l) => {
            let mut out = Vec::with_capacity(l.len());
            for v in l.iter() {
                out.push(to_json(v)?);
            }
            Some(J::Array(out))
        }
        Value::Attrs(a) => {
            let mut map = serde_json::Map::new();
            for (k, v) in a.iter_sorted() {
                let k: &tvix_eval::NixString = k;
                map.insert(k.to_str_lossy().into_owned(), to_json(v)?);
            }
            Some(J::Object(map))
        }
        _ => None,
    }
}
