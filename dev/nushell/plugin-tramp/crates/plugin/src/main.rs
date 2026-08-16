//! `nu-plugin-tramp` — A TRAMP-inspired remote filesystem plugin for Nushell.
//!
//! This is the plugin entry point.  It registers the following commands:
//!
//! - `tramp open <path>`        — read a remote file and return it as a Nushell value
//! - `tramp ls <path>`          — list a remote directory as a Nushell table
//! - `tramp save <path>`        — write piped data to a remote file
//! - `tramp rm <path>`          — delete a remote file
//! - `tramp cp <src> <dst>`     — copy files between local/remote locations
//! - `tramp cd <path>`          — set the remote working directory
//! - `tramp exec <path> <cmd>`  — execute a command on the remote (push execution)
//! - `tramp info <path>`        — show remote system info (OS, arch, hostname, disk)
//! - `tramp watch <path>`       — watch a remote path for filesystem changes (requires RPC agent)
//! - `tramp shell <path>`       — interactive remote terminal session (requires RPC agent)
//! - `tramp ping <path>`        — test connectivity to a remote host
//! - `tramp connections`        — list active connections
//! - `tramp disconnect [host]`  — close connections
//!
//! Supported backends: **ssh**, **docker**, **k8s** (kubernetes), **sudo**.
//!
//! Paths use the TRAMP URI format: `/<backend>:<user>@<host>#<port>:<remote-path>`
//!
//! Chained paths enable reaching nested environments:
//!
//! ```text
//! /ssh:myvm|docker:container:/app/config.toml
//! /ssh:myvm|sudo:root:/etc/shadow
//! ```

mod backend;
mod completion;
mod errors;
mod protocol;
mod vfs;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nu_plugin::{
    DynamicCompletionCall, EngineInterface, EvaluatedCall, MsgPackSerializer, Plugin,
    PluginCommand, serve_plugin,
};
use nu_protocol::{
    ByteStreamType, Category, DynamicSuggestion, Example, LabeledError, PipelineData, Record,
    Signature, Span, SyntaxShape, Type, Value, byte_stream::ByteStream, engine::ArgType,
};
use std::time::Duration;

use completion::{complete_tramp_path, extract_positional_string, positional_span};

use errors::TrampError;
use protocol::TrampPath;
use vfs::Vfs;

// ---------------------------------------------------------------------------
// Plugin struct
// ---------------------------------------------------------------------------

/// The main plugin object.  Holds a shared [`Vfs`] instance whose connection
/// pool is reused across all command invocations for the lifetime of the
/// plugin process.
struct TrampPlugin {
    vfs: Arc<Vfs>,
    /// The current remote working directory, set by `tramp cd`.
    /// When set, relative paths passed to tramp commands are resolved
    /// against this URI.
    remote_cwd: Arc<Mutex<Option<TrampPath>>>,
}

impl TrampPlugin {
    fn new() -> Self {
        Self {
            vfs: Arc::new(Vfs::new().expect("failed to initialise tramp VFS")),
            remote_cwd: Arc::new(Mutex::new(None)),
        }
    }
}

impl Plugin for TrampPlugin {
    fn version(&self) -> String {
        env!("CARGO_PKG_VERSION").into()
    }

    fn commands(&self) -> Vec<Box<dyn PluginCommand<Plugin = Self>>> {
        vec![
            Box::new(TrampMain),
            Box::new(TrampOpen),
            Box::new(TrampLs),
            Box::new(TrampSave),
            Box::new(TrampRm),
            Box::new(TrampCp),
            Box::new(TrampCd),
            Box::new(TrampPwd),
            Box::new(TrampExec),
            Box::new(TrampInfo),
            Box::new(TrampPing),
            Box::new(TrampConnections),
            Box::new(TrampDisconnect),
            Box::new(TrampWatch),
            #[cfg(unix)]
            Box::new(TrampShell),
        ]
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve a raw string path to a [`TrampPath`].
///
/// If the string is already a full TRAMP URI it is parsed directly.
/// If it is a relative (or absolute-but-not-TRAMP) path **and** the plugin
/// has a remote CWD set via `tramp cd`, the path is resolved against
/// that CWD.  Otherwise an error is returned explaining the expected format.
fn resolve_tramp_path(
    raw: &str,
    remote_cwd: &Mutex<Option<TrampPath>>,
    span: Span,
) -> Result<TrampPath, LabeledError> {
    // Try parsing as a full TRAMP URI first.
    match protocol::parse(raw) {
        Ok(Some(path)) => return Ok(path),
        Ok(None) => {} // Not a TRAMP URI — try relative resolution below.
        Err(e) => {
            return Err(LabeledError::new(e.to_string()).with_label("parse error", span));
        }
    }

    // Attempt relative resolution against the current remote CWD.
    let cwd_guard = remote_cwd
        .lock()
        .map_err(|e| LabeledError::new(format!("internal lock error: {e}")))?;

    if let Some(ref cwd) = *cwd_guard {
        let resolved = resolve_relative(&cwd.remote_path, raw);
        let mut new_path = cwd.clone();
        new_path.remote_path = resolved;
        Ok(new_path)
    } else {
        Err(LabeledError::new(
            "not a tramp path — expected format: /ssh:host:/remote/path\n\
             Hint: set a remote working directory with `tramp cd /ssh:host:/dir` \
             to use relative paths.",
        )
        .with_label("this is not a TRAMP URI", span))
    }
}

/// Parse a positional string argument at index 0 as a TRAMP path, with
/// relative path resolution support.
fn parse_tramp_arg(
    call: &EvaluatedCall,
    remote_cwd: &Mutex<Option<TrampPath>>,
) -> Result<TrampPath, LabeledError> {
    let raw: String = call.req(0)?;
    resolve_tramp_path(&raw, remote_cwd, call.head)
}

/// Resolve a relative path against a base directory.
///
/// - Absolute paths (starting with `/`) are returned unchanged.
/// - `..` and `.` components are handled.
/// - The result is always a clean absolute path.
fn resolve_relative(base: &str, relative: &str) -> String {
    if relative.starts_with('/') {
        return normalize_path(relative);
    }

    // Start with the base directory's components.
    let base_trimmed = base.trim_end_matches('/');
    let mut components: Vec<&str> = base_trimmed.split('/').filter(|c| !c.is_empty()).collect();

    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            other => {
                components.push(other);
            }
        }
    }

    if components.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", components.join("/"))
    }
}

/// Normalize an absolute path by resolving `.` and `..` components.
fn normalize_path(path: &str) -> String {
    let mut components: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            other => {
                components.push(other);
            }
        }
    }
    if components.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", components.join("/"))
    }
}

