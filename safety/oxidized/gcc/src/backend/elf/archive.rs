//! Archive (.a) and linker script parsing.
//!
//! Handles both regular and thin GNU archives, plus GROUP/INPUT linker scripts.

// ── Archive (.a) parsing ─────────────────────────────────────────────────────

/// Returns true if `data` starts with the thin archive magic `!<thin>\n`.
pub fn is_thin_archive(data: &[u8]) -> bool {
    data.len() >= 8 && &data[0..8] == b"!<thin>\n"
}

/// Parse a GNU thin archive (.a file with `!<thin>\n` magic), returning member
/// filenames (relative to the archive directory). In thin archives, member data
/// is NOT stored inline — the archive only contains headers and a name table.
/// The caller must read each file from disk.
pub fn parse_thin_archive_members(data: &[u8]) -> Result<Vec<String>, String> {
    if data.len() < 8 || &data[0..8] != b"!<thin>\n" {
        return Err("not a thin archive file".to_string());
    }

    let mut members = Vec::new();
    let mut pos = 8;
    let mut extended_names: Option<&[u8]> = None;

    while pos + 60 <= data.len() {
        let name_raw = &data[pos..pos + 16];
        let size_str = std::str::from_utf8(&data[pos + 48..pos + 58])
            .unwrap_or("")
            .trim();
        let magic = &data[pos + 58..pos + 60];
        if magic != b"`\n" {
            break;
        }

        let size: usize = size_str.parse().unwrap_or(0);
        let data_start = pos + 60;
        let name_str = std::str::from_utf8(name_raw).unwrap_or("").trim_end();

        if name_str == "/" || name_str == "/SYM64/" {
            // Symbol table — data is stored inline even in thin archives
            pos = data_start + size;
            if pos % 2 != 0 {
                pos += 1;
            }
            continue;
        } else if name_str == "//" {
            // Extended name table — also stored inline in thin archives
            extended_names = Some(&data[data_start..(data_start + size).min(data.len())]);
            pos = data_start + size;
            if pos % 2 != 0 {
                pos += 1;
            }
            continue;
        }

        // Regular member — in thin archives, data is NOT inline
        let member_name = if let Some(rest) = name_str.strip_prefix('/') {
            if let Some(ext) = extended_names {
                // The name field is like "/23607" or "/23607/" — extract just the digits
                let num_str = rest.trim_end_matches('/').trim();
                let name_off: usize = num_str.parse().unwrap_or(0);
                if name_off < ext.len() {
                    // In thin archives, names can contain '/' (path separators),
                    // so the terminator is the two-byte sequence "/\n", not just '/'.
                    let slice = &ext[name_off..];
                    let end = slice
                        .windows(2)
                        .position(|w| w == b"/\n")
                        .unwrap_or_else(|| {
                            // Fall back to null byte or end of table
                            slice.iter().position(|&b| b == 0).unwrap_or(slice.len())
                        });
                    String::from_utf8_lossy(&ext[name_off..name_off + end]).to_string()
                } else {
                    name_str.to_string()
                }
            } else {
                name_str.to_string()
            }
        } else {
            name_str.trim_end_matches('/').to_string()
        };

        members.push(member_name);

        // In thin archives, member headers are consecutive (no inline data)
        pos = data_start;
        if pos % 2 != 0 {
            pos += 1;
        }
    }

    Ok(members)
}

