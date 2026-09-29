//! Path/name matching helpers used across the analysis.
//!
//! These are the tool's plumbing — the LOGIC that matches a call's printed path
//! against the catalogs in [`crate::settings`], derives a finding's attack class, and
//! ranks taint origins

use crate::settings::{SINK_FUNCTION_ARGS, SOURCE_FUNCTIONS};

/// The bug label shown for a finding, worked out from its (sink, forbidden value).
pub fn attack_class(sink: &str, value: &str) -> &'static str {
    let stripped = strip_generics(sink);
    for (p, _, v, attack) in SINK_FUNCTION_ARGS {
        let func_match = seg_match(sink, p) || seg_match(&stripped, p);
        if func_match && *v == value {
            return attack;
        }
    }
    // env vars are also registered dynamically (set_var with any tracked name).
    if sink.ends_with("env::set_var") {
        return "env-tampering";
    }
    "unknown"
}

/// True if `path` is one of the dangerous SINK functions. Used to focus the
/// analysis on functions that could actually reach a sink.
pub fn is_sink_func(path: &str) -> bool {
    let stripped = strip_generics(path);
    SINK_FUNCTION_ARGS
        .iter()
        .any(|(p, _, _, _)| seg_match(path, p) || seg_match(&stripped, p))
}

/// True if `path` is one of the untrusted-input SOURCE functions. Used to focus the
/// return-origin summary pass on functions that actually READ something.
pub fn is_source_func(path: &str) -> bool {
    let stripped = strip_generics(path);
    SOURCE_FUNCTIONS
        .iter()
        .any(|p| seg_match(path, p) || seg_match(&stripped, p))
}

/// Drop the `<...>` generic bits from a full path so a short catalog name like
/// `ZipFile::name` matches the real printed `zip::read::ZipFile::<R>::name`.
pub fn strip_generics(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut depth: i32 = 0;
    for c in path.chars() {
        match c {
            '<' => depth += 1,
            '>' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    while out.contains("::::") {
        out = out.replace("::::", "::");
    }
    out
}

/// Match a full path against a short catalog name on `::` boundaries: exact, or the
/// name is a whole-segment suffix/prefix. So `Entry::path` matches `tar::Entry::path`
/// but NOT `walkdir::DirEntry::path`, and `std::fs::read` does NOT match `read_dir`.
pub fn seg_match(path: &str, key: &str) -> bool {
    path == key
        || path.ends_with(&format!("::{key}"))
        || path.starts_with(&format!("{key}::"))
}

/// Is this source a real archive/file/stream READ (as opposed to a command-line /
/// env value)? Used by the READER_GATE switch: a public function that never actually
/// reads anything is a plain path helper, not an unpacker, so its param findings are
/// dropped as false alarms.
pub fn is_reader_provenance_source(path: &str) -> bool {
    !(path.contains("env::") || path.contains("ArgMatches"))
}

/// Is this a BASE-position sink — one whose checked argument is used purely as a base
/// / output DIRECTORY rather than the whole write target? `create_dir(base)`,
/// `create_dir_all(base)`, and `copy(from, to)` (dest) fall here: a bare caller-chosen
/// directory passed to one of these is the intended destination, not an attacker entry.
/// (Contrast `write`/`File::create`/`remove_*`/`open`, where the argument IS the target
/// and a caller-supplied `..` path is a real traversal we keep flagging.) Covers the
/// `std::fs` and `tokio::fs` variants. See [`crate::parser`]'s entry-vs-base gate.
pub fn is_base_position_sink(sink_func: &str) -> bool {
    let stripped = strip_generics(sink_func);
    ["fs::create_dir", "fs::create_dir_all", "fs::copy"]
        .iter()
        .any(|k| seg_match(sink_func, k) || seg_match(&stripped, k))
}

/// Does the START of a taint path name a REAL source (from SOURCE_FUNCTIONS or a
/// "reads bytes" read), rather than a boring middle step like `into_vec`/`Path::join`
/// or a guessed `param[...]`? Used to credit a finding to the true source, not to an
/// intermediate call that merely carried the value.
pub fn origin_is_real_source(head: &str) -> bool {
    let h = head.split(" @ ").next().unwrap_or(head).trim();
    // reader-carrier flow-source label ("reads bytes @ <callee>").
    if h.starts_with("reads bytes") {
        return true;
    }
    let stripped = strip_generics(h);
    SOURCE_FUNCTIONS
        .iter()
        .any(|s| seg_match(h, s) || seg_match(&stripped, s))
}