/// Try to auto-detect the file format from the extension and parse the bytes
/// into a structured Nushell [`Value`].  Falls back to a plain string (if
/// valid UTF-8) or binary.
fn bytes_to_value(data: &[u8], remote_path: &str, span: Span) -> Value {
    let ext = std::path::Path::new(remote_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    // If the file is a known structured format, try to parse it via the raw
    // text so the user gets a rich Value.  For Phase 1 we only handle JSON
    // and TOML — extending this is trivial.
    if let Ok(text) = std::str::from_utf8(data) {
        match ext {
            "json" => {
                if let Ok(val) = serde_like_json(text, span) {
                    return val;
                }
            }
            "toml" => {
                // Return as string — let the user pipe through `from toml`
            }
            _ => {}
        }
        // Return as string for any valid UTF-8 content
        return Value::string(text, span);
    }

    // Binary fallback
    Value::binary(data, span)
}

/// Minimal JSON → Nushell Value conversion using nushell's own JSON support.
///
/// We do a very thin parse here: the nushell `from json` command is richer,
/// but having *some* auto-detection makes `tramp open` immediately useful.
fn serde_like_json(text: &str, span: Span) -> Result<Value, ()> {
    // Try to parse as a JSON value using a simple recursive descent.
    // For Phase 1, just return the raw string — the user can pipe through
    // `from json`.  A future version could use serde_json.
    let trimmed = text.trim();
    if (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
    {
        // Looks like JSON — return as string so the user can `from json`
        // This is better than returning binary.
        return Ok(Value::string(text, span));
    }
    Err(())
}

/// Convert a `TrampError` into a `LabeledError` with the call's span.
fn tramp_err(e: TrampError, span: Span) -> LabeledError {
    LabeledError::new(e.to_string()).with_label("tramp error", span)
}

// ---------------------------------------------------------------------------
// Dynamic completion helper
// ---------------------------------------------------------------------------

/// Shared implementation of `get_dynamic_completion` for any command whose
/// positional argument at `idx` is a TRAMP path.
///
/// Extracts the partial string from the AST, calls into the completion module,
/// and returns the suggestions (or `None` to fall back to defaults).
#[allow(deprecated)]
fn complete_tramp_arg(
    plugin: &TrampPlugin,
    call: DynamicCompletionCall,
    arg_type: ArgType,
    idx: usize,
) -> Option<Vec<DynamicSuggestion>> {
    // Only handle the positional argument we care about.
    match &arg_type {
        ArgType::Positional(i) if *i == idx => {}
        _ => return None,
    }

    let partial = extract_positional_string(&call.call, idx, call.strip).unwrap_or_default();
    let span = positional_span(&call.call, idx).unwrap_or(call.call.head);

    complete_tramp_path(&plugin.vfs, &plugin.remote_cwd, &partial, span)
}

/// Read `$env.TRAMP_CACHE_TTL` (in seconds) from the Nushell environment and
/// apply it to the VFS cache.  If the variable is not set or cannot be parsed,
/// the existing TTL is left unchanged.
///
/// Accepts integer seconds (`5`), float seconds (`2.5`), or a duration string
/// with an `s` / `ms` / `sec` suffix (e.g. `500ms`).
fn apply_cache_ttl(engine: &EngineInterface, vfs: &Vfs) {
    let val = match engine.get_env_var("TRAMP_CACHE_TTL") {
        Ok(Some(v)) => v,
        _ => return,
    };

    let ttl = match &val {
        Value::Int { val, .. } => {
            if *val >= 0 {
                Some(Duration::from_secs(*val as u64))
            } else {
                None
            }
        }
        Value::Float { val, .. } => {
            if *val >= 0.0 {
                Some(Duration::from_secs_f64(*val))
            } else {
                None
            }
        }
        Value::Duration { val, .. } => {
            // Nushell durations are stored in nanoseconds.
            if *val >= 0 {
                Some(Duration::from_nanos(*val as u64))
            } else {
                None
            }
        }
        Value::String { val, .. } => parse_ttl_string(val),
        _ => None,
    };

    if let Some(ttl) = ttl {
        vfs.set_cache_ttl(ttl);
    }
}

/// Parse a human-friendly TTL string like `"5"`, `"2.5"`, `"500ms"`, `"10s"`, `"0"`.
fn parse_ttl_string(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(ms) = s.strip_suffix("ms") {
        return ms
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| *v >= 0.0)
            .map(|v| Duration::from_secs_f64(v / 1000.0));
    }
    let s = s.strip_suffix('s').or(Some(s)).unwrap_or(s);
    let s = s.strip_suffix("sec").unwrap_or(s);
    s.trim()
        .parse::<f64>()
        .ok()
        .filter(|v| *v >= 0.0)
        .map(Duration::from_secs_f64)
}

// ---------------------------------------------------------------------------
// `tramp` (main / help)
// ---------------------------------------------------------------------------

struct TrampMain;

impl PluginCommand for TrampMain {
    type Plugin = TrampPlugin;

    fn name(&self) -> &str {
        "tramp"
    }

    fn description(&self) -> &str {
        "TRAMP-style remote filesystem access for Nushell"
    }

    fn extra_description(&self) -> &str {
        r#"
nu-plugin-tramp lets you transparently access remote files using TRAMP-style URIs:

    tramp open /ssh:myvm:/etc/hostname
    tramp ls   /ssh:myvm:/var/log
    tramp save /ssh:myvm:/app/config.toml
    tramp rm   /ssh:myvm:/tmp/stale.lock
    tramp cp   /ssh:myvm:/etc/config ./local-copy

Supported backends: ssh, docker, k8s (kubernetes), sudo

Chained paths let you reach nested environments:

    tramp open /ssh:myvm|docker:ctr:/app/config.toml
    tramp ls   /ssh:myvm|sudo:root:/etc
    tramp exec /ssh:myvm|docker:ctr:/ -- hostname

Glob/wildcard filtering:

    tramp ls /ssh:myvm:/var/log --glob '*.log'
    tramp cp /ssh:myvm:/var/log ./logs --glob '*.log'

System information (gathered in a single round-trip):

    tramp info /ssh:myvm:/
    tramp info /docker:mycontainer:/

Set a remote working directory to use relative paths:

    tramp cd /ssh:myvm:/app
    tramp open config.toml     # resolves to /ssh:myvm:/app/config.toml
    tramp ls                   # lists /app on myvm

Filesystem watching (requires RPC agent):

    tramp watch /ssh:myvm:/app --duration 5000
    tramp watch /ssh:myvm:/app --recursive
    tramp watch /ssh:myvm:/ --list
    tramp watch /ssh:myvm:/app --remove

Interactive remote shell (requires RPC agent):

    tramp shell /ssh:myvm:/
    tramp shell /ssh:myvm:/app --shell /bin/bash
    tramp shell /docker:mycontainer:/
    tramp shell /ssh:myvm|docker:ctr:/

Connection management:

    tramp ping /ssh:myvm:/
    tramp connections
    tramp disconnect myvm

Cache configuration (default TTL: 5 seconds):

    $env.TRAMP_CACHE_TTL = 10sec   # longer cache
    $env.TRAMP_CACHE_TTL = 0sec    # disable caching

Path format: /<backend>:<user>@<host>#<port>:<remote-path>
"#
        .trim()
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name()).category(Category::FileSystem)
    }

    fn search_terms(&self) -> Vec<&str> {
        vec![
            "tramp",
            "remote",
            "ssh",
            "sftp",
            "docker",
            "kubernetes",
            "sudo",
        ]
    }

    fn run(
        &self,
        _plugin: &TrampPlugin,
        engine: &EngineInterface,
        _call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        Ok(PipelineData::Value(
            Value::string(engine.get_help()?, Span::unknown()),
            None,
        ))
    }
}

// ---------------------------------------------------------------------------
// `tramp open`
// ---------------------------------------------------------------------------

struct TrampOpen;

impl PluginCommand for TrampOpen {
    type Plugin = TrampPlugin;

    fn name(&self) -> &str {
        "tramp open"
    }

    fn description(&self) -> &str {
        "Read a remote file via a TRAMP URI"
    }

    fn extra_description(&self) -> &str {
        "Reads the remote file and returns it as a Nushell value. \
         Text files are returned as strings; binary files as binary data. \
         Pipe through `from json`, `from toml`, etc. for structured parsing."
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .required("path", SyntaxShape::String, "TRAMP URI of the remote file")
            .input_output_types(vec![(Type::Nothing, Type::Any)])
            .category(Category::FileSystem)
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["tramp", "open", "read", "remote", "ssh", "sftp"]
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "tramp open /ssh:myvm:/etc/hostname",
                description: "Read a remote text file",
                result: None,
            },
            Example {
                example: "tramp open /ssh:admin@myvm:/app/config.json | from json",
                description: "Read and parse a remote JSON file",
                result: None,
            },
            Example {
                example: "tramp cd /ssh:myvm:/app; tramp open config.toml",
                description: "Read a file relative to the remote CWD",
                result: None,
            },
        ]
    }

    fn run(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        apply_cache_ttl(engine, &plugin.vfs);
        let path = parse_tramp_arg(call, &plugin.remote_cwd)?;
        let span = call.head;
        let signals = engine.signals().clone();

        // Use chunked streaming when the backend supports it and the file
        // is larger than the threshold (1 MB).  This avoids loading the
        // entire file into a single Value, which matters for large binaries.
        const STREAM_THRESHOLD: u64 = 1024 * 1024; // 1 MB
        const CHUNK_SIZE: u64 = 1024 * 1024; // 1 MB per chunk

        let use_streaming = plugin.vfs.supports_streaming(&path)
            && plugin
                .vfs
                .file_size(&path)
                .map(|sz| sz > STREAM_THRESHOLD)
                .unwrap_or(false);

        if use_streaming {
            let vfs = Arc::clone(&plugin.vfs);
            let remote_path = path.remote_path.clone();
            let path_for_stream = path.clone();

            // Determine the byte-stream type from the file extension.
            let ext = std::path::Path::new(&remote_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            let stream_type = match ext {
                "txt" | "log" | "md" | "csv" | "tsv" | "json" | "toml" | "yaml" | "yml" | "xml"
                | "html" | "css" | "js" | "ts" | "rs" | "py" | "sh" | "nu" | "conf" | "cfg"
                | "ini" => ByteStreamType::String,
                _ => ByteStreamType::Binary,
            };

            let byte_stream = ByteStream::from_fn(span, signals, stream_type, {
                let mut offset: u64 = 0;
                let mut done = false;
                move |buf: &mut Vec<u8>| {
                    if done {
                        return Ok(false);
                    }
                    let (chunk, eof) = vfs
                        .read_range(&path_for_stream, offset, CHUNK_SIZE)
                        .map_err(|e| nu_protocol::ShellError::GenericError {
                            error: "tramp read error".into(),
                            msg: e.to_string(),
                            span: Some(span),
                            help: None,
                            inner: vec![],
                        })?;
                    if chunk.is_empty() {
                        done = true;
                        return Ok(false);
                    }
                    offset += chunk.len() as u64;
                    if eof {
                        done = true;
                    }
                    buf.extend_from_slice(&chunk);
                    Ok(true)
                }
            });

            return Ok(PipelineData::ByteStream(byte_stream, None));
        }

        // Fallback: read the entire file into memory (small files or
        // backends that don't support streaming).
        let data = plugin
            .vfs
            .read(&path)
            .map_err(|e| tramp_err(e, call.head))?;

        let value = bytes_to_value(&data, &path.remote_path, span);
        Ok(PipelineData::Value(value, None))
    }

    #[allow(unused_variables, deprecated)]
    fn get_dynamic_completion(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: DynamicCompletionCall,
        arg_type: ArgType,
        _experimental: nu_protocol::engine::ExperimentalMarker,
    ) -> Option<Vec<DynamicSuggestion>> {
        complete_tramp_arg(plugin, call, arg_type, 0)
    }
}

// ---------------------------------------------------------------------------
// `tramp ls`
// ---------------------------------------------------------------------------

struct TrampLs;

impl PluginCommand for TrampLs {
    type Plugin = TrampPlugin;

    fn name(&self) -> &str {
        "tramp ls"
    }