/// Parse a GNU-format static archive (.a file), returning member entries as
/// `(name, data_offset, data_size)` tuples. The offsets are into the original
/// `data` slice, enabling zero-copy access.
///
/// Handles extended name tables (`//`), symbol tables (`/`, `/SYM64/`), and
/// 2-byte alignment padding between members.
pub fn parse_archive_members(data: &[u8]) -> Result<Vec<(String, usize, usize)>, String> {
    if data.len() < 8 || &data[0..8] != b"!<arch>\n" {
        return Err("not a valid archive file".to_string());
    }

    let mut members = Vec::new();
    let mut pos = 8;
    let mut extended_names: Option<&[u8]> = None;

    while pos + 60 <= data.len() {
        let name_raw = &data[pos..pos + 16];
        let size_str = std::str::from_utf8(&data[pos + 48..pos + 58])
            .unwrap_or("")
            .trim();
        let magic = &data[pos + 58..pos + 60];
        if magic != b"`\n" {
            break;
        }

        let size: usize = size_str.parse().unwrap_or(0);
        let data_start = pos + 60;
        let name_str = std::str::from_utf8(name_raw).unwrap_or("").trim_end();

        if name_str == "/" || name_str == "/SYM64/" {
            // Symbol table — skip
        } else if name_str == "//" {
            // Extended name table
            extended_names = Some(&data[data_start..(data_start + size).min(data.len())]);
        } else {
            let member_name = if let Some(rest) = name_str.strip_prefix('/') {
                // Extended name: /offset into extended names table
                if let Some(ext) = extended_names {
                    let name_off: usize = rest.trim_end_matches('/').parse().unwrap_or(0);
                    if name_off < ext.len() {
                        let end = ext[name_off..]
                            .iter()
                            .position(|&b| b == b'/' || b == b'\n' || b == 0)
                            .unwrap_or(ext.len() - name_off);
                        String::from_utf8_lossy(&ext[name_off..name_off + end]).to_string()
                    } else {
                        name_str.to_string()
                    }
                } else {
                    name_str.to_string()
                }
            } else {
                name_str.trim_end_matches('/').to_string()
            };

            if data_start + size <= data.len() {
                members.push((member_name, data_start, size));
            }
        }

        // Align to 2-byte boundary
        pos = data_start + size;
        if pos % 2 != 0 {
            pos += 1;
        }
    }

    Ok(members)
}

// ── Linker script parsing ────────────────────────────────────────────────────

/// An entry found in a GNU linker script directive (GROUP or INPUT).
#[derive(Debug, Clone)]
pub enum LinkerScriptEntry {
    /// An absolute or relative file path (e.g. `/lib/x86_64-linux-gnu/libc.so.6` or `libncurses.so.6`)
    Path(String),
    /// A `-l` library reference (e.g. `-ltinfo` becomes `tinfo`)
    Lib(String),
}

/// Parse a GNU linker script looking for `GROUP ( ... )` or `INPUT ( ... )` directives.
/// Returns the list of entries referenced, or `None` if no directive found.
///
/// Handles:
/// - `GROUP ( path1 path2 AS_NEEDED ( path3 ) )` - AS_NEEDED entries are skipped
/// - `INPUT ( libfoo.so.6 -lbar )` - both paths and `-l` library references
pub fn parse_linker_script(content: &str) -> Option<Vec<String>> {
    let entries = parse_linker_script_entries(content)?;
    let paths: Vec<String> = entries
        .into_iter()
        .filter_map(|e| match e {
            LinkerScriptEntry::Path(p) => Some(p),
            LinkerScriptEntry::Lib(_) => None,
        })
        .collect();
    if paths.is_empty() {
        None
    } else {
        Some(paths)
    }
}

/// Parse a GNU linker script, returning all entries including `-l` library references.
/// This is the full-featured version that callers with library search path access should use.
pub fn parse_linker_script_entries(content: &str) -> Option<Vec<LinkerScriptEntry>> {
    // Try GROUP first, then INPUT
    let directive_start = content.find("GROUP").or_else(|| content.find("INPUT"))?;

    let rest = &content[directive_start..];
    let paren_start = rest.find('(')?;

    // Find matching closing paren (handle nested parens for AS_NEEDED)
    let mut depth = 0;
    let mut paren_end = None;
    for (i, ch) in rest[paren_start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    paren_end = Some(paren_start + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let paren_end = paren_end?;
    let inside = &rest[paren_start + 1..paren_end];

    let mut entries = Vec::new();
    let mut in_as_needed = false;
    for token in inside.split_whitespace() {
        match token {
            "AS_NEEDED" => {
                in_as_needed = true;
                continue;
            }
            "(" => continue,
            ")" => {
                in_as_needed = false;
                continue;
            }
            _ => {}
        }
        if in_as_needed {
            continue;
        }

        if let Some(lib_name) = token.strip_prefix("-l") {
            // -ltinfo -> Lib("tinfo")
            if !lib_name.is_empty() {
                entries.push(LinkerScriptEntry::Lib(lib_name.to_string()));
            }
        } else if token.starts_with('/')
            || token.ends_with(".so")
            || token.ends_with(".a")
            || token.contains(".so.")
            || token.starts_with("lib")
        {
            entries.push(LinkerScriptEntry::Path(token.to_string()));
        }
    }

    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}
