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
        let resolved = offset_in(line)
            .map(|offset| resolve_all(&context, offset))
            .unwrap_or_default();
        if resolved.is_empty() {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        // Innermost first, as the engine orders frames.
        for frame in resolved {
            out.push_str("    at ");
            out.push_str(&frame);
            out.push('\n');
        }
    }
    Some(out)
}

/// Every `function (file:line)` at a code offset — the whole inlined chain, not
/// just the innermost.
///
/// This matters more than it sounds. At `opt-level = "s"` with fat LTO almost
/// everything small is inlined, so the innermost frame is usually a generic
/// helper — `Box::new`, `Option::copied` — while the frame that names the
/// component sits further out. Reporting only the first made resolved stacks
/// read as if the crash happened in the standard library.
///
/// Empty when the address is in no mapped range, which the caller treats as
/// "leave the raw frame alone".
fn resolve_all<R: addr2line::gimli::Reader>(
    context: &addr2line::Context<R>,
    offset: u64,
) -> Vec<String> {
    let Ok(mut frames) = context.find_frames(offset).skip_all_loads() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    while let Ok(Some(frame)) = frames.next() {
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
            (Some(f), Some(l)) => out.push(format!("{f} ({l})")),
            (Some(f), None) => out.push(f),
            (None, Some(l)) => out.push(l),
            (None, None) => {}
        }
    }
    out
}

/// Keep the part of a path that identifies the file, and says whose it is.
///
/// Rust records absolute build paths, none of which mean anything to whoever
/// reads the report. Trimming all of them to the trailing `src/…` was worse than
/// verbose though: `/rustc/<hash>/library/alloc/src/boxed.rs` came out as
/// `src/boxed.rs`, indistinguishable from a file in this repo, and a reader would
/// go looking for code that is not there. Standard library and dependency paths
/// now keep the crate that owns them.
fn trim_path(path: &str) -> String {
    // rustc's own sources: /rustc/<hash>/library/alloc/src/boxed.rs, or the same
    // relative when there was no compilation directory to join it to.
    if let Some(rest) = after(path, "/library/").or_else(|| path.strip_prefix("library/")) {
        return rest.to_string();
    }
    // A crate vendored into the compiler's own tree, which is where the hashbrown
    // behind every HashMap comes from: /rust/deps/hashbrown-0.16.1/src/raw/mod.rs.
    // Without this it trimmed to `src/raw/mod.rs` and read as a file in this
    // repo — the frontend then styled it as one, pointing at code that does not
    // exist.
    if let Some(rest) = after(path, "/rust/deps/") {
        return rest.to_string();
    }
    // A dependency:
    // …/registry/src/index.crates.io-<hash>/parking_lot-0.12.4/src/raw_mutex.rs
    // Checked before the `/src/` rule below, which such a path also matches.
    if let Some(rest) = after(path, "/registry/src/") {
        if let Some(slash) = rest.find('/') {
            // Past the registry-index directory, so it starts at <crate>-<version>.
            return rest[slash + 1..].to_string();
        }
    }
    // This repo: an absolute checkout path ending in src/….
    if let Some(idx) = path.rfind("/src/") {
        return path[idx + 1..].to_string();
    }
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// What follows `marker` in `path`, if it appears at all.
fn after<'a>(path: &'a str, marker: &str) -> Option<&'a str> {
    path.find(marker).map(|idx| &path[idx + marker.len()..])
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
        let bytes = resp.bytes().await.ok()?.to_vec();
        // A 200 does not mean the file exists. The site serves index.html for any
        // unknown path (the SPA fallback in `_redirects`), so a build with no
        // sidecar — anything from before this feature — answers with HTML. Parsing
        // that as wasm would fail somewhere deeper and look like a broken build
        // rather than a missing one, and the bytes would be cached as if good.
        if !bytes.starts_with(b"\0asm") {
            tracing::warn!(
                "symbols {hash}: not a wasm module ({} bytes) — no sidecar for that build",
                bytes.len()
            );
            return None;
        }
        Some(Arc::new(bytes))
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
        // The standard library keeps the crate that owns it, so it cannot be
        // mistaken for a file in this repo.
        assert_eq!(
            trim_path("/rustc/59807616/library/alloc/src/boxed.rs"),
            "alloc/src/boxed.rs"
        );
        // So does a dependency, minus the registry-index directory.
        assert_eq!(
            trim_path(
                "/home/me/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/\
                 parking_lot-0.12.4/src/raw_mutex.rs"
            ),
            "parking_lot-0.12.4/src/raw_mutex.rs"
        );
        // And a crate vendored into the compiler, which is the one that used to
        // masquerade as this repo: every HashMap operation resolves through
        // hashbrown, and `src/raw/mod.rs` is indistinguishable from our own.
        assert_eq!(
            trim_path("/rust/deps/hashbrown-0.16.1/src/raw/mod.rs"),
            "hashbrown-0.16.1/src/raw/mod.rs"
        );
        // A std path can arrive relative, with no compilation directory joined
        // to it. It must not fall through to the this-repo rule either.
        assert_eq!(
            trim_path("library/core/src/ptr/mod.rs"),
            "core/src/ptr/mod.rs"
        );
        assert_eq!(trim_path("weird"), "weird");
    }
}