    fn description(&self) -> &str {
        "List a remote directory via a TRAMP URI"
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .optional(
                "path",
                SyntaxShape::String,
                "TRAMP URI of the remote directory (defaults to remote CWD if set)",
            )
            .named(
                "glob",
                SyntaxShape::String,
                "Filter entries by glob pattern (e.g. '*.log', 'config.*')",
                Some('g'),
            )
            .input_output_types(vec![(Type::Nothing, Type::table())])
            .category(Category::FileSystem)
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["tramp", "ls", "list", "dir", "remote", "ssh"]
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "tramp ls /ssh:myvm:/var/log",
                description: "List files in a remote directory",
                result: None,
            },
            Example {
                example: "tramp ls /ssh:myvm:/ | where type == dir",
                description: "List only directories at the remote root",
                result: None,
            },
            Example {
                example: "tramp cd /ssh:myvm:/var/log; tramp ls",
                description: "List the current remote working directory",
                result: None,
            },
            Example {
                example: "tramp ls /ssh:myvm:/var/log --glob '*.log'",
                description: "List only .log files in a remote directory",
                result: None,
            },
            Example {
                example: "tramp ls /ssh:myvm:/etc -g 'host*'",
                description: "List entries matching a glob pattern",
                result: None,
            },
        ]
    }

    #[allow(unused_variables, deprecated)]
    fn get_dynamic_completion(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: DynamicCompletionCall,
        arg_type: ArgType,
        _experimental: nu_protocol::engine::ExperimentalMarker,
    ) -> Option<Vec<DynamicSuggestion>> {
        complete_tramp_arg(plugin, call, arg_type, 0)
    }

    fn run(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        apply_cache_ttl(engine, &plugin.vfs);
        let glob_pattern: Option<String> = call.get_flag("glob")?;
        let path = match call.opt::<String>(0)? {
            Some(raw) => resolve_tramp_path(&raw, &plugin.remote_cwd, call.head)?,
            None => {
                // No argument — use the remote CWD if set.
                let cwd_guard = plugin
                    .remote_cwd
                    .lock()
                    .map_err(|e| LabeledError::new(format!("internal lock error: {e}")))?;
                match &*cwd_guard {
                    Some(cwd) => cwd.clone(),
                    None => {
                        return Err(LabeledError::new(
                            "no path provided and no remote working directory set.\n\
                             Use `tramp cd /ssh:host:/dir` first, or pass a TRAMP URI.",
                        )
                        .with_label("missing path", call.head));
                    }
                }
            }
        };

        let entries = plugin
            .vfs
            .list(&path)
            .map_err(|e| tramp_err(e, call.head))?;

        // Apply glob filter if --glob / -g was provided.
        let entries: Vec<_> = if let Some(ref pattern) = glob_pattern {
            entries
                .into_iter()
                .filter(|e| glob_match::glob_match(pattern, &e.name))
                .collect()
        } else {
            entries
        };

        let span = call.head;
        let rows: Vec<Value> = entries
            .into_iter()
            .map(|entry| {
                let kind_str = match entry.kind {
                    backend::EntryKind::File => "file",
                    backend::EntryKind::Dir => "dir",
                    backend::EntryKind::Symlink => "symlink",
                };

                let mut record = Record::new();
                record.push("name", Value::string(&entry.name, span));
                record.push("type", Value::string(kind_str, span));

                if let Some(size) = entry.size {
                    record.push("size", Value::filesize(size as i64, span));
                } else {
                    record.push("size", Value::nothing(span));
                }

                if let Some(modified) = entry.modified {
                    if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
                        let nanos = duration.as_nanos() as i64;
                        record.push("modified", Value::date(chrono_from_nanos(nanos), span));
                    } else {
                        record.push("modified", Value::nothing(span));
                    }
                } else {
                    record.push("modified", Value::nothing(span));
                }

                if let Some(perms) = entry.permissions {
                    record.push("mode", Value::string(format!("{perms:o}"), span));
                } else {
                    record.push("mode", Value::nothing(span));
                }

                // Richer metadata fields (inspired by emacs-tramp-rpc's
                // native stat approach — all gathered in a single remote
                // command to minimise round-trips).
                if let Some(ref owner) = entry.owner {
                    record.push("owner", Value::string(owner, span));
                } else {
                    record.push("owner", Value::nothing(span));
                }

                if let Some(ref group) = entry.group {
                    record.push("group", Value::string(group, span));
                } else {
                    record.push("group", Value::nothing(span));
                }

                if let Some(nlinks) = entry.nlinks {
                    record.push("nlinks", Value::int(nlinks as i64, span));
                } else {
                    record.push("nlinks", Value::nothing(span));
                }

                if let Some(inode) = entry.inode {
                    record.push("inode", Value::int(inode as i64, span));
                } else {
                    record.push("inode", Value::nothing(span));
                }

                if let Some(ref target) = entry.symlink_target {
                    record.push("target", Value::string(target, span));
                } else {
                    record.push("target", Value::nothing(span));
                }

                Value::record(record, span)
            })
            .collect();

        Ok(PipelineData::Value(Value::list(rows, span), None))
    }
}

/// Convert nanoseconds since UNIX epoch to a chrono DateTime suitable for
/// Nushell's `Value::date`.
fn chrono_from_nanos(nanos: i64) -> chrono::DateTime<chrono::FixedOffset> {
    let secs = nanos / 1_000_000_000;
    let nsecs = (nanos % 1_000_000_000) as u32;
    let dt = chrono::DateTime::from_timestamp(secs, nsecs)
        .unwrap_or_else(|| chrono::DateTime::from_timestamp(0, 0).unwrap());
    dt.with_timezone(&chrono::FixedOffset::east_opt(0).unwrap())
}

// ---------------------------------------------------------------------------
// `tramp save`
// ---------------------------------------------------------------------------

struct TrampSave;

impl PluginCommand for TrampSave {
    type Plugin = TrampPlugin;

    fn name(&self) -> &str {
        "tramp save"
    }

    fn description(&self) -> &str {
        "Write piped data to a remote file via a TRAMP URI"
    }

    fn extra_description(&self) -> &str {
        "Accepts string or binary pipeline input and writes it to the remote path. \
         To save structured data, first convert it with `to json`, `to toml`, etc."
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .required(
                "path",
                SyntaxShape::String,
                "TRAMP URI of the remote file to write",
            )
            .input_output_types(vec![
                (Type::String, Type::Nothing),
                (Type::Binary, Type::Nothing),
                (Type::Any, Type::Nothing),
            ])
            .category(Category::FileSystem)
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["tramp", "save", "write", "remote", "ssh", "sftp"]
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: r#""hello world" | tramp save /ssh:myvm:/tmp/hello.txt"#,
                description: "Write a string to a remote file",
                result: None,
            },
            Example {
                example: "open config.toml | to toml | tramp save /ssh:myvm:/app/config.toml",
                description: "Save a local config file to a remote host",
                result: None,
            },
        ]
    }

    #[allow(unused_variables, deprecated)]
    fn get_dynamic_completion(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: DynamicCompletionCall,
        arg_type: ArgType,
        _experimental: nu_protocol::engine::ExperimentalMarker,
    ) -> Option<Vec<DynamicSuggestion>> {
        complete_tramp_arg(plugin, call, arg_type, 0)
    }

    fn run(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        apply_cache_ttl(engine, &plugin.vfs);
        let path = parse_tramp_arg(call, &plugin.remote_cwd)?;
        let span = call.head;

        const CHUNK_SIZE: usize = 1024 * 1024; // 1 MB per chunk

        // When the input is a ByteStream and the backend supports streaming,
        // write in chunks to avoid buffering the entire payload in memory.
        if let PipelineData::ByteStream(stream, _) = input {
            if plugin.vfs.supports_streaming(&path) {
                let mut reader = stream
                    .reader()
                    .ok_or_else(|| LabeledError::new("failed to get byte stream reader"))?;

                let mut offset: u64 = 0;
                let mut first = true;
                let mut buf = vec![0u8; CHUNK_SIZE];

                loop {
                    let n = std::io::Read::read(&mut reader, &mut buf)
                        .map_err(|e| LabeledError::new(format!("read error: {e}")))?;
                    if n == 0 {
                        // If we never wrote anything, create an empty file.
                        if first {
                            plugin
                                .vfs
                                .write_range(&path, 0, bytes::Bytes::new(), true)
                                .map_err(|e| tramp_err(e, span))?;
                        }
                        break;
                    }
                    let chunk = bytes::Bytes::copy_from_slice(&buf[..n]);
                    plugin
                        .vfs
                        .write_range(&path, offset, chunk, first)
                        .map_err(|e| tramp_err(e, span))?;
                    offset += n as u64;
                    first = false;
                }

                return Ok(PipelineData::Empty);
            }

            // Fallback: collect the entire ByteStream into memory.
            let collected = reader_stream_to_bytes(stream)?;
            plugin
                .vfs
                .write(&path, collected)
                .map_err(|e| tramp_err(e, span))?;
            return Ok(PipelineData::Empty);
        }

        // Non-ByteStream input: collect into bytes and write in one shot.
        let data = pipeline_to_bytes(input)?;

        plugin
            .vfs
            .write(&path, data)
            .map_err(|e| tramp_err(e, span))?;

        Ok(PipelineData::Empty)
    }
}

/// Collect a ByteStream into `Bytes` (fallback when chunked write is not available).
fn reader_stream_to_bytes(stream: ByteStream) -> Result<bytes::Bytes, LabeledError> {
    let collected = stream
        .into_bytes()
        .map_err(|e| LabeledError::new(e.to_string()))?;
    Ok(bytes::Bytes::from(collected))
}

/// Collect pipeline data into a `Bytes` value.
fn pipeline_to_bytes(input: PipelineData) -> Result<bytes::Bytes, LabeledError> {
    match input {
        PipelineData::Value(value, _) => match value {
            Value::String { val, .. } => Ok(bytes::Bytes::from(val.into_bytes())),
            Value::Binary { val, .. } => Ok(bytes::Bytes::from(val)),
            Value::Nothing { .. } => Ok(bytes::Bytes::new()),
            other => {
                // Convert arbitrary values to their display string.
                let s = other.to_expanded_string(", ", &nu_protocol::Config::default());
                Ok(bytes::Bytes::from(s.into_bytes()))
            }
        },
        PipelineData::ListStream(stream, _) => {
            let mut buf = Vec::new();
            for value in stream {
                match value {
                    Value::String { val, .. } => buf.extend_from_slice(val.as_bytes()),
                    Value::Binary { val, .. } => buf.extend_from_slice(&val),
                    other => {
                        let s = other.to_expanded_string(", ", &nu_protocol::Config::default());
                        buf.extend_from_slice(s.as_bytes());
                    }
                }
            }
            Ok(bytes::Bytes::from(buf))
        }
        PipelineData::ByteStream(stream, _) => {
            let collected = stream
                .into_bytes()
                .map_err(|e| LabeledError::new(e.to_string()))?;
            Ok(bytes::Bytes::from(collected))
        }
        PipelineData::Empty => Ok(bytes::Bytes::new()),
    }
}

// ---------------------------------------------------------------------------
// `tramp rm`
// ---------------------------------------------------------------------------

struct TrampRm;

impl PluginCommand for TrampRm {
    type Plugin = TrampPlugin;

    fn name(&self) -> &str {
        "tramp rm"
    }

