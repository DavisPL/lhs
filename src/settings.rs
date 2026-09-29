// CONFIGURATION, the settings that control how the taint tracker behaves.

pub const MAX_LOOP_ITER: u32 = 5; // number of iterations before widening

// When ON, every PUBLIC function's parameters are treated as untrusted input so a `pub param -> sink` flow is reported
// even if the function never reads a file. 
// Turn OFF to analyze a BINARY, where params are internal, not an attack surface. 
pub const PUBLIC_PARAMS_ENABLED: bool = true;


// LISTS, the actual names the tool recognizes. Add/remove a line to teach it.

// LIST: file/reader HANDLE types, used by the always-on handle-read rule
// (`is_handle_carrier_ty` in parser.rs). Passing one of these INTO a function that
// returns data means the function READS from it, so its result is untrusted.
pub const HANDLE_CARRIER_TYPES: &[&str] = &["fs::File", "io::BufReader"];

// LIST: boring "pass-through" calls to HIDE from the reported flow path, so the
// "how the data flowed" chain shows only meaningful steps, this is cosmetic only, 
// this does not impact the analysis just the report display. 
pub const CHAIN_SKIP_FNS: &[&str] = &[
    // "deref", "deref_mut", "clone", "as_ref", "as_str", "as_path", "borrow", "borrow_mut",
    // "to_owned", "into", "unwrap", "unwrap_or_default", "to_string",
];

// LIST: environment-variable names we protect, flag it if untrusted data tries to
// overwrite one of these 
pub const ENV_VARS_TO_TRACK: &[&str] = &["RUSTC", "CARGO"];

// SOURCES, the functions that introduce untrusted input. 

pub const SOURCE_FUNCTIONS: &[&str] = &[
    // Command line / environment: values the user typed or the environment gave.
    "std::env::args",
    "std::env::args_os",
    "std::env::var",
    "std::env::var_os",
    "std::env::vars",
    "std::env::vars_os",
    // Reading from any stream/reader (a file, a network socket, TLS, stdin, …): the
    // bytes read are untrusted. `read`/`read_to_end`/`read_to_string` fill a BUFFER
    // argument (tainted via `handle_read_into_buf`); the others return the data.
    "std::io::Read::read",
    "std::io::Read::read_to_string",
    "std::io::Read::read_to_end",
    "std::io::BufRead::read_line",
    "std::io::BufRead::read_until",
    "std::io::BufRead::lines",
    "std::io::BufRead::split",
    "std::io::Stdin::read_line",
    "std::io::Stdin::lines",
    // Reading a whole file's contents.
    "std::fs::read",
    "std::fs::read_to_string",
    // Memory-mapped file contents: `Mmap::as_slice()` hands back the raw bytes of a
    // mapped file — just as untrusted as `fs::read`. (Opening the map is not the
    // read; THIS call is.) This is what lets an mmap-based unpacker be traced to a
    // real file-read source.
    "Mmap::as_slice",
    "MmapMut::as_slice",
    // The async (tokio) versions of reading a file — untrusted for the same reason.
    "tokio::fs::read",
    "tokio::fs::read_to_string",
    // Command-line argument values parsed by the `clap` library — exactly what the
    // user typed, so untrusted if it flows into a file/command action.
    "ArgMatches::value_of",
    "ArgMatches::value_of_lossy",
    "ArgMatches::value_of_os",
    "ArgMatches::values_of",
    "ArgMatches::values_of_lossy",
    "ArgMatches::values_of_os",
    "ArgMatches::get_one",
    "ArgMatches::get_many",
    "ArgMatches::get_raw",
    // Archive entry NAMES read straight from an archive — the classic zip/tar-slip
    // source. These accessors return the attacker-controlled name of a file inside
    // the archive, which is exactly what must not be trusted when writing to disk.
    "ZipFile::name",
    "ZipFile::name_raw",
    "ZipEntry::filename",
    "Entry::path",
    "Entry::path_bytes",
    "Header::path",
    "Header::path_bytes",
    "SevenZArchiveEntry::name",
    // Same idea for the `qpak` archive format: `table_entry().path()` is the raw
    // entry name decoded from the archive.
    "TableEntry::path",
]; 
// note: reading from a file, reading raw bytes, reading from a network socket, reading from stdin, etc. are all sources.

