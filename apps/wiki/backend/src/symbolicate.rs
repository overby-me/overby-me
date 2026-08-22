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
    let hash = match wasm_hash(stack) {
        Some(hash) => hash,
        // Safari's wasm frames name neither the module nor an offset, so the
        // usual route to the sidecar is closed. Its JavaScript frames still
        // carry the glue's URL, and the glue names the wasm it loads — so the
        // build can be identified the long way round.
        None => match js_hash(stack) {
            Some(js) => match wasm_hash_via_glue(client, app_origin, &js).await {
                Some(hash) => hash,
                None => return stack.to_string(),
            },
            None => return stack.to_string(),
        },
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

    // Only built when a frame needs it, since it walks the whole module.
    let mut bodies: Option<Vec<u64>> = None;

    let mut out = String::with_capacity(stack.len());
    for line in stack.lines() {
        let offset = match frame_address(line) {
            Some(Address::Offset(offset)) => Some(offset),
            Some(Address::Function(index)) => {
                let bodies = bodies.get_or_insert_with(|| function_bodies(bytes));
                bodies.get(index as usize).copied().filter(|o| *o != 0)
            }
            None => None,
        };
        let mut resolved = offset
            .map(|offset| resolve_all(&context, offset))
            .unwrap_or_default();
        // A body begins with its locals declaration, which is not code and so is
        // not always in the line table. When that is why nothing resolved, step
        // over it to the first instruction and ask again — that is the address
        // Chrome and Firefox would have reported for the same frame.
        if resolved.is_empty() {
            if let (Some(body), Some(Address::Function(_))) = (offset, frame_address(line)) {
                let after_locals = first_instruction(bytes, body);
                if after_locals != body {
                    resolved = resolve_all(&context, after_locals);
                }
            }
        }
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

/// Where in the module a frame points, which is not the same question in every
/// browser.
#[derive(Debug, PartialEq)]
enum Address {
    /// A byte offset into the module, which Chrome and Firefox both report.
    Offset(u64),
    /// A function index, which is all Safari reports —
    /// `6175@wasm-function[6175]`, no offset anywhere in the line.
    Function(u32),
}

/// The start of every function body, by wasm function index.
///
/// Needed only for Safari. Its stacks name a function by index and give no
/// address, so there is nothing for DWARF to look up until the index is turned
/// back into an offset — which the module itself says, in the order its code
/// section lists the bodies.
///
/// Imported functions occupy the first indices and have no body, so the code
/// section's first entry is function number `imports`, not zero. The returned
/// vector is indexed by function index, with the imported range left at 0.
fn function_bodies(bytes: &[u8]) -> Vec<u64> {
    let mut imported = 0usize;
    let mut bodies = Vec::new();
    if !bytes.starts_with(b"\0asm") {
        return bodies;
    }
    // Past the magic and the version word.
    let mut pos = 8;
    while pos < bytes.len() {
        let Some(id) = bytes.get(pos).copied() else {
            break;
        };
        pos += 1;
        let Some(size) = uleb(bytes, &mut pos) else {
            break;
        };
        let start = pos;
        let Some(end) = start
            .checked_add(size as usize)
            .filter(|e| *e <= bytes.len())
        else {
            break;
        };
        match id {
            // Imports: counted, not read. Every entry has to be walked anyway,
            // because the count of imported FUNCTIONS is what the code section's
            // indices are offset by.
            2 => imported = count_imported_functions(&bytes[start..end]),
            // Code: one entry per defined function, each a size then a body.
            10 => {
                let mut p = start;
                if let Some(count) = uleb(bytes, &mut p) {
                    bodies = vec![0u64; imported];
                    bodies.reserve(count as usize);
                    for _ in 0..count {
                        let Some(body_size) = uleb(bytes, &mut p) else {
                            break;
                        };
                        bodies.push(p as u64);
                        p = p.saturating_add(body_size as usize);
                        if p > end {
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
        pos = end;
    }
    bodies
}

/// The offset of a function body's first instruction, given where the body
/// starts.
///
/// A body opens with its locals declaration — a count of groups, then each group
/// a count and a value type — which is data, not code, and so may sit outside the
/// line table. Returns `body` unchanged if the declaration cannot be read, which
/// just means the caller learns nothing new.
fn first_instruction(bytes: &[u8], body: u64) -> u64 {
    let mut pos = body as usize;
    let Some(groups) = uleb(bytes, &mut pos) else {
        return body;
    };
    for _ in 0..groups {
        if uleb(bytes, &mut pos).is_none() {
            return body;
        }
        pos += 1; // the value type
    }
    if pos <= bytes.len() {
        pos as u64
    } else {
        body
    }
}

/// How many of an import section's entries are functions.
fn count_imported_functions(section: &[u8]) -> usize {
    let mut pos = 0usize;
    let Some(count) = uleb(section, &mut pos) else {
        return 0;
    };
    let mut functions = 0usize;
    for _ in 0..count {
        // Module name and field name, each a length and that many bytes.
        for _ in 0..2 {
            let Some(len) = uleb(section, &mut pos) else {
                return functions;
            };
            pos = pos.saturating_add(len as usize);
        }
        let Some(kind) = section.get(pos).copied() else {
            return functions;
        };
        pos += 1;
        match kind {
            // func: a type index.
            0 => {
                functions += 1;
                if uleb(section, &mut pos).is_none() {
                    return functions;
                }
            }
            // table: a reference type, then limits.
            1 => {
                pos += 1;
                if !skip_limits(section, &mut pos) {
                    return functions;
                }
            }
            // memory: limits.
            2 => {
                if !skip_limits(section, &mut pos) {
                    return functions;
                }
            }
            // global: a value type and a mutability flag.
            3 => pos += 2,
            _ => return functions,
        }
    }
    functions
}

/// Step over a limits record: a flag byte, a minimum, and a maximum when the
/// flag says there is one.
fn skip_limits(section: &[u8], pos: &mut usize) -> bool {
    let Some(flags) = section.get(*pos).copied() else {
        return false;
    };
    *pos += 1;
    if uleb(section, pos).is_none() {
        return false;
    }
    if flags & 1 == 1 && uleb(section, pos).is_none() {
        return false;
    }
    true
}

/// Read one LEB128 unsigned integer, advancing `pos`.
fn uleb(bytes: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    let mut shift = 0u32;
    loop {
        let byte = *bytes.get(*pos)?;
        *pos += 1;
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
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
/// carry (`…/assets/wiki_bg-dxhABC123.wasm`). That hash is dx's own and
/// names the sidecar, so no build-id section is needed.
fn wasm_hash(stack: &str) -> Option<String> {
    hash_between(stack, "wiki_bg-", ".wasm")
}

/// The content hash of the JavaScript glue a stack mentions
/// (`…/assets/wiki-dxhABC.js`). Safari's route to the build: its wasm
/// frames carry no URL, but the frames on either side of them are in the glue.
fn js_hash(stack: &str) -> Option<String> {
    hash_between(stack, "wiki-", ".js")
}

/// The `<hash>` in `…<prefix><hash><suffix>…`, when it looks like one.
fn hash_between(stack: &str, prefix: &str, suffix: &str) -> Option<String> {
    let idx = stack.find(prefix)?;
    let rest = &stack[idx + prefix.len()..];
    let end = rest.find(suffix)?;
    let hash = &rest[..end];
    (!hash.is_empty() && hash.chars().all(|c| c.is_ascii_alphanumeric())).then(|| hash.to_string())
}

/// Ask the JavaScript glue which wasm module it loads.
///
/// dx writes the wasm's own hashed filename into the glue, so fetching the glue
/// named in a Safari stack yields the build that crashed.
///
/// Not cached, deliberately. The glue is a small file and the fetch worth
/// avoiding is the sidecar's tens of megabytes, which [`sidecar_bytes`] already
/// keeps. A second map keyed on a second hash would be more state to reason
/// about than the request it saves is worth.
async fn wasm_hash_via_glue(
    client: &reqwest::Client,
    base_url: &str,
    js_hash: &str,
) -> Option<String> {
    let url = format!("{base_url}/assets/wiki-{js_hash}.js");
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        tracing::warn!("glue {js_hash}: {} from {url}", resp.status());
        return None;
    }
    let body = resp.text().await.ok()?;
    let hash = wasm_hash(&body);
    if hash.is_none() {
        tracing::warn!("glue {js_hash}: names no wasm module");
    }
    hash
}

/// What a frame points at: an offset when the engine gives one, otherwise the
/// function index Safari names it by.
fn frame_address(line: &str) -> Option<Address> {
    // Chrome and Firefox both end a wasm frame with the offset.
    if let Some(idx) = line.rfind("0x") {
        let digits: String = line[idx + 2..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        if let Some(offset) = u64::from_str_radix(&digits, 16)
            .ok()
            .filter(|_| !digits.is_empty())
        {
            return Some(Address::Offset(offset));
        }
    }
    // Safari writes `6175@wasm-function[6175]` and stops there — no offset, and
    // no module URL either, which is why the sidecar has to be found by another
    // route (see `wasm_hash`).
    let idx = line.find("wasm-function[")?;
    let rest = &line[idx + "wasm-function[".len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok().map(Address::Function)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_build_hash_out_of_a_frame() {
        let stack = "at /assets/wiki_bg-dxhABC123.wasm:wasm-function[42]:0x1d4c0";
        assert_eq!(wasm_hash(stack).as_deref(), Some("dxhABC123"));
    }

    #[test]
    fn a_stack_without_the_bundle_is_left_alone() {
        assert_eq!(wasm_hash("at foo.js:1:2"), None);
    }

    #[test]
    fn reads_the_trailing_offset() {
        assert_eq!(
            frame_address("wasm-function[42]:0x1d4c0"),
            Some(Address::Offset(0x1d4c0))
        );
        // Chrome names the module first; the offset is still last.
        assert_eq!(
            frame_address("at wasm://wasm/abc:wasm-function[7]:0x9c40"),
            Some(Address::Offset(0x9c40))
        );
        assert_eq!(frame_address("at plain.js:10:5"), None);
    }

    #[test]
    fn a_safari_frame_gives_a_function_index_instead() {
        // JavaScriptCore writes the index twice and no offset at all.
        assert_eq!(
            frame_address("6175@wasm-function[6175]"),
            Some(Address::Function(6175))
        );
        assert_eq!(
            frame_address("522@wasm-function[522]"),
            Some(Address::Function(522))
        );
        // An offset still wins when there is one, so Chrome and Firefox keep
        // their exact call sites rather than dropping to a function start.
        assert_eq!(
            frame_address("@…_bg-dxh1.wasm:wasm-function[6140]:0x372e26"),
            Some(Address::Offset(0x372e26))
        );
    }

    #[test]
    fn the_glue_hash_is_read_from_a_safari_stack() {
        // The only build identifier in a Safari report: its wasm frames carry no
        // URL, but the JavaScript ones do.
        let stack = "@https://radikal.wiki/assets/wiki-dxhe4646b805d9c756.js:1:42944\n\
                     6175@wasm-function[6175]";
        assert_eq!(js_hash(stack).as_deref(), Some("dxhe4646b805d9c756"));
        assert_eq!(wasm_hash(stack), None, "there is no wasm URL to find");
    }

    #[test]
    fn function_indices_are_offset_by_the_imports() {
        // A module by hand, because the real one is 26 MB and the only thing
        // worth testing is the arithmetic: the code section's first body is
        // function number `imports`, not function zero. Get that wrong and every
        // Safari frame resolves to the wrong function, plausibly and silently.
        let mut wasm = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        // Import section: one function, then a memory (so the walk has to step
        // over a non-function entry without miscounting).
        let imports = [
            0x02, // two imports
            0x01, b'a', 0x01, b'b', 0x00, 0x00, // "a" "b" func, type 0
            0x01, b'a', 0x01, b'm', 0x02, 0x00, 0x01, // "a" "m" memory, min 1
        ];
        wasm.push(2);
        wasm.push(imports.len() as u8);
        wasm.extend_from_slice(&imports);
        // Code section: two bodies, each "no locals, end".
        let code = [0x02, 0x02, 0x00, 0x0b, 0x02, 0x00, 0x0b];
        wasm.push(10);
        wasm.push(code.len() as u8);
        wasm.extend_from_slice(&code);

        let bodies = function_bodies(&wasm);
        assert_eq!(bodies.len(), 3, "one import plus two defined functions");
        // The code section's payload begins at 26; count, then size, then body.
        assert_eq!(bodies[1], 28, "first defined function is index 1, not 0");
        assert_eq!(bodies[2], 31);
    }

    #[test]
    fn a_module_that_is_not_wasm_yields_no_functions() {
        assert!(function_bodies(b"<!DOCTYPE html>").is_empty());
    }

    /// The parser above against a real bundle, since a hand-built module proves
    /// the arithmetic but not that the section walk survives everything dx emits.
    /// Needs a built wasm, so it is not part of the normal run:
    ///
    /// ```text
    /// WIKI_WASM=target/dx/wiki/release/web/public/assets/wiki_bg-<hash>.wasm \
    ///   cargo test --  --ignored real_module
    /// ```
    ///
    /// Compare against `wasm-objdump -d <file> | grep 'func\[N\]:'`, which prints
    /// the same offsets.
    #[test]
    #[ignore = "needs a built wasm bundle; set WIKI_WASM"]
    fn real_module_offsets_match_wasm_objdump() {
        let Ok(path) = std::env::var("WIKI_WASM") else {
            panic!("set WIKI_WASM to a built wiki_bg-*.wasm");
        };
        let bytes = std::fs::read(&path).expect("read the wasm");
        let bodies = function_bodies(&bytes);
        assert!(
            bodies.len() > 1000,
            "expected a real module, got {}",
            bodies.len()
        );
        for (index, expected) in [
            (522usize, 0x16a5bbu64),
            (2548, 0x3206c7),
            (3577, 0x34ded6),
            (5243, 0x3720c2),
            (6175, 0x379afd),
            (7195, 0x37e239),
        ] {
            assert_eq!(
                bodies.get(index).copied(),
                Some(expected),
                "function {index} should start at 0x{expected:x}"
            );
        }
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