    fn description(&self) -> &str {
        "Delete a remote file via a TRAMP URI"
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .required(
                "path",
                SyntaxShape::String,
                "TRAMP URI of the remote file to delete",
            )
            .input_output_types(vec![(Type::Nothing, Type::Nothing)])
            .category(Category::FileSystem)
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["tramp", "rm", "remove", "delete", "remote", "ssh"]
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![Example {
            example: "tramp rm /ssh:myvm:/tmp/stale.lock",
            description: "Delete a remote file",
            result: None,
        }]
    }

    #[allow(unused_variables, deprecated)]
    fn get_dynamic_completion(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: DynamicCompletionCall,
        arg_type: ArgType,
        _experimental: nu_protocol::engine::ExperimentalMarker,
    ) -> Option<Vec<DynamicSuggestion>> {
        complete_tramp_arg(plugin, call, arg_type, 0)
    }

    fn run(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        apply_cache_ttl(engine, &plugin.vfs);
        let path = parse_tramp_arg(call, &plugin.remote_cwd)?;

        plugin
            .vfs
            .delete(&path)
            .map_err(|e| tramp_err(e, call.head))?;

        Ok(PipelineData::Empty)
    }
}

// ---------------------------------------------------------------------------
// `tramp cp`
// ---------------------------------------------------------------------------

struct TrampCp;

impl PluginCommand for TrampCp {
    type Plugin = TrampPlugin;

    fn name(&self) -> &str {
        "tramp cp"
    }

    fn description(&self) -> &str {
        "Copy a file between remote hosts or between local and remote"
    }

    fn extra_description(&self) -> &str {
        "Copies a file from source to destination. Both, one, or neither \
         may be TRAMP URIs. When a path is not a TRAMP URI, it is treated \
         as a local file path.\n\n\
         Use --glob to copy multiple files matching a pattern from a remote \
         directory to a local or remote destination directory.\n\n\
         Examples:\n\
         - Remote → Remote: tramp cp /ssh:vm1:/file /ssh:vm2:/file\n\
         - Remote → Local:  tramp cp /ssh:vm:/file ./local-copy\n\
         - Local → Remote:  tramp cp ./local-file /ssh:vm:/file\n\
         - Glob copy:       tramp cp /ssh:vm:/var/log /tmp/logs --glob '*.log'"
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .required(
                "source",
                SyntaxShape::String,
                "Source path (local or TRAMP URI)",
            )
            .required(
                "destination",
                SyntaxShape::String,
                "Destination path (local or TRAMP URI)",
            )
            .named(
                "glob",
                SyntaxShape::String,
                "Copy all files matching a glob pattern from the source directory",
                Some('g'),
            )
            .input_output_types(vec![(Type::Nothing, Type::Nothing)])
            .category(Category::FileSystem)
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["tramp", "cp", "copy", "remote", "ssh", "transfer"]
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "tramp cp /ssh:vm1:/etc/config /ssh:vm2:/etc/config",
                description: "Copy a file between two remote hosts",
                result: None,
            },
            Example {
                example: "tramp cp /ssh:myvm:/etc/hostname ./hostname",
                description: "Copy a remote file to local",
                result: None,
            },
            Example {
                example: "tramp cp ./config.toml /ssh:myvm:/app/config.toml",
                description: "Copy a local file to remote",
                result: None,
            },
            Example {
                example: "tramp cp /ssh:myvm:/var/log ./logs --glob '*.log'",
                description: "Copy all .log files from remote to a local directory",
                result: None,
            },
        ]
    }

    #[allow(unused_variables, deprecated)]
    fn get_dynamic_completion(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: DynamicCompletionCall,
        arg_type: ArgType,
        _experimental: nu_protocol::engine::ExperimentalMarker,
    ) -> Option<Vec<DynamicSuggestion>> {
        // Both source (pos 0) and destination (pos 1) can be TRAMP paths.
        match &arg_type {
            ArgType::Positional(0) => complete_tramp_arg(plugin, call, arg_type, 0),
            ArgType::Positional(1) => complete_tramp_arg(plugin, call, arg_type, 1),
            _ => None,
        }
    }

    fn run(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        apply_cache_ttl(engine, &plugin.vfs);

        let src_raw: String = call.req(0)?;
        let dst_raw: String = call.req(1)?;
        let glob_pattern: Option<String> = call.get_flag("glob")?;

        let src_tramp = protocol::parse(&src_raw).map_err(|e| {
            LabeledError::new(e.to_string()).with_label("source parse error", call.head)
        })?;
        let dst_tramp = protocol::parse(&dst_raw).map_err(|e| {
            LabeledError::new(e.to_string()).with_label("destination parse error", call.head)
        })?;

        // Also try relative resolution for source and destination.
        let src_tramp = match src_tramp {
            Some(p) => Some(p),
            None => resolve_tramp_path(&src_raw, &plugin.remote_cwd, call.head).ok(),
        };
        let dst_tramp = match dst_tramp {
            Some(p) => Some(p),
            None => resolve_tramp_path(&dst_raw, &plugin.remote_cwd, call.head).ok(),
        };

        // ---- Glob mode: copy multiple files matching a pattern ----
        if let Some(ref pattern) = glob_pattern {
            // Source must be a directory; list it and filter by glob.
            let src_dir = match &src_tramp {
                Some(p) => p.clone(),
                None => {
                    return Err(LabeledError::new(
                        "glob copy from local directories is not yet supported.\n\
                         Use a TRAMP URI as the source with --glob.",
                    )
                    .with_label("unsupported", call.head));
                }
            };

            let entries = plugin
                .vfs
                .list(&src_dir)
                .map_err(|e| tramp_err(e, call.head))?;

            let matched: Vec<_> = entries
                .into_iter()
                .filter(|e| {
                    e.kind == backend::EntryKind::File && glob_match::glob_match(pattern, &e.name)
                })
                .collect();

            if matched.is_empty() {
                return Err(LabeledError::new(format!(
                    "no files matching '{}' in {}",
                    pattern, src_dir.remote_path,
                ))
                .with_label("no matches", call.head));
            }

            // Ensure the destination directory exists (for remote: best-effort mkdir).
            if dst_tramp.is_none() {
                std::fs::create_dir_all(&dst_raw).map_err(|e| {
                    LabeledError::new(format!(
                        "failed to create local directory '{}': {}",
                        dst_raw, e
                    ))
                    .with_label("local dir error", call.head)
                })?;
            }

            let mut copied = 0u64;
            for entry in &matched {
                // Build source path for this file.
                let src_file_remote = format!(
                    "{}/{}",
                    src_dir.remote_path.trim_end_matches('/'),
                    entry.name
                );
                let mut src_file_path = src_dir.clone();
                src_file_path.remote_path = src_file_remote;

                let data = plugin
                    .vfs
                    .read(&src_file_path)
                    .map_err(|e| tramp_err(e, call.head))?;

                // Build destination path for this file.
                match &dst_tramp {
                    Some(dst_dir) => {
                        let dst_file_remote = format!(
                            "{}/{}",
                            dst_dir.remote_path.trim_end_matches('/'),
                            entry.name
                        );
                        let mut dst_file_path = dst_dir.clone();
                        dst_file_path.remote_path = dst_file_remote;
                        plugin
                            .vfs
                            .write(&dst_file_path, data)
                            .map_err(|e| tramp_err(e, call.head))?;
                    }
                    None => {
                        let dst_file = std::path::Path::new(&dst_raw).join(&entry.name);
                        std::fs::write(&dst_file, &data).map_err(|e| {
                            LabeledError::new(format!(
                                "failed to write local file '{}': {}",
                                dst_file.display(),
                                e
                            ))
                            .with_label("local write error", call.head)
                        })?;
                    }
                }
                copied += 1;
            }

            return Ok(PipelineData::Value(
                Value::string(
                    format!("copied {} file(s) matching '{}'", copied, pattern),
                    call.head,
                ),
                None,
            ));
        }

        // ---- Single-file copy (original behavior) ----

        // Read the source data.
        let data: bytes::Bytes = match src_tramp {
            Some(ref src_path) => plugin
                .vfs
                .read(src_path)
                .map_err(|e| tramp_err(e, call.head))?,
            None => {
                // Local file read.
                let contents = std::fs::read(&src_raw).map_err(|e| {
                    LabeledError::new(format!("failed to read local file '{}': {}", src_raw, e))
                        .with_label("local read error", call.head)
                })?;
                bytes::Bytes::from(contents)
            }
        };

        // Write the destination data.
        match dst_tramp {
            Some(ref dst_path) => {
                plugin
                    .vfs
                    .write(dst_path, data)
                    .map_err(|e| tramp_err(e, call.head))?;
            }
            None => {
                // Local file write.
                std::fs::write(&dst_raw, &data).map_err(|e| {
                    LabeledError::new(format!("failed to write local file '{}': {}", dst_raw, e))
                        .with_label("local write error", call.head)
                })?;
            }
        }

        Ok(PipelineData::Empty)
    }
}

// ---------------------------------------------------------------------------
// `tramp cd`
// ---------------------------------------------------------------------------

struct TrampCd;

impl PluginCommand for TrampCd {
    type Plugin = TrampPlugin;

    fn name(&self) -> &str {
        "tramp cd"
    }

    fn description(&self) -> &str {
        "Set the remote working directory for subsequent tramp commands"
    }

