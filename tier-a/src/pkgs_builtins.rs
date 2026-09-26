//! Native `derivationStrict` builtin, implemented on `nix-compat`'s
//! derivation/ATerm/store-path code (see nixbox/README.md — `tvix-glue`,
//! upstream's real implementation, needs `tokio` and doesn't target wasm;
//! this reimplements just enough of it). `derivation` itself (the
//! `outputs`-fanout wrapper user code actually calls) is the standard
//! Nix-source `corepkgs/derivation.nix`, registered in `lib.rs` via
//! `add_src_builtin` — this builtin is `derivationStrict` only.
//!
//! No real store or build: this computes exactly the store paths real Nix
//! would (verified byte-for-byte against `nix-instantiate` — see
//! `spikes/parity/snippets/33-derivation.nix`), but nothing is written
//! anywhere and `input_derivations` (paths of other derivations referenced
//! as build inputs) is always empty. That's a correctness gap only for
//! derivations that reference *other derivations'* `drvPath`/outputs in
//! their environment (their ATerm hash — and so their own output paths —
//! would differ from a real Nix build with a populated store); the teaching
//! `pkgs` stub (`nixbox/pkgs/default.nix`) is built to avoid that by not
//! chaining derivations. See nixbox/README.md.

use std::collections::BTreeMap;

use bstr::{BString, ByteSlice};
use nix_compat::derivation::{CAHash, Derivation, Output};
use nix_compat::nixhash::NixHash;
use tvix_eval::builtin_macros::builtins;
use tvix_eval::generators::{self, GenCo};
use tvix_eval::{CoercionKind, ErrorKind, NixAttrs, NixString, Value};

fn internal_error(msg: impl std::fmt::Display) -> ErrorKind {
    // There's no dedicated "derivation builder" ErrorKind in tvix-eval;
    // TvixBug is the closest existing internal-error variant. These paths
    // (bad hash encoding, invalid derivation name, …) aren't expected to
    // be hit by NixQuest's own `pkgs` stub; a lesson snippet that manages
    // to hit one gets `errorClass: "internal"` and this text.
    ErrorKind::TvixBug {
        msg: "derivationStrict failed",
        metadata: Some(std::rc::Rc::new(msg.to_string())),
    }
}

async fn coerce_string(co: &GenCo, v: Value) -> Result<Result<String, Value>, ErrorKind> {
    let forced = generators::request_force(co, v).await;
    if forced.is_catchable() {
        return Ok(Err(forced));
    }
    match generators::request_string_coerce(
        co,
        forced,
        CoercionKind {
            strong: true,
            import_paths: false,
        },
    )
    .await
    {
        Ok(s) => Ok(Ok(s.to_str_lossy().into_owned())),
        Err(cek) => Ok(Err(Value::Catchable(Box::new(cek)))),
    }
}

async fn string_list(co: &GenCo, v: Value) -> Result<Result<Vec<String>, Value>, ErrorKind> {
    let forced = generators::request_force(co, v).await;
    if forced.is_catchable() {
        return Ok(Err(forced));
    }
    let list = forced.to_list()?;
    let mut out = Vec::with_capacity(list.len());
    for item in list.into_iter() {
        match Box::pin(coerce_string(co, item)).await? {
            Ok(s) => out.push(s),
            Err(catchable) => return Ok(Err(catchable)),
        }
    }
    Ok(Ok(out))
}

#[builtins]
mod derivation_builtins {
    use genawaiter::rc::Gen;

    use super::*;

