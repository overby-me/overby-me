//! Turn a reported wasm stack into source lines.
//!
//! The browser reports frames as a wasm URL plus a byte offset into the module —
//! `wasm-function[4231]:0x1d4c0` — because a wasm stack carries no names and no
//! source locations, whatever the compiler emitted. The mapping lives in DWARF,
//! which the frontend build keeps OUT of the shipped binary (it is ~20 MB) and
//! publishes beside it as `/symbols/<hash>.debug.wasm`.
//!
//! So the offsets arrive here meaningless, and this resolves them. Stripping only
//! removes trailing custom sections, so the code section keeps its offset in both
//! files — an offset from the shipped binary addresses the same instruction in
//! the sidecar. That equivalence is the whole trick; `split-symbols.nu` documents
//! the check that proves it.
//!
//! Both report paths use this: `/log` (every warn/error the app ships) and
//! `/feedback` (what the crash dialog's Report button sends).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};

/// Sidecar BYTES already fetched, by content hash. A build is immutable, so this
/// never goes stale.
///
/// The bytes are cached rather than the parsed DWARF because
/// `addr2line::Context` caches internally and is therefore neither `Send` nor
/// `Sync` — it cannot be shared between requests at all. Re-parsing per report is
/// the price, and a cheap one: reports are rare, and it is the fetch (tens of MB)
/// that actually costs.
///
/// A BTreeMap, not a HashMap: there is one entry per deployed build, so this
/// holds one or two keys in practice and hashing would buy nothing.
type Cache = Mutex<BTreeMap<String, Option<Arc<Vec<u8>>>>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Admits one symbolication at a time — see `resolve_stack` for why.
fn permit() -> &'static tokio::sync::Semaphore {
    static PERMIT: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    PERMIT.get_or_init(|| tokio::sync::Semaphore::new(1))
}

/// Rewrite every wasm frame in `stack` that can be resolved, leaving the rest
/// exactly as it arrived.
///
/// Being lenient is deliberate: Chrome, Firefox and Safari each render these
/// frames differently, and a shape none of them writes today must degrade to the
/// raw stack rather than lose it.
pub async fn resolve_stack(client: &reqwest::Client, app_origin: &str, stack: &str) -> String {
    let Some(hash) = wasm_hash(stack) else {
        return stack.to_string();
    };
    let Some(bytes) = sidecar_bytes(client, app_origin, &hash).await else {
        return stack.to_string();
    };
    let owned = stack.to_string();
    // One at a time. Measured, a single symbolication peaks around 41 MB — 25 of
    // it the shared sidecar, the rest per-parse state. The container has 256 MB
    // and accepts 50 concurrent requests, so a burst of reports doing this at
    // once is the one way this could exhaust it. Reports are rare enough that
    // serialising them costs nothing worth having.
    let _permit = permit().acquire().await.ok();
    // Parsing DWARF is CPU work and must not sit on the async runtime.
    let rewritten = tokio::task::spawn_blocking(move || rewrite(&bytes, &owned)).await;
    match rewritten {
        Ok(Some(text)) => text,
        _ => stack.to_string(),
    }
}