    fn extra_description(&self) -> &str {
        "Sets a remote CWD so that subsequent tramp commands can accept \
         relative paths. The path must be a TRAMP URI pointing to a \
         directory (or use a relative path if a remote CWD is already set).\n\n\
         Use `tramp cd --reset` to clear the remote CWD.\n\
         Use `tramp pwd` to see the current remote CWD."
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .optional(
                "path",
                SyntaxShape::String,
                "TRAMP URI or relative path to set as the remote working directory",
            )
            .switch("reset", "Clear the remote working directory", Some('r'))
            .input_output_types(vec![(Type::Nothing, Type::String)])
            .category(Category::FileSystem)
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["tramp", "cd", "directory", "remote", "ssh"]
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "tramp cd /ssh:myvm:/app",
                description: "Set the remote CWD to /app on myvm",
                result: None,
            },
            Example {
                example: "tramp cd subdir",
                description: "Navigate to a subdirectory relative to the current remote CWD",
                result: None,
            },
            Example {
                example: "tramp cd ..",
                description: "Go up one directory",
                result: None,
            },
            Example {
                example: "tramp cd --reset",
                description: "Clear the remote working directory",
                result: None,
            },
        ]
    }

    #[allow(unused_variables, deprecated)]
    fn get_dynamic_completion(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: DynamicCompletionCall,
        arg_type: ArgType,
        _experimental: nu_protocol::engine::ExperimentalMarker,
    ) -> Option<Vec<DynamicSuggestion>> {
        complete_tramp_arg(plugin, call, arg_type, 0)
    }

    fn run(
        &self,
        plugin: &TrampPlugin,
        _engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        // Handle --reset flag.
        if call.has_flag("reset")? {
            let mut cwd_guard = plugin
                .remote_cwd
                .lock()
                .map_err(|e| LabeledError::new(format!("internal lock error: {e}")))?;
            *cwd_guard = None;
            return Ok(PipelineData::Value(
                Value::string("remote CWD cleared", call.head),
                None,
            ));
        }

        let raw: Option<String> = call.opt(0)?;
        let raw = match raw {
            Some(r) => r,
            None => {
                // No argument, no reset — show the current CWD.
                let cwd_guard = plugin
                    .remote_cwd
                    .lock()
                    .map_err(|e| LabeledError::new(format!("internal lock error: {e}")))?;
                return match &*cwd_guard {
                    Some(cwd) => Ok(PipelineData::Value(
                        Value::string(cwd.to_string(), call.head),
                        None,
                    )),
                    None => Ok(PipelineData::Value(
                        Value::string("(no remote CWD set)", call.head),
                        None,
                    )),
                };
            }
        };

        // Resolve the target path (may be absolute TRAMP URI or relative).
        let target = resolve_tramp_path(&raw, &plugin.remote_cwd, call.head)?;

        // Verify the path exists and is a directory.
        let meta = plugin
            .vfs
            .stat(&target)
            .map_err(|e| tramp_err(e, call.head))?;

        if meta.kind != backend::EntryKind::Dir {
            return Err(
                LabeledError::new(format!("not a directory: {}", target.remote_path))
                    .with_label("expected a directory", call.head),
            );
        }

        let display = target.to_string();

        // Store the new CWD.
        {
            let mut cwd_guard = plugin
                .remote_cwd
                .lock()
                .map_err(|e| LabeledError::new(format!("internal lock error: {e}")))?;
            *cwd_guard = Some(target);
        }

        Ok(PipelineData::Value(Value::string(display, call.head), None))
    }
}

// ---------------------------------------------------------------------------
// `tramp pwd`
// ---------------------------------------------------------------------------

struct TrampPwd;

impl PluginCommand for TrampPwd {
    type Plugin = TrampPlugin;

    fn name(&self) -> &str {
        "tramp pwd"
    }

    fn description(&self) -> &str {
        "Show the current remote working directory"
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .input_output_types(vec![(Type::Nothing, Type::String)])
            .category(Category::FileSystem)
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["tramp", "pwd", "directory", "remote"]
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![Example {
            example: "tramp pwd",
            description: "Show the current remote working directory",
            result: None,
        }]
    }

    fn run(
        &self,
        plugin: &TrampPlugin,
        _engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        let cwd_guard = plugin
            .remote_cwd
            .lock()
            .map_err(|e| LabeledError::new(format!("internal lock error: {e}")))?;

        match &*cwd_guard {
            Some(cwd) => Ok(PipelineData::Value(
                Value::string(cwd.to_string(), call.head),
                None,
            )),
            None => Ok(PipelineData::Value(
                Value::string("(no remote CWD set)", call.head),
                None,
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// `tramp ping`
// ---------------------------------------------------------------------------

struct TrampPing;

impl PluginCommand for TrampPing {
    type Plugin = TrampPlugin;

    fn name(&self) -> &str {
        "tramp ping"
    }

    fn description(&self) -> &str {
        "Test connectivity to a remote host via a TRAMP URI"
    }

    fn extra_description(&self) -> &str {
        "Opens an SSH connection (or reuses a pooled one) and runs a \
         health-check command (`true`) on the remote. Reports success \
         or failure with timing information."
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .required(
                "path",
                SyntaxShape::String,
                "TRAMP URI to test (e.g. /ssh:myvm:/)",
            )
            .input_output_types(vec![(Type::Nothing, Type::record())])
            .category(Category::FileSystem)
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["tramp", "ping", "test", "connection", "ssh"]
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![Example {
            example: "tramp ping /ssh:myvm:/",
            description: "Test SSH connectivity to myvm",
            result: None,
        }]
    }

    #[allow(unused_variables, deprecated)]
    fn get_dynamic_completion(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: DynamicCompletionCall,
        arg_type: ArgType,
        _experimental: nu_protocol::engine::ExperimentalMarker,
    ) -> Option<Vec<DynamicSuggestion>> {
        complete_tramp_arg(plugin, call, arg_type, 0)
    }

    fn run(
        &self,
        plugin: &TrampPlugin,
        _engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        let path = parse_tramp_arg(call, &plugin.remote_cwd)?;
        let span = call.head;

        let start = std::time::Instant::now();
        let result = plugin.vfs.ping(&path);
        let elapsed = start.elapsed();

        let mut record = Record::new();
        record.push("host", Value::string(&path.hops[0].host, span));
        record.push(
            "backend",
            Value::string(path.hops[0].backend.to_string(), span),
        );

        match result {
            Ok(()) => {
                record.push("status", Value::string("ok", span));
                record.push(
                    "time_ms",
                    Value::float(elapsed.as_secs_f64() * 1000.0, span),
                );
                record.push("error", Value::nothing(span));
            }
            Err(ref e) => {
                record.push("status", Value::string("error", span));
                record.push(
                    "time_ms",
                    Value::float(elapsed.as_secs_f64() * 1000.0, span),
                );
                record.push("error", Value::string(e.to_string(), span));
            }
        }

        Ok(PipelineData::Value(Value::record(record, span), None))
    }
}

// ---------------------------------------------------------------------------
// `tramp connections`
// ---------------------------------------------------------------------------

struct TrampConnections;

impl PluginCommand for TrampConnections {
    type Plugin = TrampPlugin;

    fn name(&self) -> &str {
        "tramp connections"
    }

    fn description(&self) -> &str {
        "List active remote connections"
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .input_output_types(vec![(Type::Nothing, Type::table())])
            .category(Category::FileSystem)
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["tramp", "connections", "pool", "ssh", "status"]
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![Example {
            example: "tramp connections",
            description: "Show all active remote connections",
            result: None,
        }]
    }

    fn run(
        &self,
        plugin: &TrampPlugin,
        _engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        let span = call.head;
        let connections = plugin.vfs.active_connections_detailed();

        if connections.is_empty() {
            return Ok(PipelineData::Value(Value::list(vec![], span), None));
        }

        let rows: Vec<Value> = connections
            .into_iter()
            .map(|info| {
                let mut record = Record::new();
                record.push("backend", Value::string(&info.backend, span));
                record.push(
                    "user",
                    info.user
                        .as_deref()
                        .map(|u| Value::string(u, span))
                        .unwrap_or_else(|| Value::nothing(span)),
                );
                record.push("host", Value::string(&info.host, span));
                record.push(
                    "port",
                    info.port
                        .map(|p| Value::int(p as i64, span))
                        .unwrap_or_else(|| Value::nothing(span)),
                );
                record.push(
                    "chain",
                    info.chain
                        .as_deref()
                        .map(|c| Value::string(c, span))
                        .unwrap_or_else(|| Value::nothing(span)),
                );
                Value::record(record, span)
            })
            .collect();

        Ok(PipelineData::Value(Value::list(rows, span), None))
    }
}

// ---------------------------------------------------------------------------
// `tramp disconnect`
// ---------------------------------------------------------------------------

struct TrampDisconnect;

impl PluginCommand for TrampDisconnect {
    type Plugin = TrampPlugin;

    fn name(&self) -> &str {
        "tramp disconnect"
    }

    fn description(&self) -> &str {
        "Close remote connections"
    }

    fn extra_description(&self) -> &str {
        "With a host argument, disconnects only sessions to that host. \
         With --all, disconnects every active connection. \
         Also clears any cached metadata for the affected hosts."
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .optional(
                "host",
                SyntaxShape::String,
                "Hostname to disconnect (e.g. 'myvm')",
            )
            .switch("all", "Disconnect all active connections", Some('a'))
            .input_output_types(vec![(Type::Nothing, Type::String)])
            .category(Category::FileSystem)
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["tramp", "disconnect", "close", "ssh"]
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "tramp disconnect myvm",
                description: "Close all connections to myvm",
                result: None,
            },
            Example {
                example: "tramp disconnect --all",
                description: "Close all remote connections",
                result: None,
            },
        ]
    }

    fn run(
        &self,
        plugin: &TrampPlugin,
        _engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        let span = call.head;

        if call.has_flag("all")? {
            let count = plugin.vfs.connection_count();
            plugin.vfs.disconnect_all();

            // Also clear the remote CWD since all connections are gone.
            if let Ok(mut cwd) = plugin.remote_cwd.lock() {
                *cwd = None;
            }

            return Ok(PipelineData::Value(
                Value::string(format!("disconnected {} connection(s)", count), span),
                None,
            ));
        }

        let host: Option<String> = call.opt(0)?;
        match host {
            Some(host) => {
                plugin.vfs.disconnect_host(&host);

                // If the remote CWD was on this host, clear it.
                if let Ok(mut cwd_guard) = plugin.remote_cwd.lock() {
                    let should_clear = cwd_guard
                        .as_ref()
                        .is_some_and(|cwd| cwd.hops.iter().any(|h| h.host == host));
                    if should_clear {
                        *cwd_guard = None;
                    }
                }

                Ok(PipelineData::Value(
                    Value::string(format!("disconnected from {host}"), span),
                    None,
                ))
            }
            None => Err(LabeledError::new(
                "provide a hostname to disconnect, or use --all to disconnect everything",
            )
            .with_label("missing argument", span)),
        }
    }
}

// ---------------------------------------------------------------------------
// `tramp exec`
// ---------------------------------------------------------------------------

struct TrampExec;

impl PluginCommand for TrampExec {
    type Plugin = TrampPlugin;

    fn name(&self) -> &str {
        "tramp exec"
    }

    fn description(&self) -> &str {
        "Execute a command on a remote host (push execution model)"
    }

    fn extra_description(&self) -> &str {
        "Runs an arbitrary command on the remote host identified by the \
         TRAMP URI and returns its stdout as a string (or binary if not \
         valid UTF-8). The remote path in the URI is ignored — only the \
         hop chain matters.\n\n\
         Use `--` to separate the TRAMP path from the command and its \
         arguments:\n\n\
         \x20   tramp exec /ssh:myvm:/ -- ls -la /tmp\n\
         \x20   tramp exec /ssh:myvm|docker:ctr:/ -- cat /etc/hostname"
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .required(
                "path",
                SyntaxShape::String,
                "TRAMP URI identifying the remote target",
            )
            .required(
                "command",
                SyntaxShape::String,
                "Command to execute on the remote",
            )
            .rest(
                "args",
                SyntaxShape::String,
                "Arguments to the remote command",
            )
            .input_output_types(vec![(Type::Nothing, Type::Any)])
            .category(Category::FileSystem)
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["tramp", "exec", "run", "remote", "ssh", "docker", "kubectl"]
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "tramp exec /ssh:myvm:/ -- ls -la /tmp",
                description: "List /tmp on a remote host",
                result: None,
            },
            Example {
                example: "tramp exec /ssh:myvm|docker:ctr:/ -- hostname",
                description: "Get the hostname inside a Docker container on a remote host",
                result: None,
            },
            Example {
                example: "tramp exec /docker:mycontainer:/ -- cat /etc/os-release",
                description: "Read a file inside a local Docker container",
                result: None,
            },
            Example {
                example: "tramp exec /sudo:root:/ -- cat /etc/shadow",
                description: "Read a file as root via sudo",
                result: None,
            },
        ]
    }

    #[allow(unused_variables, deprecated)]
    fn get_dynamic_completion(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: DynamicCompletionCall,
        arg_type: ArgType,
        _experimental: nu_protocol::engine::ExperimentalMarker,
    ) -> Option<Vec<DynamicSuggestion>> {
        complete_tramp_arg(plugin, call, arg_type, 0)
    }

    fn run(
        &self,
        plugin: &TrampPlugin,
        _engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        let path = parse_tramp_arg(call, &plugin.remote_cwd)?;
        let command: String = call.req(1)?;
        let extra_args: Vec<String> = call.rest(2)?;

        let arg_refs: Vec<&str> = extra_args.iter().map(|s| s.as_str()).collect();

        let result = plugin
            .vfs
            .exec(&path, &command, &arg_refs)
            .map_err(|e| tramp_err(e, call.head))?;

        let span = call.head;

        if result.exit_code != 0 {
            let stderr = String::from_utf8_lossy(&result.stderr);
            let msg = if stderr.trim().is_empty() {
                format!(
                    "remote command `{command}` exited with code {}",
                    result.exit_code
                )
            } else {
                format!(
                    "remote command `{command}` exited with code {}: {}",
                    result.exit_code,
                    stderr.trim()
                )
            };
            return Err(LabeledError::new(msg).with_label("non-zero exit", span));
        }

        // Return stdout as string if valid UTF-8, otherwise as binary.
        let value = match String::from_utf8(result.stdout.to_vec()) {
            Ok(s) => Value::string(s, span),
            Err(e) => Value::binary(e.into_bytes(), span),
        };

        Ok(PipelineData::Value(value, None))
    }
}

// ---------------------------------------------------------------------------
// `tramp info` — remote system information
// ---------------------------------------------------------------------------

/// Gather remote system information in a single round-trip.
///
/// Inspired by emacs-tramp-rpc's `system.info` + `system.statvfs` RPC
/// methods — collects hostname, OS, architecture, user, and disk usage
/// from the remote in one batch command.
struct TrampInfo;

impl PluginCommand for TrampInfo {
    type Plugin = TrampPlugin;