    /// `derivationStrict :: AttrSet -> AttrSet`
    ///
    /// Given the raw attrset passed to `derivation { ... }` (after
    /// `corepkgs/derivation.nix` has defaulted `outputs`), computes the
    /// real `.drv` path and every output's store path, the same way real
    /// Nix's `EvalState::derivationStrict` (primops.cc) does: build a
    /// `nix-compat::derivation::Derivation`, serialize it to ATerm, hash
    /// it (`hashDerivationModulo`), and derive each output's store path
    /// from that hash (or, for a fixed-output derivation, from its
    /// `outputHash` directly). Returns `{ drvPath; <output> = <path>; }`.
    #[builtin("derivationStrict")]
    async fn builtin_derivation_strict(co: GenCo, attrs: Value) -> Result<Value, ErrorKind> {
        let attrs = attrs.to_attrs()?;

        let name = match attrs.select_str("name") {
            Some(v) => match coerce_string(&co, v.clone()).await? {
                Ok(s) => s,
                Err(c) => return Ok(c),
            },
            None => return Err(internal_error("derivation requires a 'name' attribute")),
        };

        let system = match attrs.select_str("system") {
            Some(v) => match coerce_string(&co, v.clone()).await? {
                Ok(s) => s,
                Err(c) => return Ok(c),
            },
            None => String::new(),
        };

        let builder = match attrs.select_str("builder") {
            Some(v) => match coerce_string(&co, v.clone()).await? {
                Ok(s) => s,
                Err(c) => return Ok(c),
            },
            None => String::new(),
        };

        let arguments = match attrs.select_str("args") {
            Some(v) => match string_list(&co, v.clone()).await? {
                Ok(a) => a,
                Err(c) => return Ok(c),
            },
            None => Vec::new(),
        };

        let outputs: Vec<String> = match attrs.select_str("outputs") {
            Some(v) => match string_list(&co, v.clone()).await? {
                Ok(o) if !o.is_empty() => o,
                Ok(_) => vec!["out".to_string()],
                Err(c) => return Ok(c),
            },
            None => vec!["out".to_string()],
        };

        // Fixed-output derivation? `outputHash`/`outputHashAlgo` (and
        // optional `outputHashMode`, default "flat"), same as real Nix.
        // Only valid for a single "out" output, same as upstream.
        let fixed_output_hash: Option<CAHash> = match attrs.select_str("outputHash") {
            Some(hash_val) => {
                let hash_str = match coerce_string(&co, hash_val.clone()).await? {
                    Ok(s) => s,
                    Err(c) => return Ok(c),
                };
                let algo_str = match attrs.select_str("outputHashAlgo") {
                    Some(v) => match coerce_string(&co, v.clone()).await? {
                        Ok(s) => Some(s),
                        Err(c) => return Ok(c),
                    },
                    None => None,
                };
                let mode = match attrs.select_str("outputHashMode") {
                    Some(v) => match coerce_string(&co, v.clone()).await? {
                        Ok(s) => s,
                        Err(c) => return Ok(c),
                    },
                    None => "flat".to_string(),
                };
                let hash = parse_output_hash(&hash_str, algo_str.as_deref())
                    .map_err(internal_error)?;
                Some(if mode == "recursive" || mode == "r:recursive" {
                    CAHash::Nar(hash)
                } else {
                    CAHash::Flat(hash)
                })
            }
            None => None,
        };

        // Environment: every attribute of the original set, string-coerced
        // (real Nix's own `coerceMore = true` string coercion — lists
        // become space-joined, ints/bools/floats stringified, nested
        // derivations resolve via their `outPath`), *except* `args`
        // (dedicated field, never duplicated into env — verified against
        // a real `.drv` file, see nixbox/README.md).
        let mut environment: BTreeMap<String, BString> = BTreeMap::new();
        for (k, v) in attrs.iter_sorted() {
            let key = k.to_str_lossy().into_owned();
            if key == "args" {
                continue;
            }
            match coerce_string(&co, v.clone()).await? {
                Ok(s) => {
                    environment.insert(key, BString::from(s));
                }
                Err(c) => return Ok(c),
            }
        }
        for out in &outputs {
            environment.entry(out.clone()).or_default();
        }

        let mut outputs_map: BTreeMap<String, Output> = BTreeMap::new();
        for out in &outputs {
            let ca_hash = if out == "out" && outputs.len() == 1 {
                fixed_output_hash.clone()
            } else {
                None
            };
            outputs_map.insert(
                out.clone(),
                Output {
                    path: None,
                    ca_hash,
                },
            );
        }

        let mut drv = Derivation {
            arguments,
            builder,
            environment,
            input_derivations: Default::default(),
            input_sources: Default::default(),
            outputs: outputs_map,
            system,
        };

        // No input derivations are tracked (see module doc), so the
        // lookup closure is never actually called.
        let hash_modulo = drv.hash_derivation_modulo(|_| [0u8; 32]);
        drv.calculate_output_paths(&name, &hash_modulo)
            .map_err(internal_error)?;
        let drv_path = drv
            .calculate_derivation_path(&name)
            .map_err(internal_error)?;

        let mut result: Vec<(NixString, Value)> = vec![(
            NixString::from("drvPath"),
            Value::String(NixString::from(drv_path.to_absolute_path())),
        )];
        for (out_name, output) in &drv.outputs {
            let path = output
                .path
                .as_ref()
                .expect("calculate_output_paths populates every output's path");
            result.push((
                NixString::from(out_name.as_str()),
                Value::String(NixString::from(path.to_absolute_path())),
            ));
        }

        Ok(Value::attrs(NixAttrs::from_iter(result)))
    }
}

fn parse_output_hash(hash: &str, algo: Option<&str>) -> Result<NixHash, String> {
    // Accepts the plain nixbase32/hex/SRI form via nix-compat's own
    // parser when a `:`-prefixed algo is embedded (SRI-ish), otherwise
    // requires `algo` and tries hex, then nixbase32.
    if let Some(algo) = algo {
        if let Some(h) = NixHash::from_nix_hex_str(&format!("{algo}:{hash}")) {
            return Ok(h);
        }
        if let Ok(bytes) = data_encoding::HEXLOWER_PERMISSIVE.decode(hash.as_bytes()) {
            return bytes_to_hash(algo, &bytes);
        }
        return Err(format!("invalid hash '{hash}' for algo '{algo}'"));
    }
    NixHash::from_nix_hex_str(hash).ok_or_else(|| format!("invalid hash '{hash}' (no outputHashAlgo given)"))
}

fn bytes_to_hash(algo: &str, bytes: &[u8]) -> Result<NixHash, String> {
    match algo {
        "md5" => Ok(NixHash::Md5(bytes.try_into().map_err(|_| "bad md5 length")?)),
        "sha1" => Ok(NixHash::Sha1(bytes.try_into().map_err(|_| "bad sha1 length")?)),
        "sha256" => Ok(NixHash::Sha256(bytes.try_into().map_err(|_| "bad sha256 length")?)),
        "sha512" => Ok(NixHash::Sha512(Box::new(
            bytes.try_into().map_err(|_| "bad sha512 length")?,
        ))),
        other => Err(format!("unknown hash algo '{other}'")),
    }
}

pub(crate) fn builtins() -> Vec<(&'static str, Value)> {
    derivation_builtins::builtins()
}
