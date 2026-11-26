//! Fails when a stand-in in `vm-tests/src/` drifts away from the real declaration.
//!
//! The harness compiles the shipping `party.rs` and `server_hop.rs`, but it has to restate the
//! few types and functions those files borrow from the GUI crate. A restated type can go stale
//! and the harness keeps passing, silently testing a shape the launcher no longer has. Each test
//! below reads the real source and compares its fields, variant payload and function signatures
//! with the stand-in, so the mismatch fails here instead of passing quietly.

use std::fs;
use std::path::PathBuf;

fn repo_file(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("could not read {}: {e}", path.display()))
}

/// Collapses whitespace and drops every `path::` prefix, so `crate::types::ServerItem` and
/// `ServerItem` - or `std::path::PathBuf` and `PathBuf` - compare as the same type.
fn normalize(text: &str) -> String {
    let mut compact = String::new();
    for ch in text.chars() {
        if !ch.is_whitespace() {
            compact.push(ch);
        }
    }
    let chars: Vec<char> = compact.chars().collect();
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

/// The text between the braces of `header`, brace-balanced.
fn body_of(src: &str, header: &str) -> String {
    let start = src
        .find(header)
        .unwrap_or_else(|| panic!("`{header}` is not in the source any more"));
    let rest = &src[start + header.len()..];
    let next = rest
        .chars()
        .find(|c| !c.is_whitespace())
        .unwrap_or_else(|| panic!("`{header}` ends the file"));
    assert!(
        next == '{' || next == '<' || next == '(',
        "`{header}` matched a longer name in the source"
    );
    let open = start + src[start..].find('{').expect("header has no body") as usize;
    let mut depth = 0usize;
    for (offset, ch) in src[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return src[open + 1..open + offset].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced braces after `{header}`")
}

/// Splits on `separator` at the top level only, ignoring separators inside `<>`, `()`, `[]`,
/// `{}` and string literals.
fn split_top_level(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut in_string = false;
    for ch in text.chars() {
        if in_string {
            current.push(ch);
            if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                current.push(ch);
            }
            '(' | '[' | '{' | '<' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' | '}' | '>' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts
}

/// Strips `#[...]` attributes and `//` comments so only declarations remain.
fn strip_noise(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("#[") || trimmed.starts_with("//") || trimmed.starts_with("#!") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// `field: type` for every declared field, sorted so a reordering is not a failure.
fn struct_fields(src: &str, name: &str) -> Vec<String> {
    let body = strip_noise(&body_of(src, &format!("pub struct {name}")));
    let mut fields = Vec::new();
    for part in split_top_level(&body) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((field, ty)) = part.split_once(':') else {
            panic!("{name} has a field without a type: {part:?}");
        };
        let field = field.trim().trim_start_matches("pub ").trim();
        fields.push(format!("{}:{}", field, normalize(ty)));
    }
    fields.sort();
    fields
}

/// A variant's payload, e.g. `Party(PartyEvent)`, normalized for comparison.
fn enum_variant(src: &str, name: &str, variant: &str) -> String {
    let body = strip_noise(&body_of(src, &format!("pub enum {name}")));
    let marker = format!("{variant}(");
    let start = body
        .find(&marker)
        .unwrap_or_else(|| panic!("`{name}::{variant}` is gone from the real enum"));
    let mut depth = 0usize;
    for (offset, ch) in body[start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return normalize(&body[start..start + offset + 1]);
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced payload for `{name}::{variant}`")
}

/// Parameter types and return type of a `pub fn`, ignoring parameter names.
fn fn_shape(src: &str, name: &str) -> String {
    let header = format!("pub fn {name}(");
    let start = src
        .find(&header)
        .unwrap_or_else(|| panic!("`{header}` is gone from the real source"));
    let open = start + header.len() - 1;
    let mut depth = 0usize;
    let mut close = None;
    for (offset, ch) in src[open..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + offset);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close.expect("unbalanced parameter list");
    let params = split_top_level(&src[open + 1..close])
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .map(|part| match part.split_once(':') {
            Some((_, ty)) => normalize(ty),
            None => normalize(&part),
        })
        .collect::<Vec<_>>()
        .join(",");
    let after = &src[close + 1..];
    let returns = match after.find("->") {
        Some(at) => {
            let tail = &after[at + 2..];
            let end = tail.find(&['{', ';'][..]).unwrap_or(tail.len());
            normalize(&tail[..end])
        }
        None => "()".to_string(),
    };
    format!("({params})->{returns}")
}

#[test]
fn server_item_matches_the_launcher() {
    let real = struct_fields(&repo_file("src/types.rs"), "ServerItem");
    let stub = struct_fields(&repo_file("vm-tests/src/types.rs"), "ServerItem");
    assert_eq!(
        stub, real,
        "\nvm-tests/src/types.rs restates ServerItem and it no longer matches src/types.rs.\n\
         Update the stand-in (and anything that builds it) to the real fields.\n"
    );
}

#[test]
fn party_presence_matches_the_launcher() {
    let real = struct_fields(&repo_file("src/api.rs"), "PartyPresence");
    let stub = struct_fields(&repo_file("vm-tests/src/api.rs"), "PartyPresence");
    assert_eq!(
        stub, real,
        "\nvm-tests/src/api.rs restates PartyPresence and it no longer matches src/api.rs.\n"
    );
}

#[test]
fn msg_carries_the_party_event_variant() {
    let real = enum_variant(&repo_file("src/types.rs"), "Msg", "Party");
    let stub = enum_variant(&repo_file("vm-tests/src/types.rs"), "Msg", "Party");
    assert_eq!(
        stub, real,
        "\nvm-tests/src/types.rs restates Msg::Party and its payload changed in src/types.rs.\n"
    );
}

#[test]
fn stubbed_function_signatures_match() {
    // (real file, stand-in file, function) - the stand-ins party.rs and server_hop.rs call.
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
    for (real_file, stub_file, name) in functions {
        let real = fn_shape(&repo_file(real_file), name);
        let stub = fn_shape(&repo_file(stub_file), name);
        if real != stub {
            drift.push(format!("{name}: real {real} / stub {stub}"));
        }
    }
    assert!(
        drift.is_empty(),
        "\nA stand-in function no longer matches the launcher:\n  {}\n",
        drift.join("\n  ")
    );
}