    fn name(&self) -> &str {
        "tramp info"
    }

    fn description(&self) -> &str {
        "Show remote system information (OS, arch, hostname, user, disk usage)"
    }

    fn extra_description(&self) -> &str {
        "Gathers all info in a single remote round-trip for efficiency.\n\
         Inspired by emacs-tramp-rpc's system.info RPC method."
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .required(
                "path",
                SyntaxShape::String,
                "TRAMP URI of the remote target",
            )
            .input_output_types(vec![(Type::Nothing, Type::record())])
            .category(Category::FileSystem)
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["tramp", "info", "system", "remote", "uname", "hostname"]
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "tramp info /ssh:myvm:/",
                description: "Show system info for a remote SSH host",
                result: None,
            },
            Example {
                example: "tramp info /docker:mycontainer:/",
                description: "Show system info inside a Docker container",
                result: None,
            },
            Example {
                example: "tramp info /ssh:myvm|docker:webapp:/",
                description: "Show system info inside a container on a remote host",
                result: None,
            },
        ]
    }

    #[allow(unused_variables, deprecated)]
    fn get_dynamic_completion(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: DynamicCompletionCall,
        arg_type: ArgType,
        _experimental: nu_protocol::engine::ExperimentalMarker,
    ) -> Option<Vec<DynamicSuggestion>> {
        complete_tramp_arg(plugin, call, arg_type, 0)
    }

    fn run(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        apply_cache_ttl(engine, &plugin.vfs);
        let path = parse_tramp_arg(call, &plugin.remote_cwd)?;

        // Gather everything in a single remote exec call to minimise
        // round-trips — this is the key pattern from emacs-tramp-rpc.
        let script = r#"printf '%s\t' \
  "$(uname -s)" \
  "$(uname -m)" \
  "$(hostname 2>/dev/null || cat /etc/hostname 2>/dev/null || echo unknown)" \
  "$(whoami 2>/dev/null || id -un 2>/dev/null || echo unknown)" \
  "$(uname -r)" \
  "$(nproc 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo unknown)"
# Disk usage for root filesystem (df -P for POSIX-portable output)
df -P / 2>/dev/null | tail -1"#;

        let result = plugin
            .vfs
            .exec(&path, "sh", &["-c", script])
            .map_err(|e| tramp_err(e, call.head))?;

        let stdout = String::from_utf8_lossy(&result.stdout);
        let lines: Vec<&str> = stdout.lines().collect();

        let span = call.head;
        let mut record = Record::new();

        // First line: tab-separated system fields
        if let Some(info_line) = lines.first() {
            let fields: Vec<&str> = info_line.split('\t').collect();

            record.push(
                "os",
                Value::string(fields.first().unwrap_or(&"unknown").trim(), span),
            );
            record.push(
                "arch",
                Value::string(fields.get(1).unwrap_or(&"unknown").trim(), span),
            );
            record.push(
                "hostname",
                Value::string(fields.get(2).unwrap_or(&"unknown").trim(), span),
            );
            record.push(
                "user",
                Value::string(fields.get(3).unwrap_or(&"unknown").trim(), span),
            );
            record.push(
                "kernel",
                Value::string(fields.get(4).unwrap_or(&"unknown").trim(), span),
            );
            record.push(
                "cpus",
                Value::string(fields.get(5).unwrap_or(&"unknown").trim(), span),
            );
        }

        // Second line: df output (filesystem  blocks  used  avail  capacity  mount)
        if let Some(df_line) = lines.get(1) {
            let df_fields: Vec<&str> = df_line.split_whitespace().collect();
            if df_fields.len() >= 4 {
                // df -P reports 1024-byte blocks
                if let Ok(total_kb) = df_fields[1].parse::<i64>() {
                    record.push("disk_total", Value::filesize(total_kb * 1024, span));
                }
                if let Ok(used_kb) = df_fields[2].parse::<i64>() {
                    record.push("disk_used", Value::filesize(used_kb * 1024, span));
                }
                if let Ok(avail_kb) = df_fields[3].parse::<i64>() {
                    record.push("disk_available", Value::filesize(avail_kb * 1024, span));
                }
                if let Some(pct) = df_fields.get(4) {
                    record.push("disk_use_pct", Value::string(*pct, span));
                }
            }
        }

        // Include the connection chain for context
        let chain = path.to_string();
        record.push("connection", Value::string(chain, span));

        Ok(PipelineData::Value(Value::record(record, span), None))
    }
}

// ---------------------------------------------------------------------------
// Watch command
// ---------------------------------------------------------------------------

struct TrampWatch;

impl PluginCommand for TrampWatch {
    type Plugin = TrampPlugin;

    fn name(&self) -> &str {
        "tramp watch"
    }

    fn description(&self) -> &str {
        "Watch a remote path for filesystem changes (requires RPC agent)"
    }

    fn extra_description(&self) -> &str {
        "Subscribes to filesystem change notifications on a remote path using\n\
         the RPC agent's inotify/kqueue integration.  The command watches for\n\
         the specified duration (default 10s), collects all change events, and\n\
         returns them as a table.\n\n\
         Requires the RPC agent to be deployed on the remote host (automatic\n\
         for SSH and standalone Docker/K8s backends).\n\n\
         Use --recursive to also watch subdirectories.\n\
         Use --duration to control how long to watch (in milliseconds).\n\
         Use --list to show currently active watches instead of starting a new one.\n\
         Use --remove to stop watching a previously added path."
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .required(
                "path",
                SyntaxShape::String,
                "TRAMP URI of the remote path to watch",
            )
            .switch("recursive", "Watch subdirectories recursively", Some('r'))
            .named(
                "duration",
                SyntaxShape::Int,
                "How long to watch in milliseconds (default: 10000)",
                Some('d'),
            )
            .switch("list", "List currently active watches", Some('l'))
            .switch("remove", "Remove an existing watch", None)
            .input_output_types(vec![
                (Type::Nothing, Type::table()),
                (Type::Nothing, Type::String),
            ])
            .category(Category::FileSystem)
    }