// SANITIZERS, functions that make a value SAFE again.

// function names should be written in full, crate::module::Type::method, so the 
// catalog does not match a function with smae name in two different crates or modules. 

pub const SANITIZERS: &[&str] = &[
    "ZipFile::enclosed_name",
    "ZipFile::mangled_name",
    "ZipEntry::enclosed_name",

    "minipacked::archive::validate_output_path",
];

// SINKS, where an untrusted value should not go.

// Each row is: (function name, which argument to check, forbidden pattern, bug type).
//   • "which argument"  = 0 is the first argument, 1 is the second, …
//   • forbidden pattern = what the value must NOT be able to be. `*` is a wildcard,
//                         so `*..*` means "the path could contain `..`" (traversal),
//                         and `rm -rf *` means "starts with rm -rf".
//   • bug type          = the label shown in the report (e.g. path-traversal).
// A finding fires when a TAINTED value reaches that argument AND Z3 agrees it could
// match the pattern. 

// We can add nad remove rows to add/remove sinks. 

// Hassnain: Current list is not exhaustive, I wonder if there is an automatic way of doing it for all possible sinks?
// anything that goes to a side effect (file write, command execution, env var set) is a sink?


pub const SINK_FUNCTION_ARGS: &[(&str, usize, &str, &str)] = &[
    // Tampering with a protected environment variable, or running a shell wipe.
    ("std::env::set_var", 0, "RUSTC", "env-tampering"),
    ("std::process::Command::new", 0, "rm -rf *", "command-injection"),
    ("std::process::Command::new" , 0, "*", "command-injection"),
    // Writing/creating/linking/deleting a file at a path that could contain `..`
    // (the zip/tar-slip bug): an attacker-named path escaping the target folder.
    ("std::fs::write", 0, "*..*", "path-traversal"),
    ("std::fs::write", 0, "/proc/self/mem", "path-traversal"),
    ("std::fs::File::create", 0, "*..*", "path-traversal"),
    ("std::fs::File::create_new", 0, "*..*", "path-traversal"),
    ("std::fs::OpenOptions::open", 1, "*..*", "path-traversal"),
    ("std::fs::create_dir", 0, "*..*", "path-traversal"),
    ("std::fs::create_dir_all", 0, "*..*", "path-traversal"),
    ("std::fs::copy", 1, "*..*", "path-traversal"),
    ("std::fs::rename", 1, "*..*", "path-traversal"),
    ("std::fs::hard_link", 1, "*..*", "path-traversal"),
    ("std::fs::remove_file", 0, "*..*", "path-traversal"),
    ("std::fs::remove_dir_all", 0, "*..*", "path-traversal"),
    ("std::os::unix::fs::symlink", 1, "*..*", "path-traversal"),
    ("std::os::windows::fs::symlink_file", 1, "*..*", "path-traversal"),
    // The async (tokio) versions of the same file actions — same danger.
    ("tokio::fs::write", 0, "*..*", "path-traversal"),
    ("tokio::fs::File::create", 0, "*..*", "path-traversal"),
    ("tokio::fs::File::create_new", 0, "*..*", "path-traversal"),
    ("tokio::fs::OpenOptions::open", 1, "*..*", "path-traversal"),
    ("tokio::fs::create_dir", 0, "*..*", "path-traversal"),
    ("tokio::fs::create_dir_all", 0, "*..*", "path-traversal"),
    ("tokio::fs::copy", 1, "*..*", "path-traversal"),
    ("tokio::fs::rename", 1, "*..*", "path-traversal"),
    ("tokio::fs::hard_link", 1, "*..*", "path-traversal"),
    ("tokio::fs::remove_file", 0, "*..*", "path-traversal"),
    ("tokio::fs::remove_dir_all", 0, "*..*", "path-traversal"),
    ("tokio::fs::symlink", 1, "*..*", "path-traversal"),
    ("tokio::fs::symlink_file", 1, "*..*", "path-traversal"),
];

// NOTE: the matching/logic helpers that consume these catalogs (attack_class,
// is_sink_func, is_source_func, is_base_position_sink, strip_generics, seg_match,
// is_reader_provenance_source, origin_is_real_source) live in `src/matching.rs`