/// Rewrite the resolvable frames of `stack` against sidecar `bytes`. Synchronous
/// and self-contained, so it can run on a blocking thread.
fn rewrite(bytes: &[u8], stack: &str) -> Option<String> {
    use object::{Object, ObjectSection};
    let file = object::File::parse(bytes).ok()?;
    let load = |id: addr2line::gimli::SectionId| -> Result<_, ()> {
        // Borrowed only: a compressed section would have to outlive this
        // closure, and none of the DWARF wasm emits is compressed, so treat that
        // case as an absent section rather than leaking to satisfy a lifetime.
        let data = match file.section_by_name(id.name()).map(|s| s.data()) {
            Some(Ok(bytes)) => bytes,
            _ => &[],
        };
        Ok(addr2line::gimli::EndianSlice::new(
            data,
            addr2line::gimli::LittleEndian,
        ))
    };
    let dwarf = addr2line::gimli::Dwarf::load(load).ok()?;
    let context = addr2line::Context::from_dwarf(dwarf).ok()?;

    let mut out = String::with_capacity(stack.len());
    for line in stack.lines() {
        match offset_in(line).and_then(|offset| resolve_one(&context, offset)) {
            Some(resolved) => {
                out.push_str("    at ");
                out.push_str(&resolved);
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    Some(out)
}

/// `function (file:line)` for a code offset, or `None` when the address is not in
/// a mapped range — library code built without line tables, say.
fn resolve_one<R: addr2line::gimli::Reader>(
    context: &addr2line::Context<R>,
    offset: u64,
) -> Option<String> {
    let mut frames = context.find_frames(offset).skip_all_loads().ok()?;
    let frame = frames.next().ok()??;
    let function = frame
        .function
        .as_ref()
        .and_then(|f| f.demangle().ok())
        .map(|name| name.to_string());
    let location = frame.location.as_ref().and_then(|loc| {
        loc.file
            .map(|file| format!("{}:{}", trim_path(file), loc.line.unwrap_or(0)))
    });
    match (function, location) {
        (Some(f), Some(l)) => Some(format!("{f} ({l})")),
        (Some(f), None) => Some(f),
        (None, Some(l)) => Some(l),
        (None, None) => None,
    }
}

/// Keep the part of a path that identifies the file. Rust records absolute build
/// paths, and rustc's own sources arrive under `/rustc/<hash>/`; neither means
/// anything to whoever is reading the log.
fn trim_path(path: &str) -> String {
    if let Some(idx) = path.rfind("/src/") {
        return path[idx + 1..].to_string();
    }
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// Fetch a sidecar, remembering failures too so one missing build does not mean a
/// fetch per report.
async fn sidecar_bytes(
    client: &reqwest::Client,
    base_url: &str,
    hash: &str,
) -> Option<Arc<Vec<u8>>> {
    if let Some(hit) = cache().lock().ok().and_then(|c| c.get(hash).cloned()) {
        return hit;
    }
    let url = format!("{base_url}/symbols/{hash}.debug.wasm");
    let fetched = async {
        let resp = client.get(&url).send().await.ok()?;
        if !resp.status().is_success() {
            tracing::warn!("symbols {hash}: {} from {url}", resp.status());
            return None;
        }
        Some(Arc::new(resp.bytes().await.ok()?.to_vec()))
    }
    .await;
    if let Ok(mut c) = cache().lock() {
        c.insert(hash.to_string(), fetched.clone());
    }
    fetched
}

/// The content hash of the wasm a stack refers to, from the asset URL its frames
/// carry (`…/assets/wiki-dioxus_bg-dxhABC123.wasm`). That hash is dx's own and
/// names the sidecar, so no build-id section is needed.
fn wasm_hash(stack: &str) -> Option<String> {
    let idx = stack.find("wiki-dioxus_bg-")?;
    let rest = &stack[idx + "wiki-dioxus_bg-".len()..];
    let end = rest.find(".wasm")?;
    let hash = &rest[..end];
    hash.chars()
        .all(|c| c.is_ascii_alphanumeric())
        .then(|| hash.to_string())
}

/// The code offset in a frame, from the trailing `0x…` every engine appends.
fn offset_in(line: &str) -> Option<u64> {
    let idx = line.rfind("0x")?;
    let digits: String = line[idx + 2..]
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .collect();
    (!digits.is_empty())
        .then(|| u64::from_str_radix(&digits, 16).ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_build_hash_out_of_a_frame() {
        let stack = "at /assets/wiki-dioxus_bg-dxhABC123.wasm:wasm-function[42]:0x1d4c0";
        assert_eq!(wasm_hash(stack).as_deref(), Some("dxhABC123"));
    }

    #[test]
    fn a_stack_without_the_bundle_is_left_alone() {
        assert_eq!(wasm_hash("at foo.js:1:2"), None);
    }

    #[test]
    fn reads_the_trailing_offset() {
        assert_eq!(offset_in("wasm-function[42]:0x1d4c0"), Some(0x1d4c0));
        // Chrome names the module first; the offset is still last.
        assert_eq!(
            offset_in("at wasm://wasm/abc:wasm-function[7]:0x9c40"),
            Some(0x9c40)
        );
        assert_eq!(offset_in("at plain.js:10:5"), None);
    }

    #[test]
    fn build_paths_shrink_to_something_readable() {
        assert_eq!(
            trim_path("/home/me/Work/overby.me2/web/wiki/src/components/folder.rs"),
            "src/components/folder.rs"
        );
        // rustc's own sources have the same shape and trim the same way.
        assert_eq!(
            trim_path("/rustc/59807616/library/alloc/src/boxed.rs"),
            "src/boxed.rs"
        );
        assert_eq!(trim_path("weird"), "weird");
    }
}