    fn search_terms(&self) -> Vec<&str> {
        vec![
            "tramp",
            "watch",
            "notify",
            "inotify",
            "filesystem",
            "changes",
        ]
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "tramp watch /ssh:myvm:/app --duration 5000",
                description: "Watch /app on a remote host for 5 seconds",
                result: None,
            },
            Example {
                example: "tramp watch /ssh:myvm:/app --recursive",
                description: "Watch /app and all subdirectories for changes",
                result: None,
            },
            Example {
                example: "tramp watch /ssh:myvm:/ --list",
                description: "List all active watches on the remote host",
                result: None,
            },
            Example {
                example: "tramp watch /ssh:myvm:/app --remove",
                description: "Stop watching /app",
                result: None,
            },
        ]
    }

    #[allow(unused_variables, deprecated)]
    fn get_dynamic_completion(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: DynamicCompletionCall,
        arg_type: ArgType,
        _experimental: nu_protocol::engine::ExperimentalMarker,
    ) -> Option<Vec<DynamicSuggestion>> {
        complete_tramp_arg(plugin, call, arg_type, 0)
    }

    fn run(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        apply_cache_ttl(engine, &plugin.vfs);
        let path = parse_tramp_arg(call, &plugin.remote_cwd)?;
        let span = call.head;

        let is_list = call.has_flag("list")?;
        let is_remove = call.has_flag("remove")?;

        // Check that the backend supports watching.
        let supports = plugin
            .vfs
            .supports_watch(&path)
            .map_err(|e| tramp_err(e, span))?;
        if !supports {
            return Err(
                LabeledError::new("filesystem watching is not supported by this backend")
                    .with_label(
                        "requires the RPC agent (only available for SSH and standalone Docker/K8s)",
                        span,
                    ),
            );
        }

        // --list: show currently active watches.
        if is_list {
            let watches = plugin
                .vfs
                .watch_list(&path)
                .map_err(|e| tramp_err(e, span))?;

            let rows: Vec<Value> = watches
                .iter()
                .map(|w| {
                    let mut record = Record::new();
                    record.push("path", Value::string(&w.path, span));
                    record.push("recursive", Value::bool(w.recursive, span));
                    Value::record(record, span)
                })
                .collect();

            return Ok(PipelineData::Value(Value::list(rows, span), None));
        }

        // --remove: stop watching.
        if is_remove {
            plugin
                .vfs
                .watch_remove(&path)
                .map_err(|e| tramp_err(e, span))?;

            return Ok(PipelineData::Value(
                Value::string(format!("watch removed: {}", path.remote_path), span),
                None,
            ));
        }

        // Default: add a watch and poll for notifications.
        let recursive = call.has_flag("recursive")?;
        let duration_ms: i64 = call
            .get_flag_value("duration")
            .and_then(|v| v.as_int().ok())
            .unwrap_or(10_000);

        if duration_ms < 0 {
            return Err(LabeledError::new("duration must be non-negative")
                .with_label("invalid duration", span));
        }

        // Add the watch.
        plugin
            .vfs
            .watch_add(&path, recursive)
            .map_err(|e| tramp_err(e, span))?;

        // Poll for notifications over the specified duration.
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(duration_ms as u64);
        let poll_interval = std::time::Duration::from_millis(250);
        let mut all_events: Vec<Value> = Vec::new();

        while std::time::Instant::now() < deadline {
            match plugin.vfs.watch_poll(&path) {
                Ok(notifications) => {
                    for notif in &notifications {
                        let mut record = Record::new();
                        record.push(
                            "paths",
                            Value::list(
                                notif.paths.iter().map(|p| Value::string(p, span)).collect(),
                                span,
                            ),
                        );
                        record.push("kind", Value::string(&notif.kind, span));
                        record.push(
                            "timestamp",
                            Value::string(
                                chrono::Local::now()
                                    .format("%Y-%m-%d %H:%M:%S%.3f")
                                    .to_string(),
                                span,
                            ),
                        );
                        all_events.push(Value::record(record, span));
                    }
                }
                Err(e) => {
                    // If polling fails, break out with what we have.
                    eprintln!("tramp: watch poll error: {e}");
                    break;
                }
            }

            // Sleep briefly before the next poll.
            std::thread::sleep(poll_interval);
        }

        // Clean up the watch.
        let _ = plugin.vfs.watch_remove(&path);

        Ok(PipelineData::Value(Value::list(all_events, span), None))
    }
}

// ---------------------------------------------------------------------------
// `tramp shell` — interactive remote terminal session
// ---------------------------------------------------------------------------

/// Start an interactive remote shell session using the RPC agent's PTY support.
///
/// Opens `/dev/tty` directly (bypassing the plugin's stdin/stdout MsgPack-RPC
/// channel), puts it in raw mode, and relays bytes bidirectionally between the
/// local terminal and the remote PTY.
#[cfg(unix)]
struct TrampShell;

#[cfg(unix)]
mod tty_raw {
    //! Minimal terminal helpers using `libc`.
    //!
    //! We access the controlling terminal via `/dev/tty` so that the plugin's
    //! stdin/stdout (used for MsgPack-RPC with Nushell) remain undisturbed.

    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::unix::io::AsRawFd;

    /// Open `/dev/tty` for read+write.
    pub fn open_tty() -> io::Result<File> {
        OpenOptions::new().read(true).write(true).open("/dev/tty")
    }

    /// Query the terminal dimensions of the given fd.
    /// Returns `(cols, rows)`.  Falls back to 80×24 on failure.
    pub fn terminal_size(fd: i32) -> (u16, u16) {
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
                (ws.ws_col, ws.ws_row)
            } else {
                (80, 24)
            }
        }
    }

    /// Save the current termios settings and switch to raw mode.
    /// Returns the original termios so it can be restored later.
    pub fn enable_raw_mode(fd: i32) -> io::Result<libc::termios> {
        unsafe {
            let mut orig: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut orig) != 0 {
                return Err(io::Error::last_os_error());
            }
            let mut raw = orig;
            libc::cfmakeraw(&mut raw);
            // Ensure we still get reads even on a single byte.
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            if libc::tcsetattr(fd, libc::TCSANOW, &raw) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(orig)
        }
    }

    /// Restore previously saved termios settings.
    pub fn restore_mode(fd: i32, orig: &libc::termios) {
        unsafe {
            libc::tcsetattr(fd, libc::TCSANOW, orig);
        }
    }

    /// Non-blocking read with a timeout (via `poll`).
    /// Returns `Some(n)` with the number of bytes read, or `None` on
    /// timeout / error / EOF.
    pub fn read_with_timeout(fd: i32, buf: &mut [u8], timeout_ms: i32) -> Option<usize> {
        unsafe {
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let ret = libc::poll(&mut pfd, 1, timeout_ms);
            if ret > 0 && (pfd.revents & libc::POLLIN) != 0 {
                let n = libc::read(fd, buf.as_mut_ptr().cast(), buf.len());
                if n > 0 { Some(n as usize) } else { None }
            } else {
                None
            }
        }
    }

    /// Write all bytes to an fd.
    pub fn write_all(fd: i32, data: &[u8]) {
        unsafe {
            let mut off = 0;
            while off < data.len() {
                let n = libc::write(fd, data[off..].as_ptr().cast(), data.len() - off);
                if n <= 0 {
                    break;
                }
                off += n as usize;
            }
        }
    }

    /// A guard that restores termios on drop (panic-safe cleanup).
    pub struct RawModeGuard {
        fd: i32,
        orig: libc::termios,
    }

    impl RawModeGuard {
        pub fn new(tty: &File) -> io::Result<Self> {
            let fd = tty.as_raw_fd();
            let orig = enable_raw_mode(fd)?;
            Ok(Self { fd, orig })
        }

        #[allow(dead_code)]
        pub fn fd(&self) -> i32 {
            self.fd
        }
    }

    impl Drop for RawModeGuard {
        fn drop(&mut self) {
            restore_mode(self.fd, &self.orig);
        }
    }
}

#[cfg(unix)]
impl PluginCommand for TrampShell {
    type Plugin = TrampPlugin;

    fn name(&self) -> &str {
        "tramp shell"
    }

    fn description(&self) -> &str {
        "Start an interactive remote shell session (requires RPC agent)"
    }

    fn extra_description(&self) -> &str {
        "Opens a full interactive terminal on the remote host using the \
         RPC agent's pseudo-terminal (PTY) support.  Keystrokes are forwarded \
         to the remote shell and its output is displayed locally.\n\n\
         The session ends when the remote shell exits (e.g. `exit`, Ctrl-D).\n\n\
         Requires the RPC agent to be deployed on the remote host (this \
         happens automatically for SSH and standalone Docker/K8s backends).\n\n\
         The terminal is put in raw mode for the duration of the session so \
         that control characters (Ctrl-C, etc.) are forwarded to the remote \
         rather than interpreted locally."
    }

    fn signature(&self) -> Signature {
        Signature::build(self.name())
            .required(
                "path",
                SyntaxShape::String,
                "TRAMP URI identifying the remote target (remote path is used as CWD)",
            )
            .named(
                "shell",
                SyntaxShape::String,
                "Shell program to run (default: $SHELL or /bin/sh)",
                Some('s'),
            )
            .input_output_types(vec![(Type::Nothing, Type::Nothing)])
            .category(Category::FileSystem)
    }

