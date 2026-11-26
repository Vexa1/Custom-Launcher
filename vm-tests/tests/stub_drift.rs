//! Fails when a stand-in in `vm-tests/src/` drifts from the real declaration it restates.
//!
//! The harness compiles the shipping `party.rs` and `server_hop.rs`, but it has to restate the
//! few types and functions they borrow from the GUI crate. A restated shape can go stale and the
//! harness keeps passing, so this reads the real sources with `syn` and compares declarations.

use quote::ToTokens;
use std::fs;
use std::path::PathBuf;

fn parse(relative: &str) -> syn::File {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()));
    syn::parse_file(&text).unwrap_or_else(|e| panic!("{} does not parse: {e}", path.display()))
}

/// Token text without whitespace or `path::` prefixes, so `crate::types::ServerItem` and
/// `ServerItem` compare as the same type.
fn shape<T: ToTokens>(item: &T) -> String {
    let raw: String = item
        .to_token_stream()
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ':' && chars.get(i + 1) == Some(&':') {
            while out.ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                out.pop();
            }
            i += 2;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn field_shapes(fields: &syn::Fields) -> Vec<String> {
    fields
        .iter()
        .map(|f| match &f.ident {
            Some(name) => format!("{name}:{}", shape(&f.ty)),
            None => shape(&f.ty),
        })
        .collect()
}

/// Field shapes sorted, so reordering a declaration is not mistaken for drift.
fn struct_shapes(file: &syn::File, name: &str) -> Vec<String> {
    for item in &file.items {
        if let syn::Item::Struct(s) = item {
            if s.ident == name {
                let mut shapes = field_shapes(&s.fields);
                shapes.sort();
                return shapes;
            }
        }
    }
    panic!("`{name}` is no longer a struct in the real source")
}

fn variant_shape(file: &syn::File, name: &str, variant: &str) -> String {
    for item in &file.items {
        if let syn::Item::Enum(e) = item {
            if e.ident == name {
                let found = e
                    .variants
                    .iter()
                    .find(|v| v.ident == variant)
                    .unwrap_or_else(|| panic!("`{name}::{variant}` is gone from the real enum"));
                let payload = field_shapes(&found.fields);
                return if payload.is_empty() {
                    variant.to_string()
                } else {
                    format!("{variant}({})", payload.join(","))
                };
            }
        }
    }
    panic!("`{name}` is no longer an enum in the real source")
}

/// Parameter types and return type, ignoring parameter names.
fn fn_shape(file: &syn::File, name: &str) -> String {
    for item in &file.items {
        if let syn::Item::Fn(f) = item {
            if f.sig.ident == name && matches!(f.vis, syn::Visibility::Public(_)) {
                let params: Vec<String> = f
                    .sig
                    .inputs
                    .iter()
                    .map(|arg| match arg {
                        syn::FnArg::Typed(t) => shape(&t.ty),
                        other => shape(other),
                    })
                    .collect();
                let returns = match &f.sig.output {
                    syn::ReturnType::Default => "()".to_string(),
                    syn::ReturnType::Type(_, ty) => shape(&*ty),
                };
                return format!("({})->{returns}", params.join(","));
            }
        }
    }
    panic!("`pub fn {name}` is gone from the real source")
}

#[test]
fn stand_ins_match_the_launcher() {
    let structs = [
        ("src/types.rs", "vm-tests/src/types.rs", "ServerItem"),
        ("src/api.rs", "vm-tests/src/api.rs", "PartyPresence"),
    ];
    // The stand-ins `party.rs` and `server_hop.rs` call.
    let functions = [
        ("src/api.rs", "vm-tests/src/api.rs", "party_presence"),
        (
            "src/api.rs",
            "vm-tests/src/api.rs",
            "fetch_public_server_pool",
        ),
        ("src/auth.rs", "vm-tests/src/auth.rs", "data_dir"),
        ("src/auth.rs", "vm-tests/src/auth.rs", "log"),
        (
            "src/roblox_process.rs",
            "vm-tests/src/roblox_process.rs",
            "kill_all_roblox_processes",
        ),
        (
            "src/roblox_process.rs",
            "vm-tests/src/roblox_process.rs",
            "launch_authenticated_server",
        ),
    ];

    let mut drift = Vec::new();
    for (real, stub, name) in structs {
        let (real, stub) = (struct_shapes(&parse(real), name), struct_shapes(&parse(stub), name));
        if real != stub {
            drift.push(format!("struct {name}: real {real:?} / stub {stub:?}"));
        }
    }
    let real = variant_shape(&parse("src/types.rs"), "Msg", "Party");
    let stub = variant_shape(&parse("vm-tests/src/types.rs"), "Msg", "Party");
    if real != stub {
        drift.push(format!("enum Msg: real {real} / stub {stub}"));
    }
    for (real, stub, name) in functions {
        let (real, stub) = (fn_shape(&parse(real), name), fn_shape(&parse(stub), name));
        if real != stub {
            drift.push(format!("fn {name}: real {real} / stub {stub}"));
        }
    }

    assert!(
        drift.is_empty(),
        "\nA stand-in no longer matches the launcher - update vm-tests/src/:\n  {}\n",
        drift.join("\n  ")
    );
}