    fn search_terms(&self) -> Vec<&str> {
        vec![
            "tramp",
            "shell",
            "terminal",
            "pty",
            "interactive",
            "remote",
            "ssh",
        ]
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                example: "tramp shell /ssh:myvm:/",
                description: "Open an interactive shell on a remote host",
                result: None,
            },
            Example {
                example: "tramp shell /ssh:myvm:/app",
                description: "Open a shell with /app as the working directory",
                result: None,
            },
            Example {
                example: "tramp shell /ssh:myvm:/ --shell /bin/bash",
                description: "Open a bash shell on a remote host",
                result: None,
            },
            Example {
                example: "tramp shell /docker:mycontainer:/",
                description: "Open a shell inside a local Docker container",
                result: None,
            },
            Example {
                example: "tramp shell /ssh:myvm|docker:ctr:/",
                description: "Open a shell inside a Docker container on a remote host",
                result: None,
            },
        ]
    }

    #[allow(unused_variables, deprecated)]
    fn get_dynamic_completion(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: DynamicCompletionCall,
        arg_type: ArgType,
        _experimental: nu_protocol::engine::ExperimentalMarker,
    ) -> Option<Vec<DynamicSuggestion>> {
        complete_tramp_arg(plugin, call, arg_type, 0)
    }

    fn run(
        &self,
        plugin: &TrampPlugin,
        engine: &EngineInterface,
        call: &EvaluatedCall,
        _input: PipelineData,
    ) -> Result<PipelineData, LabeledError> {
        use std::os::unix::io::AsRawFd;

        apply_cache_ttl(engine, &plugin.vfs);
        let path = parse_tramp_arg(call, &plugin.remote_cwd)?;
        let span = call.head;

        // Ensure the backend supports PTY.
        let supports = plugin.vfs.supports_pty(&path);
        if !supports {
            return Err(
                LabeledError::new("PTY support is not available on this backend").with_label(
                    "requires the RPC agent (only available for SSH and standalone Docker/K8s)",
                    span,
                ),
            );
        }

        // Open /dev/tty for direct terminal access (bypasses plugin stdio).
        let tty = tty_raw::open_tty().map_err(|e| {
            LabeledError::new(format!("cannot open terminal (/dev/tty): {e}"))
                .with_label("not running in an interactive terminal?", span)
        })?;
        let tty_fd = tty.as_raw_fd();

        // Query terminal dimensions.
        let (cols, rows) = tty_raw::terminal_size(tty_fd);

        // Determine which shell to run.
        let shell: String = call
            .get_flag_value("shell")
            .and_then(|v| v.as_str().ok().map(|s| s.to_string()))
            .unwrap_or_else(|| {
                // Try to detect the remote default shell via $SHELL.
                if let Ok(result) = plugin.vfs.exec(&path, "sh", &["-c", "echo $SHELL"]) {
                    let s = String::from_utf8_lossy(&result.stdout).trim().to_string();
                    if !s.is_empty() && result.exit_code == 0 {
                        return s;
                    }
                }
                "/bin/sh".to_string()
            });

        // Build args: use -l for a login shell, and set CWD via `cd <dir> &&`.
        // Instead of passing CWD as an arg (not all shells support it), we
        // wrap in `sh -c 'cd <dir> && exec <shell> -l'` when CWD != "/".
        let remote_dir = &path.remote_path;
        let (program, args): (String, Vec<String>) = if remote_dir != "/" {
            (
                "sh".to_string(),
                vec![
                    "-c".to_string(),
                    format!(
                        "cd {} && exec {} -l",
                        shell_escape(remote_dir),
                        shell_escape(&shell)
                    ),
                ],
            )
        } else {
            (shell.clone(), vec!["-l".to_string()])
        };

        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        // Start the remote PTY.
        let pty_handle = plugin
            .vfs
            .pty_start(&path, &program, &arg_refs, rows, cols)
            .map_err(|e| tramp_err(e, span))?;
        let handle_id = pty_handle.handle;

        // Switch the local terminal to raw mode.  The guard restores it on
        // drop (including panics).
        let _raw_guard = tty_raw::RawModeGuard::new(&tty).map_err(|e| {
            // Kill the PTY before returning the error.
            let _ = plugin.vfs.pty_kill(&path, handle_id);
            LabeledError::new(format!("failed to set terminal to raw mode: {e}"))
        })?;

        // Shared flag: set to `false` when the remote process exits.
        let running = Arc::new(AtomicBool::new(true));

        // Clone what the input thread needs.
        let vfs_in = Arc::clone(&plugin.vfs);
        let path_in = path.clone();
        let running_in = Arc::clone(&running);

        // --- Input thread: /dev/tty → remote PTY ---
        let input_handle = std::thread::Builder::new()
            .name("tramp-shell-input".into())
            .spawn(move || {
                let mut buf = [0u8; 4096];
                while running_in.load(Ordering::Relaxed) {
                    // Poll the tty with a 50ms timeout so we can check the
                    // running flag periodically.
                    if let Some(n) = tty_raw::read_with_timeout(tty_fd, &mut buf, 50) {
                        let data = bytes::Bytes::copy_from_slice(&buf[..n]);
                        if vfs_in.pty_write(&path_in, handle_id, data).is_err() {
                            break;
                        }
                    }
                }
            })
            .map_err(|e| {
                let _ = plugin.vfs.pty_kill(&path, handle_id);
                LabeledError::new(format!("failed to spawn input thread: {e}"))
            })?;

        // --- Output loop (runs on the current thread): remote PTY → /dev/tty ---
        let mut prev_cols = cols;
        let mut prev_rows = rows;
        // Check for terminal resize every ~20 iterations (~100ms at 5ms sleep).
        let mut resize_counter: u32 = 0;

        loop {
            match plugin.vfs.pty_read(&path, handle_id) {
                Ok(result) => {
                    if !result.data.is_empty() {
                        tty_raw::write_all(tty_fd, &result.data);
                    }
                    if !result.running {
                        running.store(false, Ordering::Relaxed);
                        break;
                    }
                    // If no data was available, sleep briefly to avoid busy-loop.
                    if result.data.is_empty() {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
                Err(_) => {
                    // Communication error — treat as session end.
                    running.store(false, Ordering::Relaxed);
                    break;
                }
            }

            // Periodically check for terminal resize.
            resize_counter += 1;
            if resize_counter >= 20 {
                resize_counter = 0;
                let (new_cols, new_rows) = tty_raw::terminal_size(tty_fd);
                if new_cols != prev_cols || new_rows != prev_rows {
                    prev_cols = new_cols;
                    prev_rows = new_rows;
                    let _ = plugin.vfs.pty_resize(&path, handle_id, new_rows, new_cols);
                }
            }
        }

        // Signal the input thread to stop and wait for it.
        running.store(false, Ordering::Relaxed);
        let _ = input_handle.join();

        // Clean up the remote PTY (best-effort).
        let _ = plugin.vfs.pty_kill(&path, handle_id);

        // _raw_guard is dropped here, restoring the terminal.

        Ok(PipelineData::Value(Value::nothing(span), None))
    }
}

/// Escape a string for use inside a POSIX shell single-quoted argument.
///
/// Wraps the value in single quotes, replacing any embedded single quotes
/// with the `'\''` idiom (end quote, escaped quote, start quote).
#[cfg(unix)]
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    serve_plugin(&TrampPlugin::new(), MsgPackSerializer {})
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_relative_simple() {
        assert_eq!(resolve_relative("/app", "config.toml"), "/app/config.toml");
    }

    #[test]
    fn resolve_relative_subdir() {
        assert_eq!(
            resolve_relative("/app", "sub/file.txt"),
            "/app/sub/file.txt"
        );
    }

    #[test]
    fn resolve_relative_dotdot() {
        assert_eq!(resolve_relative("/app/sub", ".."), "/app");
    }

    #[test]
    fn resolve_relative_dotdot_and_file() {
        assert_eq!(
            resolve_relative("/app/sub", "../other/file.txt"),
            "/app/other/file.txt"
        );
    }

    #[test]
    fn resolve_relative_dot() {
        assert_eq!(
            resolve_relative("/app", "./config.toml"),
            "/app/config.toml"
        );
    }

    #[test]
    fn resolve_relative_absolute_overrides() {
        assert_eq!(resolve_relative("/app", "/etc/config"), "/etc/config");
    }

    #[test]
    fn resolve_relative_to_root() {
        assert_eq!(resolve_relative("/app", ".."), "/");
    }

    #[test]
    fn resolve_relative_past_root() {
        assert_eq!(resolve_relative("/", ".."), "/");
    }

    #[test]
    fn resolve_relative_trailing_slash_base() {
        assert_eq!(resolve_relative("/app/", "config.toml"), "/app/config.toml");
    }

    #[test]
    fn normalize_path_dots() {
        assert_eq!(normalize_path("/app/./sub/../file"), "/app/file");
    }

    #[test]
    fn normalize_path_root() {
        assert_eq!(normalize_path("/"), "/");
    }

    #[test]
    fn normalize_path_clean() {
        assert_eq!(normalize_path("/a/b/c"), "/a/b/c");
    }

    // -- parse_ttl_string ---------------------------------------------------

    #[test]
    fn ttl_integer_seconds() {
        let d = parse_ttl_string("5").unwrap();
        assert_eq!(d, Duration::from_secs(5));
    }

    #[test]
    fn ttl_float_seconds() {
        let d = parse_ttl_string("2.5").unwrap();
        assert_eq!(d, Duration::from_secs_f64(2.5));
    }

    #[test]
    fn ttl_zero() {
        let d = parse_ttl_string("0").unwrap();
        assert_eq!(d, Duration::from_secs(0));
    }

    #[test]
    fn ttl_with_s_suffix() {
        let d = parse_ttl_string("10s").unwrap();
        assert_eq!(d, Duration::from_secs(10));
    }

    #[test]
    fn ttl_with_sec_suffix() {
        let d = parse_ttl_string("3sec").unwrap();
        assert_eq!(d, Duration::from_secs(3));
    }

    #[test]
    fn ttl_with_ms_suffix() {
        let d = parse_ttl_string("500ms").unwrap();
        // 500ms = 0.5s
        assert!(d.as_millis() >= 499 && d.as_millis() <= 501);
    }

    #[test]
    fn ttl_empty_string() {
        assert!(parse_ttl_string("").is_none());
    }

    #[test]
    fn ttl_whitespace_only() {
        assert!(parse_ttl_string("   ").is_none());
    }

    #[test]
    fn ttl_invalid_string() {
        assert!(parse_ttl_string("abc").is_none());
    }

    #[test]
    fn ttl_negative_rejected() {
        assert!(parse_ttl_string("-1").is_none());
    }

    #[test]
    fn ttl_with_whitespace_padding() {
        let d = parse_ttl_string("  7  ").unwrap();
        assert_eq!(d, Duration::from_secs(7));
    }
}
