use object::Object as _;
use object::ObjectSection as _;
use object::ObjectSymbol as _;
use object::read::elf::{ElfFile, FileHeader, ProgramHeader as _, SectionHeader as _};
use object::read::elf::{Rel as _, Rela as _};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::Path;
use std::process;
use std::time::SystemTime;

const VERSION: &str = "0.1.0";
const PKG: &str = "rust-binutils";

// ─── Multicall dispatch ───────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let argv0 = Path::new(&args[0])
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| args[0].clone());

    let (tool, tool_args) = if argv0 == "rust-binutils" {
        // Direct invocation: first arg is the tool name
        if args.len() < 2 {
            eprintln!("Usage: rust-binutils <tool> [args...]");
            eprintln!(
                "Tools: ar ranlib nm strings size readelf objdump objcopy strip addr2line c++filt as ld"
            );
            process::exit(1);
        }
        (args[1].as_str().to_owned(), args[2..].to_vec())
    } else {
        (argv0, args[1..].to_vec())
    };

    let code = match tool.as_str() {
        "ar" => tool_ar(&tool_args),
        "ranlib" => tool_ranlib(&tool_args),
        "nm" => tool_nm(&tool_args),
        "strings" => tool_strings(&tool_args),
        "size" => tool_size(&tool_args),
        "readelf" => tool_readelf(&tool_args),
        "objdump" => tool_objdump(&tool_args),
        "objcopy" => tool_objcopy(&tool_args),
        "strip" => tool_strip(&tool_args),
        "addr2line" => tool_addr2line(&tool_args),
        "c++filt" => tool_cxxfilt(&tool_args),
        "as" => tool_as(&tool_args),
        "ld" => tool_ld(&tool_args),
        _ => {
            eprintln!("rust-binutils: unknown tool '{tool}'");
            1
        }
    };
    process::exit(code);
}

fn version_string(tool: &str) -> String {
    format!("{tool} ({PKG}) {VERSION}")
}

fn check_version_help(tool: &str, args: &[String]) -> bool {
    for a in args {
        if a == "--version" || a == "-V" {
            println!("{}", version_string(tool));
            return true;
        }
        if a == "--help" || a == "-h" {
            println!("{}", version_string(tool));
            return true;
        }
    }
    false
}

// ─── AR ───────────────────────────────────────────────────────────────────────

const AR_MAGIC: &[u8] = b"!<arch>\n";
const AR_HDR_SIZE: usize = 60;
const AR_FMAG: &[u8] = b"`\n";

#[derive(Clone)]
struct ArMember {
    name: String,
    mtime: u64,
    uid: u32,
    gid: u32,
    mode: u32,
    data: Vec<u8>,
}

fn ar_pad_field(s: &str, width: usize) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.resize(width, b' ');
    v
}

fn ar_encode_header(
    name_field: &str,
    size: u64,
    mtime: u64,
    uid: u32,
    gid: u32,
    mode: u32,
) -> Vec<u8> {
    let mut hdr = Vec::with_capacity(AR_HDR_SIZE);
    hdr.extend_from_slice(&ar_pad_field(name_field, 16));
    hdr.extend_from_slice(&ar_pad_field(&mtime.to_string(), 12));
    hdr.extend_from_slice(&ar_pad_field(&uid.to_string(), 6));
    hdr.extend_from_slice(&ar_pad_field(&gid.to_string(), 6));
    hdr.extend_from_slice(&ar_pad_field(&format!("{mode:o}"), 8));
    hdr.extend_from_slice(&ar_pad_field(&size.to_string(), 10));
    hdr.extend_from_slice(AR_FMAG);
    hdr
}

fn ar_parse(data: &[u8]) -> Result<Vec<ArMember>, String> {
    if data.len() < 8 || &data[..8] != AR_MAGIC {
        return Err("not a valid archive".into());
    }
    let mut members = Vec::new();
    let mut pos = 8;
    let mut long_names = Vec::new();

    while pos + AR_HDR_SIZE <= data.len() {
        let hdr = &data[pos..pos + AR_HDR_SIZE];
        if &hdr[58..60] != AR_FMAG {
            return Err(format!("bad archive header at offset {pos}"));
        }

        let name_raw = std::str::from_utf8(&hdr[0..16])
            .map_err(|_| "invalid name")?
            .trim_end();
        let size: usize = std::str::from_utf8(&hdr[48..58])
            .map_err(|_| "invalid size")?
            .trim()
            .parse()
            .map_err(|_| "invalid size")?;
        let mtime: u64 = std::str::from_utf8(&hdr[16..28])
            .map_err(|_| "invalid mtime")?
            .trim()
            .parse()
            .unwrap_or(0);
        let uid: u32 = std::str::from_utf8(&hdr[28..34])
            .map_err(|_| "invalid uid")?
            .trim()
            .parse()
            .unwrap_or(0);
        let gid: u32 = std::str::from_utf8(&hdr[34..40])
            .map_err(|_| "invalid gid")?
            .trim()
            .parse()
            .unwrap_or(0);
        let mode: u32 = std::str::from_utf8(&hdr[40..48])
            .map_err(|_| "invalid mode")?
            .trim()
            .parse::<u32>()
            .ok()
            .or_else(|| {
                u32::from_str_radix(std::str::from_utf8(&hdr[40..48]).unwrap_or("").trim(), 8).ok()
            })
            .unwrap_or(0o100644);

        let member_data_start = pos + AR_HDR_SIZE;
        let member_data_end = member_data_start + size;
        if member_data_end > data.len() {
            return Err("truncated archive member".into());
        }
        let member_data = &data[member_data_start..member_data_end];

        if name_raw == "//" {
            // GNU long filename table
            long_names = member_data.to_vec();
        } else if name_raw == "/" {
            // Symbol table - skip during parsing, regenerated on write
        } else {
            let name = if name_raw.starts_with('/') && name_raw.len() > 1 {
                // GNU long name reference: /offset
                let offset: usize = name_raw[1..].parse().map_err(|_| "bad long name ref")?;
                let end = long_names[offset..]
                    .iter()
                    .position(|&b| b == b'/' || b == b'\n')
                    .map(|p| offset + p)
                    .unwrap_or(long_names.len());
                String::from_utf8_lossy(&long_names[offset..end]).into_owned()
            } else {
                name_raw.trim_end_matches('/').to_string()
            };

            members.push(ArMember {
                name,
                mtime,
                uid,
                gid,
                mode,
                data: member_data.to_vec(),
            });
        }

        pos = member_data_end;
        if pos % 2 != 0 {
            pos += 1; // pad to even
        }
    }

    Ok(members)
}

fn ar_build_symtab(members: &[ArMember], member_offsets: &[u32]) -> Vec<u8> {
    // Collect symbols from ELF object files
    let mut symbols: Vec<(u32, String)> = Vec::new(); // (member_offset, name)

    for (i, member) in members.iter().enumerate() {
        if let Ok(obj) = object::File::parse(&*member.data) {
            for sym in obj.symbols() {
                if sym.is_global()
                    && !sym.is_undefined()
                    && let Ok(name) = sym.name()
                    && !name.is_empty()
                {
                    symbols.push((member_offsets[i], name.to_string()));
                }
            }
            // Also check dynamic symbols
            for sym in obj.dynamic_symbols() {
                if sym.is_global()
                    && sym.is_definition()
                    && let Ok(name) = sym.name()
                    && !name.is_empty()
                {
                    symbols.push((member_offsets[i], name.to_string()));
                }
            }
        }
    }

    if symbols.is_empty() {
        return Vec::new();
    }

    let mut buf = Vec::new();
    // Big-endian count
    buf.extend_from_slice(&(symbols.len() as u32).to_be_bytes());
    // Big-endian offsets
    for (offset, _) in &symbols {
        buf.extend_from_slice(&offset.to_be_bytes());
    }
    // Null-terminated names
    for (_, name) in &symbols {
        buf.extend_from_slice(name.as_bytes());
        buf.push(0);
    }
    buf
}

fn ar_write(members: &[ArMember], with_symtab: bool) -> Vec<u8> {
    // First pass: determine if we need a long name table
    let mut long_name_table = Vec::new();
    let mut name_offsets: HashMap<usize, usize> = HashMap::new();

    for (i, m) in members.iter().enumerate() {
        if m.name.len() > 15 {
            let offset = long_name_table.len();
            name_offsets.insert(i, offset);
            long_name_table.extend_from_slice(m.name.as_bytes());
            long_name_table.extend_from_slice(b"/\n");
        }
    }

    // We need to compute member offsets to build the symbol table.
    // But the symbol table itself affects offsets. So we do two passes.
    let compute_offsets = |symtab_data: &[u8]| -> Vec<u32> {
        let mut offset: usize = 8; // after magic

        // Symbol table
        if !symtab_data.is_empty() {
            offset += AR_HDR_SIZE + symtab_data.len();
            if !offset.is_multiple_of(2) {
                offset += 1;
            }
        }

        // Long name table
        if !long_name_table.is_empty() {
            offset += AR_HDR_SIZE + long_name_table.len();
            if !offset.is_multiple_of(2) {
                offset += 1;
            }
        }

        let mut offsets = Vec::with_capacity(members.len());
        for m in members {
            offsets.push(offset as u32);
            offset += AR_HDR_SIZE + m.data.len();
            if !offset.is_multiple_of(2) {
                offset += 1;
            }
        }
        offsets
    };

    // First pass with empty symtab to get approximate offsets
    let mut symtab_data = Vec::new();
    if with_symtab {
        let offsets = compute_offsets(&[]);
        let trial_symtab = ar_build_symtab(members, &offsets);
        // Recompute with actual symtab size
        let offsets = compute_offsets(&trial_symtab);
        symtab_data = ar_build_symtab(members, &offsets);
        // Verify offsets are stable (they should be since symtab size didn't change names)
        let final_offsets = compute_offsets(&symtab_data);
        if final_offsets != offsets {
            // One more iteration
            symtab_data = ar_build_symtab(members, &final_offsets);
        }
    }

    // Now write the archive
    let mut out = Vec::new();
    out.extend_from_slice(AR_MAGIC);

    // Symbol table member
    if !symtab_data.is_empty() {
        out.extend_from_slice(&ar_encode_header("/", symtab_data.len() as u64, 0, 0, 0, 0));
        out.extend_from_slice(&symtab_data);
        if out.len() % 2 != 0 {
            out.push(b'\n');
        }
    }

    // Long name table member
    if !long_name_table.is_empty() {
        out.extend_from_slice(&ar_encode_header(
            "//",
            long_name_table.len() as u64,
            0,
            0,
            0,
            0,
        ));
        out.extend_from_slice(&long_name_table);
        if out.len() % 2 != 0 {
            out.push(b'\n');
        }
    }

    // Members
    for (i, m) in members.iter().enumerate() {
        let name_field = if let Some(&offset) = name_offsets.get(&i) {
            format!("/{offset}")
        } else if m.name.len() > 15 {
            // Shouldn't happen, but fallback
            format!("/{}", name_offsets.get(&i).copied().unwrap_or(0))
        } else {
            format!("{}/", m.name)
        };

        out.extend_from_slice(&ar_encode_header(
            &name_field,
            m.data.len() as u64,
            m.mtime,
            m.uid,
            m.gid,
            m.mode,
        ));
        out.extend_from_slice(&m.data);
        if out.len() % 2 != 0 {
            out.push(b'\n');
        }
    }

    out
}

fn tool_ar(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("ar: no operation specified");
        eprintln!("Usage: ar [OPERATION][MODIFIERS] ARCHIVE FILE...");
        return 1;
    }
    if check_version_help("ar", args) {
        return 0;
    }

    // Parse the operation key (first non-dash argument, or first char of combined arg)
    let mut op = ' ';
    let mut create = false;
    let mut symtab = false;
    let mut update_only = false;
    let mut verbose = false;
    // First arg is the operation+modifiers key
    let key = if args[0].starts_with('-') {
        &args[0][1..]
    } else {
        args[0].as_str()
    };
    for ch in key.chars() {
        match ch {
            'r' | 't' | 'x' | 'd' | 'q' | 'p' => {
                if op == ' ' {
                    op = ch;
                }
            }
            'c' => create = true,
            's' => symtab = true,
            'u' => update_only = true,
            'v' => verbose = true,
            'D' => {} // deterministic mode, ignore
            'U' => {} // non-deterministic mode, ignore
            'T' => {} // thin archive, ignore
            _ => {}
        }
    }
    let remaining_args: Vec<String> = args[1..].to_vec();

    if op == ' ' && symtab {
        // Just `ar s archive` means ranlib
        if remaining_args.is_empty() {
            eprintln!("ar: no archive specified");
            return 1;
        }
        let archive = &remaining_args[0];
        return ranlib_file(archive);
    }

    if op == ' ' {
        eprintln!("ar: no operation specified");
        return 1;
    }

    if remaining_args.is_empty() {
        eprintln!("ar: no archive specified");
        return 1;
    }

    let archive_path = &remaining_args[0];
    let file_args = &remaining_args[1..];

    match op {
        'r' => ar_op_replace(
            archive_path,
            file_args,
            create,
            symtab,
            update_only,
            verbose,
        ),
        'q' => ar_op_quick_append(archive_path, file_args, create, symtab, verbose),
        't' => ar_op_list(archive_path, verbose),
        'x' => ar_op_extract(archive_path, file_args, verbose),
        'd' => ar_op_delete(archive_path, file_args, symtab, verbose),
        'p' => ar_op_print(archive_path, file_args),
        _ => {
            eprintln!("ar: unsupported operation '{op}'");
            1
        }
    }
}

fn ar_op_replace(
    archive: &str,
    files: &[String],
    create: bool,
    _with_symtab: bool,
    update_only: bool,
    verbose: bool,
) -> i32 {
    let mut members = if Path::new(archive).exists() {
        match fs::read(archive) {
            Ok(data) => match ar_parse(&data) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("ar: {archive}: {e}");
                    return 1;
                }
            },
            Err(e) => {
                eprintln!("ar: {archive}: {e}");
                return 1;
            }
        }
    } else if create || !files.is_empty() {
        if !create {
            eprintln!("ar: creating {archive}");
        }
        Vec::new()
    } else {
        eprintln!("ar: {archive}: No such file or directory");
        return 1;
    };

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    for f in files {
        let path = Path::new(f);
        let data = match fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("ar: {f}: {e}");
                return 1;
            }
        };

        let fname = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| f.clone());

        let mtime = fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(now);

        let meta = fs::metadata(path).ok();
        let mode = meta.as_ref().map(|_| 0o100644u32).unwrap_or(0o100644);

        if let Some(existing) = members.iter_mut().find(|m| m.name == fname) {
            if update_only && mtime <= existing.mtime {
                continue;
            }
            if verbose {
                eprintln!("r - {fname}");
            }
            existing.data = data;
            existing.mtime = mtime;
            existing.mode = mode;
        } else {
            if verbose {
                eprintln!("a - {fname}");
            }
            members.push(ArMember {
                name: fname,
                mtime,
                uid: 0,
                gid: 0,
                mode,
                data,
            });
        }
    }

    let out = ar_write(&members, true); // always write symtab
    if let Err(e) = fs::write(archive, &out) {
        eprintln!("ar: {archive}: {e}");
        return 1;
    }
    0
}

fn ar_op_quick_append(
    archive: &str,
    files: &[String],
    create: bool,
    _with_symtab: bool,
    verbose: bool,
) -> i32 {
    let mut members = if Path::new(archive).exists() {
        match fs::read(archive) {
            Ok(data) => match ar_parse(&data) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("ar: {archive}: {e}");
                    return 1;
                }
            },
            Err(e) => {
                eprintln!("ar: {archive}: {e}");
                return 1;
            }
        }
    } else {
        if !create {
            eprintln!("ar: creating {archive}");
        }
        Vec::new()
    };

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    for f in files {
        let path = Path::new(f);
        let data = match fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("ar: {f}: {e}");
                return 1;
            }
        };
        let fname = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| f.clone());
        let mtime = fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(now);
        if verbose {
            eprintln!("a - {fname}");
        }
        members.push(ArMember {
            name: fname,
            mtime,
            uid: 0,
            gid: 0,
            mode: 0o100644,
            data,
        });
    }

    let out = ar_write(&members, true);
    if let Err(e) = fs::write(archive, &out) {
        eprintln!("ar: {archive}: {e}");
        return 1;
    }
    0
}

fn ar_op_list(archive: &str, verbose: bool) -> i32 {
    let data = match fs::read(archive) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("ar: {archive}: {e}");
            return 1;
        }
    };
    let members = match ar_parse(&data) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("ar: {archive}: {e}");
            return 1;
        }
    };
    for m in &members {
        if verbose {
            println!(
                "{:o} {:>5}/{:<5} {:>8} {} {}",
                m.mode,
                m.uid,
                m.gid,
                m.data.len(),
                m.mtime,
                m.name
            );
        } else {
            println!("{}", m.name);
        }
    }
    0
}

fn ar_op_extract(archive: &str, files: &[String], verbose: bool) -> i32 {
    let data = match fs::read(archive) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("ar: {archive}: {e}");
            return 1;
        }
    };
    let members = match ar_parse(&data) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("ar: {archive}: {e}");
            return 1;
        }
    };
    let extract_all = files.is_empty();
    for m in &members {
        if extract_all || files.iter().any(|f| f == &m.name) {
            if verbose {
                eprintln!("x - {}", m.name);
            }
            if let Err(e) = fs::write(&m.name, &m.data) {
                eprintln!("ar: {}: {e}", m.name);
                return 1;
            }
        }
    }
    0
}

fn ar_op_delete(archive: &str, files: &[String], _with_symtab: bool, verbose: bool) -> i32 {
    let data = match fs::read(archive) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("ar: {archive}: {e}");
            return 1;
        }
    };
    let mut members = match ar_parse(&data) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("ar: {archive}: {e}");
            return 1;
        }
    };
    members.retain(|m| {
        if files.iter().any(|f| f == &m.name) {
            if verbose {
                eprintln!("d - {}", m.name);
            }
            false
        } else {
            true
        }
    });
    let out = ar_write(&members, true);
    if let Err(e) = fs::write(archive, &out) {
        eprintln!("ar: {archive}: {e}");
        return 1;
    }
    0
}

fn ar_op_print(archive: &str, files: &[String]) -> i32 {
    let data = match fs::read(archive) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("ar: {archive}: {e}");
            return 1;
        }
    };
    let members = match ar_parse(&data) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("ar: {archive}: {e}");
            return 1;
        }
    };
    let print_all = files.is_empty();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    for m in &members {
        if print_all || files.iter().any(|f| f == &m.name) {
            let _ = out.write_all(&m.data);
        }
    }
    0
}

// ─── RANLIB ───────────────────────────────────────────────────────────────────

fn ranlib_file(archive: &str) -> i32 {
    let data = match fs::read(archive) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("ranlib: {archive}: {e}");
            return 1;
        }
    };
    let members = match ar_parse(&data) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("ranlib: {archive}: {e}");
            return 1;
        }
    };
    let out = ar_write(&members, true);
    if let Err(e) = fs::write(archive, &out) {
        eprintln!("ranlib: {archive}: {e}");
        return 1;
    }
    0
}

fn tool_ranlib(args: &[String]) -> i32 {
    if check_version_help("ranlib", args) {
        return 0;
    }
    if args.is_empty() {
        eprintln!("ranlib: no archive specified");
        return 1;
    }
    let mut errors = 0;
    for a in args {
        if a.starts_with('-') {
            continue; // skip flags
        }
        errors += ranlib_file(a);
    }
    errors
}

// ─── NM ───────────────────────────────────────────────────────────────────────

fn tool_nm(args: &[String]) -> i32 {
    if check_version_help("nm", args) {
        return 0;
    }

    let mut extern_only = false;
    let mut undefined_only = false;
    let mut dynamic = false;
    let mut no_sort = false;
    let mut show_filename = false;
    let mut files: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-g" | "--extern-only" => extern_only = true,
            "-u" | "--undefined-only" => undefined_only = true,
            "-D" | "--dynamic" => dynamic = true,
            "-p" | "--no-sort" => no_sort = true,
            "-A" | "-o" | "--print-file-name" => show_filename = true,
            _ if arg.starts_with('-') && !arg.starts_with("--") => {
                for ch in arg[1..].chars() {
                    match ch {
                        'g' => extern_only = true,
                        'u' => undefined_only = true,
                        'D' => dynamic = true,
                        'p' => no_sort = true,
                        'A' | 'o' => show_filename = true,
                        _ => {}
                    }
                }
            }
            _ => files.push(arg.clone()),
        }
        i += 1;
    }

    if files.is_empty() {
        files.push("a.out".into());
    }

    let multi = files.len() > 1;
    let mut errors = 0;

    for file in &files {
        let data = match fs::read(file) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("nm: '{file}': {e}");
                errors += 1;
                continue;
            }
        };

        // Check if it's an archive
        if data.starts_with(b"!<arch>\n") {
            // Parse archive members and nm each one
            let members = parse_archive_members(&data);
            for (name, member_data) in &members {
                if name == "/" || name == "//" || name.is_empty() {
                    continue; // skip symbol table and long name table
                }
                let display_name = format!("{file}({name})");
                let obj = match object::File::parse(&**member_data) {
                    Ok(o) => o,
                    Err(_) => continue,
                };
                println!("\n{display_name}:");
                nm_print_symbols(
                    &obj,
                    &display_name,
                    extern_only,
                    undefined_only,
                    dynamic,
                    no_sort,
                    show_filename,
                );
            }
            continue;
        }

        let obj = match object::File::parse(&*data) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("nm: {file}: Unsupported file format");
                let _ = e;
                errors += 1;
                continue;
            }
        };

        if multi && !show_filename {
            println!("\n{file}:");
        }

        let symbols: Box<dyn Iterator<Item = object::read::Symbol<'_, '_>>> = if dynamic {
            Box::new(obj.dynamic_symbols())
        } else {
            Box::new(obj.symbols())
        };

        let mut syms: Vec<(u64, char, String)> = Vec::new();

        for sym in symbols {
            let name = sym.name().unwrap_or("");
            if name.is_empty() && sym.kind() == object::SymbolKind::Unknown {
                continue;
            }
            if extern_only && !sym.is_global() {
                continue;
            }
            if undefined_only && !sym.is_undefined() {
                continue;
            }

            let type_char = nm_type_char(&sym);
            let addr = sym.address();
            syms.push((addr, type_char, name.to_string()));
        }

        if !no_sort {
            syms.sort_by(|a, b| a.2.cmp(&b.2));
        }

        for (addr, ty, name) in &syms {
            let prefix = if show_filename {
                format!("{file}:")
            } else {
                String::new()
            };
            if *ty == 'U' || *ty == 'w' {
                println!("{prefix}{:>16} {ty} {name}", "");
            } else {
                println!("{prefix}{addr:016x} {ty} {name}");
            }
        }
    }

    if errors > 0 { 1 } else { 0 }
}

fn nm_print_symbols(
    obj: &object::File,
    display_name: &str,
    extern_only: bool,
    undefined_only: bool,
    dynamic: bool,
    no_sort: bool,
    show_filename: bool,
) {
    use object::Object as _;
    use object::ObjectSymbol as _;

    let symbols: Box<dyn Iterator<Item = object::read::Symbol<'_, '_>>> = if dynamic {
        Box::new(obj.dynamic_symbols())
    } else {
        Box::new(obj.symbols())
    };

    let mut syms: Vec<(u64, char, String)> = Vec::new();
    for sym in symbols {
        let name = sym.name().unwrap_or("");
        if name.is_empty() && sym.kind() == object::SymbolKind::Unknown {
            continue;
        }
        if extern_only && !sym.is_global() {
            continue;
        }
        if undefined_only && !sym.is_undefined() {
            continue;
        }
        syms.push((sym.address(), nm_type_char(&sym), name.to_string()));
    }
    if !no_sort {
        syms.sort_by(|a, b| a.2.cmp(&b.2));
    }
    for (addr, ty, name) in &syms {
        let prefix = if show_filename {
            format!("{display_name}:")
        } else {
            String::new()
        };
        if *ty == 'U' || *ty == 'w' {
            println!("{prefix}{:>16} {ty} {name}", "");
        } else {
            println!("{prefix}{addr:016x} {ty} {name}");
        }
    }
}

/// Parse archive members from raw archive data.
/// Returns Vec of (name, data) for each member.
fn parse_archive_members(data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut members = Vec::new();
    if data.len() < 8 || &data[..8] != b"!<arch>\n" {
        return members;
    }
    let mut pos = 8;
    let mut long_names = String::new();

    while pos + 60 <= data.len() {
        let header = &data[pos..pos + 60];
        let name_field = std::str::from_utf8(&header[0..16]).unwrap_or("").trim_end();
        let size_str = std::str::from_utf8(&header[48..58]).unwrap_or("0").trim();
        let size: usize = size_str.parse().unwrap_or(0);
        pos += 60;

        if pos + size > data.len() {
            break;
        }
        let member_data = &data[pos..pos + size];

        let name = if name_field == "//" {
            long_names = String::from_utf8_lossy(member_data).to_string();
            pos += size;
            if pos % 2 != 0 {
                pos += 1;
            }
            members.push(("//".to_string(), member_data.to_vec()));
            continue;
        } else if name_field == "/" {
            pos += size;
            if pos % 2 != 0 {
                pos += 1;
            }
            members.push(("/".to_string(), member_data.to_vec()));
            continue;
        } else if let Some(idx_str) = name_field.strip_prefix('/') {
            let idx: usize = idx_str.trim_end_matches('/').parse().unwrap_or(0);
            if idx < long_names.len() {
                let end = long_names[idx..]
                    .find('/')
                    .map(|p| idx + p)
                    .unwrap_or(long_names.len());
                long_names[idx..end].to_string()
            } else {
                name_field.trim_end_matches('/').to_string()
            }
        } else {
            name_field.trim_end_matches('/').to_string()
        };

        members.push((name, member_data.to_vec()));
        pos += size;
        if pos % 2 != 0 {
            pos += 1;
        }
    }
    members
}

fn nm_type_char(sym: &object::read::Symbol<'_, '_>) -> char {
    let is_global = sym.is_global();

    if sym.is_undefined() {
        return if sym.is_weak() { 'w' } else { 'U' };
    }

    if sym.is_common() {
        return 'C';
    }

    let section_char = match sym.section() {
        object::SymbolSection::Section(idx) => {
            // Try to determine type from section name via the symbol's section_index
            // We can't access the section directly here easily, so use kind-based heuristic
            let _ = idx;
            match sym.kind() {
                object::SymbolKind::Text => 't',
                object::SymbolKind::Data => 'd',
                _ => '?',
            }
        }
        object::SymbolSection::Absolute => 'a',
        _ => '?',
    };

    // Try harder with symbol kind
    let c = match sym.kind() {
        object::SymbolKind::Text => 't',
        object::SymbolKind::Data => 'd',
        object::SymbolKind::Tls => 'd',
        _ => section_char,
    };

    if is_global { c.to_ascii_uppercase() } else { c }
}

// ─── STRINGS ──────────────────────────────────────────────────────────────────

fn tool_strings(args: &[String]) -> i32 {
    if check_version_help("strings", args) {
        return 0;
    }

    let mut min_len: usize = 4;
    let mut offset_format: Option<char> = None;
    let mut files: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-a" | "--all" | "-" => {} // scan all is default
            "-n" | "--bytes" => {
                i += 1;
                if i < args.len() {
                    min_len = args[i].parse().unwrap_or(4);
                }
            }
            "-t" | "--radix" => {
                i += 1;
                if i < args.len() {
                    offset_format = args[i].chars().next();
                }
            }
            _ if arg.starts_with("-n") => {
                min_len = arg[2..].parse().unwrap_or(4);
            }
            _ if arg.starts_with("-t") => {
                offset_format = arg.chars().nth(2);
            }
            _ if arg.starts_with("--bytes=") => {
                min_len = arg[8..].parse().unwrap_or(4);
            }
            _ if arg.starts_with("--radix=") => {
                offset_format = arg.chars().nth(8);
            }
            _ if !arg.starts_with('-') => {
                files.push(arg.clone());
            }
            _ => {}
        }
        i += 1;
    }

    if files.is_empty() {
        // Read from stdin
        let mut data = Vec::new();
        let _ = io::stdin().lock().read_to_end(&mut data);
        strings_scan(&data, min_len, offset_format);
        return 0;
    }

    let mut errors = 0;
    for file in &files {
        let data = match fs::read(file) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("strings: {file}: {e}");
                errors += 1;
                continue;
            }
        };
        strings_scan(&data, min_len, offset_format);
    }

    if errors > 0 { 1 } else { 0 }
}

fn strings_scan(data: &[u8], min_len: usize, offset_format: Option<char>) {
    let mut current = Vec::new();
    let mut start_offset = 0;
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for (i, &b) in data.iter().enumerate() {
        if (0x20..0x7f).contains(&b) {
            if current.is_empty() {
                start_offset = i;
            }
            current.push(b);
        } else {
            if current.len() >= min_len {
                if let Some(fmt) = offset_format {
                    match fmt {
                        'd' => {
                            let _ = write!(out, "{:>7} ", start_offset);
                        }
                        'o' => {
                            let _ = write!(out, "{:>7o} ", start_offset);
                        }
                        'x' => {
                            let _ = write!(out, "{:>7x} ", start_offset);
                        }
                        _ => {}
                    }
                }
                let _ = out.write_all(&current);
                let _ = out.write_all(b"\n");
            }
            current.clear();
        }
    }
    // Flush remaining
    if current.len() >= min_len {
        if let Some(fmt) = offset_format {
            match fmt {
                'd' => {
                    let _ = write!(out, "{:>7} ", start_offset);
                }
                'o' => {
                    let _ = write!(out, "{:>7o} ", start_offset);
                }
                'x' => {
                    let _ = write!(out, "{:>7x} ", start_offset);
                }
                _ => {}
            }
        }
        let _ = out.write_all(&current);
        let _ = out.write_all(b"\n");
    }
}

// ─── SIZE ─────────────────────────────────────────────────────────────────────

fn tool_size(args: &[String]) -> i32 {
    if check_version_help("size", args) {
        return 0;
    }

    let mut sysv_format = false;
    let mut show_totals = false;
    let mut files: Vec<String> = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-A" | "--format=sysv" => sysv_format = true,
            "-B" | "--format=berkeley" => sysv_format = false,
            "-t" | "--totals" => show_totals = true,
            _ if !arg.starts_with('-') => files.push(arg.clone()),
            _ => {}
        }
    }

    if files.is_empty() {
        files.push("a.out".into());
    }

    let mut total_text: u64 = 0;
    let mut total_data: u64 = 0;
    let mut total_bss: u64 = 0;
    let mut errors = 0;

    if !sysv_format {
        println!("   text\t   data\t    bss\t    dec\t    hex\tfilename");
    }

    for file in &files {
        let data = match fs::read(file) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("size: {file}: {e}");
                errors += 1;
                continue;
            }
        };
        let obj = match object::File::parse(&*data) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("size: {file}: {e}");
                errors += 1;
                continue;
            }
        };

        if sysv_format {
            println!("{file}  :");
            println!("section             size      addr");
            let mut total: u64 = 0;
            for section in obj.sections() {
                let name = section.name().unwrap_or("");
                if name.is_empty() {
                    continue;
                }
                let sz = section.size();
                let addr = section.address();
                println!("{name:<20}{sz:<10}{addr:<10}");
                total += sz;
            }
            println!("Total               {total}");
            println!();
        } else {
            let mut text: u64 = 0;
            let mut data_size: u64 = 0;
            let mut bss: u64 = 0;
            for section in obj.sections() {
                let name = section.name().unwrap_or("");
                let sz = section.size();
                if name == ".text"
                    || name.starts_with(".text.")
                    || name == ".init"
                    || name == ".fini"
                    || name == ".rodata"
                    || name.starts_with(".rodata.")
                {
                    text += sz;
                } else if name == ".bss" || name.starts_with(".bss.") || name == ".tbss" {
                    bss += sz;
                } else if name == ".data"
                    || name.starts_with(".data.")
                    || name == ".tdata"
                    || name == ".got"
                    || name == ".got.plt"
                {
                    data_size += sz;
                }
            }
            let dec = text + data_size + bss;
            println!("{text:>7}\t{data_size:>7}\t{bss:>7}\t{dec:>7}\t{dec:>7x}\t{file}");
            total_text += text;
            total_data += data_size;
            total_bss += bss;
        }
    }

    if show_totals && !sysv_format {
        let dec = total_text + total_data + total_bss;
        println!("{total_text:>7}\t{total_data:>7}\t{total_bss:>7}\t{dec:>7}\t{dec:>7x}\t(TOTALS)");
    }

    if errors > 0 { 1 } else { 0 }
}

// ─── READELF ──────────────────────────────────────────────────────────────────

fn tool_readelf(args: &[String]) -> i32 {
    if check_version_help("readelf", args) {
        return 0;
    }

    let mut show_header = false;
    let mut show_sections = false;
    let mut show_program_headers = false;
    let mut show_symbols = false;
    let mut show_dynamic = false;
    let mut show_relocs = false;
    let mut wide = false;
    let mut files: Vec<String> = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-a" | "--all" => {
                show_header = true;
                show_sections = true;
                show_program_headers = true;
                show_symbols = true;
                show_dynamic = true;
                show_relocs = true;
            }
            "-h" | "--file-header" => show_header = true,
            "-S" | "--section-headers" | "--sections" => show_sections = true,
            "-l" | "--program-headers" | "--segments" => show_program_headers = true,
            "-s" | "--syms" | "--symbols" => show_symbols = true,
            "-d" | "--dynamic" => show_dynamic = true,
            "-r" | "--relocs" => show_relocs = true,
            "-W" | "--wide" => wide = true,
            _ if arg.starts_with('-') && !arg.starts_with("--") => {
                for ch in arg[1..].chars() {
                    match ch {
                        'a' => {
                            show_header = true;
                            show_sections = true;
                            show_program_headers = true;
                            show_symbols = true;
                            show_dynamic = true;
                            show_relocs = true;
                        }
                        'h' => show_header = true,
                        'S' => show_sections = true,
                        'l' => show_program_headers = true,
                        's' => show_symbols = true,
                        'd' => show_dynamic = true,
                        'r' => show_relocs = true,
                        'W' => wide = true,
                        _ => {}
                    }
                }
            }
            _ if !arg.starts_with('-') => files.push(arg.clone()),
            _ => {}
        }
    }

    if files.is_empty() {
        eprintln!("readelf: Warning: Nothing to do.");
        return 1;
    }

    let _ = wide; // used in formatting below

    let mut errors = 0;
    for file in &files {
        let data = match fs::read(file) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("readelf: Error: '{file}': {e}");
                errors += 1;
                continue;
            }
        };

        // Try 64-bit first, then 32-bit
        if let Ok(elf) = ElfFile::<object::elf::FileHeader64<object::Endianness>>::parse(&*data) {
            readelf_display(
                &elf,
                &data,
                file,
                show_header,
                show_sections,
                show_program_headers,
                show_symbols,
                show_dynamic,
                show_relocs,
                wide,
            );
        } else if let Ok(elf) =
            ElfFile::<object::elf::FileHeader32<object::Endianness>>::parse(&*data)
        {
            readelf_display(
                &elf,
                &data,
                file,
                show_header,
                show_sections,
                show_program_headers,
                show_symbols,
                show_dynamic,
                show_relocs,
                wide,
            );
        } else {
            eprintln!("readelf: Error: Not an ELF file - {file}");
            errors += 1;
        }
    }

    if errors > 0 { 1 } else { 0 }
}

#[allow(clippy::too_many_arguments)]
fn readelf_display<'data, Elf: FileHeader>(
    elf: &ElfFile<'data, Elf>,
    data: &'data [u8],
    _file: &str,
    show_header: bool,
    show_sections: bool,
    show_program_headers: bool,
    show_symbols: bool,
    show_dynamic: bool,
    show_relocs: bool,
    _wide: bool,
) {
    let endian = elf.endian();

    if show_header {
        let header = elf.elf_header();
        let ident = header.e_ident();
        println!("ELF Header:");
        print!("  Magic:  ");
        for b in &ident.magic {
            print!(" {b:02x}");
        }
        for b in &[
            ident.class,
            ident.data,
            ident.version,
            ident.os_abi,
            ident.abi_version,
        ] {
            print!(" {b:02x}");
        }
        for b in &ident.padding {
            print!(" {b:02x}");
        }
        println!();
        println!(
            "  Class:                             {}",
            match ident.class {
                1 => "ELF32",
                2 => "ELF64",
                _ => "Unknown",
            }
        );
        println!(
            "  Data:                              {}",
            match ident.data {
                1 => "2's complement, little endian",
                2 => "2's complement, big endian",
                _ => "Unknown",
            }
        );
        println!(
            "  Version:                           {} (current)",
            ident.version
        );
        println!(
            "  OS/ABI:                            {}",
            elf_osabi_name(ident.os_abi)
        );
        println!("  ABI Version:                       {}", ident.abi_version);
        println!(
            "  Type:                              {}",
            elf_type_name(header.e_type(endian))
        );
        println!(
            "  Machine:                           {}",
            elf_machine_name(header.e_machine(endian))
        );
        println!(
            "  Version:                           0x{:x}",
            header.e_version(endian)
        );
        println!(
            "  Entry point address:               0x{:x}",
            header.e_entry(endian).into()
        );
        println!(
            "  Start of program headers:          {} (bytes into file)",
            header.e_phoff(endian).into()
        );
        println!(
            "  Start of section headers:          {} (bytes into file)",
            header.e_shoff(endian).into()
        );
        println!(
            "  Flags:                             0x{:x}",
            header.e_flags(endian)
        );
        println!(
            "  Size of this header:               {} (bytes)",
            header.e_ehsize(endian)
        );
        println!(
            "  Size of program headers:           {} (bytes)",
            header.e_phentsize(endian)
        );
        println!(
            "  Number of program headers:         {}",
            header.e_phnum(endian)
        );
        println!(
            "  Size of section headers:           {} (bytes)",
            header.e_shentsize(endian)
        );
        println!(
            "  Number of section headers:         {}",
            header.e_shnum(endian)
        );
        println!(
            "  Section header string table index: {}",
            header.e_shstrndx(endian)
        );
        println!();
    }

    if show_sections && let Ok(sections) = elf.elf_header().sections(endian, data) {
        println!("Section Headers:");
        println!(
            "  [Nr] Name              Type            Address          Off    Size   ES Flg Lk Inf Al"
        );
        for (i, section) in sections.iter().enumerate() {
            let name = sections
                .section_name(endian, section)
                .ok()
                .and_then(|n| std::str::from_utf8(n).ok())
                .unwrap_or("");
            let sh_type = section.sh_type(endian);
            let addr: u64 = section.sh_addr(endian).into();
            let offset: u64 = section.sh_offset(endian).into();
            let size: u64 = section.sh_size(endian).into();
            let entsize: u64 = section.sh_entsize(endian).into();
            let flags: u64 = section.sh_flags(endian).into();
            let link = section.sh_link(endian);
            let info = section.sh_info(endian);
            let addralign: u64 = section.sh_addralign(endian).into();

            let flag_str = elf_section_flags(flags);
            println!(
                "  [{i:>2}] {name:<17} {:<15} {addr:016x} {offset:06x} {size:06x} {entsize:02x} {flag_str:<3} {link:>2} {info:>3} {addralign:>2}",
                elf_section_type_name(sh_type)
            );
        }
        println!("Key to Flags:");
        println!("  W (write), A (alloc), X (execute), M (merge), S (strings)");
        println!();
    }

    if show_program_headers && let Ok(segments) = elf.elf_header().program_headers(endian, data) {
        println!("\nProgram Headers:");
        println!(
            "  Type           Offset   VirtAddr           PhysAddr           FileSiz  MemSiz   Flg Align"
        );
        for segment in segments {
            let p_type = segment.p_type(endian);
            let offset: u64 = segment.p_offset(endian).into();
            let vaddr: u64 = segment.p_vaddr(endian).into();
            let paddr: u64 = segment.p_paddr(endian).into();
            let filesz: u64 = segment.p_filesz(endian).into();
            let memsz: u64 = segment.p_memsz(endian).into();
            let flags = segment.p_flags(endian);
            let align: u64 = segment.p_align(endian).into();

            let flag_str = format!(
                "{}{}{}",
                if flags & 4 != 0 { "R" } else { " " },
                if flags & 2 != 0 { "W" } else { " " },
                if flags & 1 != 0 { "E" } else { " " }
            );

            println!(
                "  {:<14} 0x{offset:06x} 0x{vaddr:016x} 0x{paddr:016x} 0x{filesz:06x} 0x{memsz:06x} {flag_str} 0x{align:x}",
                elf_segment_type_name(p_type)
            );
        }
        println!();
    }

    if show_symbols {
        println!("\nSymbol table:");
        println!("   Num:    Value          Size Type    Bind   Vis      Ndx Name");
        for (i, sym) in elf.symbols().enumerate() {
            let name = sym.name().unwrap_or("");
            let value = sym.address();
            let size = sym.size();
            let kind = match sym.kind() {
                object::SymbolKind::Text => "FUNC",
                object::SymbolKind::Data => "OBJECT",
                object::SymbolKind::Section => "SECTION",
                object::SymbolKind::File => "FILE",
                object::SymbolKind::Tls => "TLS",
                _ => "NOTYPE",
            };
            let bind = if sym.is_weak() {
                "WEAK"
            } else if sym.is_global() {
                "GLOBAL"
            } else {
                "LOCAL"
            };
            let ndx = if sym.is_undefined() {
                "UND".to_string()
            } else if sym.is_common() {
                "COM".to_string()
            } else {
                match sym.section() {
                    object::SymbolSection::Absolute => "ABS".to_string(),
                    object::SymbolSection::Section(idx) => format!("{}", idx.0),
                    _ => "UND".to_string(),
                }
            };
            println!(
                "  {i:>4}: {value:016x} {size:>5} {kind:<7} {bind:<6} DEFAULT  {ndx:>3} {name}"
            );
        }
        println!();
    }

    if show_dynamic {
        println!("\nDynamic section:");
        for sym in elf.dynamic_symbols() {
            let name = sym.name().unwrap_or("");
            let value = sym.address();
            println!("  0x{value:016x} {name}");
        }
        println!();
    }

    if show_relocs {
        readelf_relocs(elf, data, endian);
    }
}

fn readelf_relocs<'data, Elf: FileHeader>(
    elf: &ElfFile<'data, Elf>,
    data: &'data [u8],
    endian: Elf::Endian,
) {
    if let Ok(sections) = elf.elf_header().sections(endian, data) {
        for section in sections.iter() {
            let name = sections
                .section_name(endian, section)
                .ok()
                .and_then(|n| std::str::from_utf8(n).ok())
                .unwrap_or("");

            if let Ok(Some((rels, _))) = section.rel(endian, data) {
                println!(
                    "\nRelocation section '{name}' contains {} entries:",
                    rels.len()
                );
                println!("  Offset          Info           Type           Sym. Value    Sym. Name");
                for rel in rels {
                    let r_offset: u64 = rel.r_offset(endian).into();
                    let r_info: u64 = rel.r_info(endian).into();
                    let r_sym = rel.r_sym(endian);
                    println!("  {r_offset:016x}  {r_info:012x}                    {r_sym}");
                }
            }

            if let Ok(Some((relas, _))) = section.rela(endian, data) {
                println!(
                    "\nRelocation section '{name}' contains {} entries:",
                    relas.len()
                );
                println!(
                    "  Offset          Info           Type           Sym. Value    Sym. Name + Addend"
                );
                for rela in relas {
                    let r_offset: u64 = rela.r_offset(endian).into();
                    let r_info: u64 = rela.r_info(endian, false).into();
                    let r_sym = rela.r_sym(endian, false);
                    let r_addend: i64 = rela.r_addend(endian).into();
                    println!(
                        "  {r_offset:016x}  {r_info:012x}                    {r_sym} + {r_addend:x}"
                    );
                }
            }
        }
    }
}

fn elf_osabi_name(osabi: u8) -> &'static str {
    match osabi {
        0 => "UNIX - System V",
        1 => "HP-UX",
        2 => "NetBSD",
        3 => "GNU/Linux",
        6 => "Solaris",
        9 => "FreeBSD",
        12 => "OpenBSD",
        _ => "Unknown",
    }
}

fn elf_type_name(ty: u16) -> &'static str {
    match ty {
        0 => "NONE (No file type)",
        1 => "REL (Relocatable file)",
        2 => "EXEC (Executable file)",
        3 => "DYN (Shared object file)",
        4 => "CORE (Core file)",
        _ => "Unknown",
    }
}

fn elf_machine_name(machine: u16) -> &'static str {
    match machine {
        0 => "None",
        3 => "Intel 80386",
        8 => "MIPS R3000",
        20 => "PowerPC",
        21 => "PowerPC64",
        40 => "ARM",
        43 => "SPARC v9",
        62 => "Advanced Micro Devices X86-64",
        183 => "AArch64",
        243 => "RISC-V",
        _ => "Unknown",
    }
}

fn elf_section_type_name(ty: u32) -> &'static str {
    match ty {
        0 => "NULL",
        1 => "PROGBITS",
        2 => "SYMTAB",
        3 => "STRTAB",
        4 => "RELA",
        5 => "HASH",
        6 => "DYNAMIC",
        7 => "NOTE",
        8 => "NOBITS",
        9 => "REL",
        11 => "DYNSYM",
        14 => "INIT_ARRAY",
        15 => "FINI_ARRAY",
        0x6ffffff6 => "GNU_HASH",
        0x6ffffffd => "GNU_VERDEF",
        0x6ffffffe => "GNU_VERNEED",
        0x6fffffff => "GNU_VERSYM",
        _ => "UNKNOWN",
    }
}

fn elf_section_flags(flags: u64) -> String {
    let mut s = String::new();
    if flags & 0x1 != 0 {
        s.push('W');
    }
    if flags & 0x2 != 0 {
        s.push('A');
    }
    if flags & 0x4 != 0 {
        s.push('X');
    }
    if flags & 0x10 != 0 {
        s.push('M');
    }
    if flags & 0x20 != 0 {
        s.push('S');
    }
    if flags & 0x40 != 0 {
        s.push('I');
    }
    if flags & 0x80 != 0 {
        s.push('L');
    }
    if flags & 0x200 != 0 {
        s.push('G');
    }
    if flags & 0x400 != 0 {
        s.push('T');
    }
    if flags & 0x800 != 0 {
        s.push('E');
    }
    s
}

fn elf_segment_type_name(ty: u32) -> &'static str {
    match ty {
        0 => "NULL",
        1 => "LOAD",
        2 => "DYNAMIC",
        3 => "INTERP",
        4 => "NOTE",
        5 => "SHLIB",
        6 => "PHDR",
        7 => "TLS",
        0x6474e550 => "GNU_EH_FRAME",
        0x6474e551 => "GNU_STACK",
        0x6474e552 => "GNU_RELRO",
        0x6474e553 => "GNU_PROPERTY",
        _ => "UNKNOWN",
    }
}

// ─── OBJDUMP ──────────────────────────────────────────────────────────────────

fn tool_objdump(args: &[String]) -> i32 {
    if check_version_help("objdump", args) {
        return 0;
    }

    let mut disassemble = false;
    let mut show_headers = false;
    let mut show_symbols = false;
    let mut show_relocs = false;
    let mut show_private = false;
    let mut files: Vec<String> = Vec::new();

    for arg in args {
        match arg.as_str() {
            "-d" | "--disassemble" => disassemble = true,
            "-h" | "--section-headers" | "--headers" => show_headers = true,
            "-t" | "--syms" => show_symbols = true,
            "-r" | "--reloc" => show_relocs = true,
            "-p" | "--private-headers" => show_private = true,
            _ if arg.starts_with('-') && !arg.starts_with("--") && arg != "-" => {
                for ch in arg[1..].chars() {
                    match ch {
                        'd' => disassemble = true,
                        'h' => show_headers = true,
                        't' => show_symbols = true,
                        'r' => show_relocs = true,
                        'p' => show_private = true,
                        _ => {}
                    }
                }
            }
            _ if !arg.starts_with('-') => files.push(arg.clone()),
            _ => {}
        }
    }

    if files.is_empty() {
        eprintln!("objdump: no input files");
        return 1;
    }

    let mut errors = 0;
    for file in &files {
        let data = match fs::read(file) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("objdump: '{file}': {e}");
                errors += 1;
                continue;
            }
        };
        let obj = match object::File::parse(&*data) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("objdump: {file}: {e}");
                errors += 1;
                continue;
            }
        };

        println!("\n{file}:     file format {}", objdump_format_name(&obj));

        if show_private {
            println!("\nProgram Header:");
            // Reuse readelf logic at a simpler level
            if let Ok(elf) = ElfFile::<object::elf::FileHeader64<object::Endianness>>::parse(&*data)
            {
                let endian = elf.endian();
                if let Ok(segments) = elf.elf_header().program_headers(endian, &*data) {
                    for segment in segments {
                        let p_type = segment.p_type(endian);
                        let vaddr: u64 = segment.p_vaddr(endian);
                        let filesz: u64 = segment.p_filesz(endian);
                        let memsz: u64 = segment.p_memsz(endian);
                        println!(
                            "    {:<14} off    0x{:016x} vaddr 0x{vaddr:016x} filesz 0x{filesz:06x} memsz 0x{memsz:06x}",
                            elf_segment_type_name(p_type),
                            Into::<u64>::into(segment.p_offset(endian))
                        );
                    }
                }
            }
        }

        if show_headers {
            println!("\nSections:");
            println!(
                "Idx Name          Size      VMA               LMA               File off  Algn"
            );
            for (i, section) in obj.sections().enumerate() {
                let name = section.name().unwrap_or("");
                if name.is_empty() && i == 0 {
                    continue;
                }
                let size = section.size();
                let addr = section.address();
                let align = section.align();
                println!(
                    "{i:>3} {name:<13} {size:08x}  {addr:016x}  {addr:016x}  {:08x}  2**{align}",
                    0 // file offset not easily available from object crate
                );
            }
        }

        if show_symbols {
            println!("\nSYMBOL TABLE:");
            for sym in obj.symbols() {
                let name = sym.name().unwrap_or("");
                let value = sym.address();
                let section_name = match sym.section() {
                    object::SymbolSection::Section(idx) => obj
                        .section_by_index(idx)
                        .ok()
                        .and_then(|s| s.name().ok())
                        .unwrap_or("*UND*"),
                    object::SymbolSection::Undefined => "*UND*",
                    object::SymbolSection::Absolute => "*ABS*",
                    _ => "*UND*",
                };
                let flags = if sym.is_global() { "g" } else { "l" };
                let kind = match sym.kind() {
                    object::SymbolKind::Text => "F",
                    object::SymbolKind::Data => "O",
                    _ => " ",
                };
                println!(
                    "{value:016x} {flags:<2}{kind} {section_name}\t{:016x} {name}",
                    sym.size()
                );
            }
        }

        if show_relocs {
            println!();
            for section in obj.sections() {
                let name = section.name().unwrap_or("");
                {
                    let relocs: Vec<_> = section.relocations().collect();
                    if !relocs.is_empty() {
                        println!("RELOCATION RECORDS FOR [{name}]:");
                        println!("OFFSET           TYPE              VALUE");
                        for (offset, reloc) in &relocs {
                            println!("{offset:016x} {:?}  {:?}", reloc.kind(), reloc.target());
                        }
                    }
                }
            }
        }

        if disassemble {
            println!("\nDisassembly:");
            for section in obj.sections() {
                let name = section.name().unwrap_or("");
                if section.kind() != object::SectionKind::Text {
                    continue;
                }
                println!("\nDisassembly of section {name}:");
                if let Ok(data) = section.data() {
                    let base = section.address();
                    // Print hex bytes (no actual disassembly)
                    let mut offset = 0;
                    while offset < data.len() {
                        let addr = base + offset as u64;
                        let end = (offset + 16).min(data.len());
                        let bytes = &data[offset..end];
                        print!("  {addr:8x}:\t");
                        for b in bytes {
                            print!("{b:02x} ");
                        }
                        println!();
                        offset += 16;
                    }
                }
            }
        }
    }

    if errors > 0 { 1 } else { 0 }
}

fn objdump_format_name(obj: &object::File<'_>) -> &'static str {
    match (obj.format(), obj.is_64()) {
        (object::BinaryFormat::Elf, true) => "elf64",
        (object::BinaryFormat::Elf, false) => "elf32",
        _ => "unknown",
    }
}

// ─── OBJCOPY ──────────────────────────────────────────────────────────────────

fn tool_objcopy(args: &[String]) -> i32 {
    if check_version_help("objcopy", args) {
        return 0;
    }

    let mut strip_debug = false;
    let mut strip_all = false;
    let mut remove_sections: Vec<String> = Vec::new();
    let mut keep_sections: Vec<String> = Vec::new();
    let mut output_format: Option<String> = None;
    let mut files: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--strip-debug" | "-g" => strip_debug = true,
            "--strip-all" | "-S" => strip_all = true,
            "-j" | "--only-section" => {
                i += 1;
                if i < args.len() {
                    keep_sections.push(args[i].clone());
                }
            }
            "-R" | "--remove-section" => {
                i += 1;
                if i < args.len() {
                    remove_sections.push(args[i].clone());
                }
            }
            "-O" | "--output-target" => {
                i += 1;
                if i < args.len() {
                    output_format = Some(args[i].clone());
                }
            }
            _ if arg.starts_with("--only-section=") => {
                keep_sections.push(arg.split_once('=').unwrap().1.to_string());
            }
            _ if arg.starts_with("--remove-section=") => {
                remove_sections.push(arg.split_once('=').unwrap().1.to_string());
            }
            _ if arg.starts_with("--output-target=") => {
                output_format = Some(arg.split_once('=').unwrap().1.to_string());
            }
            _ if !arg.starts_with('-') => files.push(arg.clone()),
            _ => {}
        }
        i += 1;
    }

    if files.is_empty() {
        eprintln!("objcopy: no input file");
        return 1;
    }

    let input = &files[0];
    let output = if files.len() > 1 { &files[1] } else { input };

    // For binary output format, extract raw sections
    if output_format.as_deref() == Some("binary") {
        return objcopy_to_binary(input, output, &keep_sections);
    }

    // Default: copy file, applying section removal / stripping
    let data = match fs::read(input) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("objcopy: '{input}': {e}");
            return 1;
        }
    };

    let obj = match object::File::parse(&*data) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("objcopy: {input}: {e}");
            return 1;
        }
    };

    let mut builder =
        object::write::Object::new(obj.format(), obj.architecture(), obj.endianness());

    if let object::FileFlags::Elf {
        os_abi,
        abi_version,
        e_flags,
    } = obj.flags()
    {
        builder.flags = object::FileFlags::Elf {
            os_abi,
            abi_version,
            e_flags,
        };
    }

    let mut section_map: HashMap<object::SectionIndex, object::write::SectionId> = HashMap::new();

    for section in obj.sections() {
        if section.index().0 == 0 {
            continue;
        }
        let name = section.name().unwrap_or("");

        // Apply filters
        if !keep_sections.is_empty() && !keep_sections.iter().any(|s| s == name) {
            continue;
        }
        if remove_sections.iter().any(|s| s == name) {
            continue;
        }
        if strip_all && (name == ".symtab" || name == ".strtab" || is_debug_section(name)) {
            continue;
        }
        if strip_debug && is_debug_section(name) {
            continue;
        }
        if name == ".symtab" || name == ".strtab" {
            continue; // managed by writer
        }

        let new_id = builder.add_section(Vec::new(), name.as_bytes().to_vec(), section.kind());
        builder.section_mut(new_id).flags = section.flags();

        if let Ok(section_data) = section.uncompressed_data()
            && !section_data.is_empty()
        {
            builder.set_section_data(new_id, section_data.into_owned(), section.align());
        }

        section_map.insert(section.index(), new_id);
    }

    // Copy symbols unless stripping all
    if !strip_all {
        for sym in obj.symbols() {
            if sym.index().0 == 0 {
                continue;
            }
            let name = match sym.name_bytes() {
                Ok(n) => n,
                Err(_) => continue,
            };

            let section = match sym.section() {
                object::SymbolSection::Section(idx) => {
                    if let Some(&new_id) = section_map.get(&idx) {
                        object::write::SymbolSection::Section(new_id)
                    } else {
                        continue;
                    }
                }
                object::SymbolSection::Absolute => object::write::SymbolSection::Absolute,
                object::SymbolSection::Common => object::write::SymbolSection::Common,
                object::SymbolSection::Undefined => object::write::SymbolSection::Undefined,
                _ => continue,
            };

            builder.add_symbol(object::write::Symbol {
                name: name.to_vec(),
                value: sym.address(),
                size: sym.size(),
                kind: sym.kind(),
                scope: sym.scope(),
                weak: sym.is_weak(),
                section,
                flags: object::SymbolFlags::None,
            });
        }
    }

    let mut out_buf = Vec::new();
    if let Err(e) = builder.emit(&mut out_buf) {
        eprintln!("objcopy: failed to write output: {e}");
        return 1;
    }

    if let Err(e) = fs::write(output, &out_buf) {
        eprintln!("objcopy: {output}: {e}");
        return 1;
    }

    0
}

fn objcopy_to_binary(input: &str, output: &str, keep_sections: &[String]) -> i32 {
    let data = match fs::read(input) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("objcopy: '{input}': {e}");
            return 1;
        }
    };
    let obj = match object::File::parse(&*data) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("objcopy: {input}: {e}");
            return 1;
        }
    };

    // Collect loadable sections, sorted by address
    let mut sections: Vec<(u64, Vec<u8>)> = Vec::new();
    for section in obj.sections() {
        let name = section.name().unwrap_or("");
        if !keep_sections.is_empty() && !keep_sections.iter().any(|s| s == name) {
            continue;
        }
        if section.size() == 0 {
            continue;
        }
        if let Ok(d) = section.data()
            && !d.is_empty()
        {
            sections.push((section.address(), d.to_vec()));
        }
    }

    sections.sort_by_key(|(addr, _)| *addr);

    if sections.is_empty() {
        let _ = fs::write(output, []);
        return 0;
    }

    let base = sections[0].0;
    let end = sections
        .iter()
        .map(|(addr, data)| addr + data.len() as u64)
        .max()
        .unwrap_or(base);
    let total = (end - base) as usize;
    let mut out = vec![0u8; total];
    for (addr, data) in &sections {
        let offset = (addr - base) as usize;
        let len = data.len().min(total - offset);
        out[offset..offset + len].copy_from_slice(&data[..len]);
    }

    if let Err(e) = fs::write(output, &out) {
        eprintln!("objcopy: {output}: {e}");
        return 1;
    }
    0
}

fn is_debug_section(name: &str) -> bool {
    name.starts_with(".debug_")
        || name.starts_with(".zdebug_")
        || name == ".line"
        || name == ".stab"
        || name == ".stabstr"
        || name == ".gdb_index"
        || name == ".comment"
}

// ─── STRIP ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StripMode {
    All,
    Debug,
    Unneeded,
}

fn tool_strip(args: &[String]) -> i32 {
    if check_version_help("strip", args) {
        return 0;
    }

    let mut mode = StripMode::All;
    let mut preserve_dates = false;
    let mut output_file: Option<String> = None;
    let mut remove_sections: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-s" | "--strip-all" => mode = StripMode::All,
            "-g" | "-S" | "--strip-debug" => mode = StripMode::Debug,
            "--strip-unneeded" => mode = StripMode::Unneeded,
            "-p" | "--preserve-dates" => preserve_dates = true,
            "-o" => {
                i += 1;
                if i < args.len() {
                    output_file = Some(args[i].clone());
                }
            }
            "-R" | "--remove-section" => {
                i += 1;
                if i < args.len() {
                    remove_sections.push(args[i].clone());
                }
            }
            _ if arg.starts_with("--output-file=") => {
                output_file = Some(arg.split_once('=').unwrap().1.to_string());
            }
            _ if arg.starts_with("--remove-section=") => {
                remove_sections.push(arg.split_once('=').unwrap().1.to_string());
            }
            _ if !arg.starts_with('-') => files.push(arg.clone()),
            _ => {}
        }
        i += 1;
    }

    if files.is_empty() {
        eprintln!("strip: no input files");
        return 1;
    }

    if output_file.is_some() && files.len() > 1 {
        eprintln!("strip: -o may not be used with multiple files");
        return 1;
    }

    let mut errors = 0;
    for file in &files {
        let path = Path::new(file);
        if !path.exists() {
            eprintln!("strip: '{file}': No such file");
            errors += 1;
            continue;
        }

        let timestamps = if preserve_dates {
            fs::metadata(path).ok().and_then(|m| m.modified().ok())
        } else {
            None
        };

        let data = match fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("strip: {file}: {e}");
                errors += 1;
                continue;
            }
        };

        let obj = match object::File::parse(&*data) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("strip: {file}: {e}");
                errors += 1;
                continue;
            }
        };

        let reloc_symbols = if mode == StripMode::Unneeded {
            collect_reloc_symbols(&data)
        } else {
            HashSet::new()
        };

        match strip_rewrite(&obj, mode, &remove_sections, &reloc_symbols) {
            Ok(out) => {
                let out_path = output_file.as_deref().unwrap_or(file);
                if let Err(e) = fs::write(out_path, &out) {
                    eprintln!("strip: {out_path}: {e}");
                    errors += 1;
                }
                if let Some(mtime) = timestamps {
                    let _ = set_file_times(Path::new(out_path), mtime);
                }
            }
            Err(e) => {
                eprintln!("strip: {file}: {e}");
                errors += 1;
            }
        }
    }

    if errors > 0 { 1 } else { 0 }
}

fn strip_rewrite(
    obj: &object::File<'_>,
    mode: StripMode,
    remove_sections: &[String],
    reloc_symbols: &HashSet<object::SymbolIndex>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut builder =
        object::write::Object::new(obj.format(), obj.architecture(), obj.endianness());

    if let object::FileFlags::Elf {
        os_abi,
        abi_version,
        e_flags,
    } = obj.flags()
    {
        builder.flags = object::FileFlags::Elf {
            os_abi,
            abi_version,
            e_flags,
        };
    }

    let mut section_map: HashMap<object::SectionIndex, object::write::SectionId> = HashMap::new();

    for section in obj.sections() {
        if section.index().0 == 0 {
            continue;
        }
        let name = section.name().unwrap_or("");

        if should_remove_section(name, mode, remove_sections) {
            continue;
        }
        if name == ".symtab" || name == ".strtab" {
            continue;
        }

        let new_id = builder.add_section(Vec::new(), name.as_bytes().to_vec(), section.kind());
        builder.section_mut(new_id).flags = section.flags();

        if let Ok(section_data) = section.uncompressed_data()
            && !section_data.is_empty()
        {
            builder.set_section_data(new_id, section_data.into_owned(), section.align());
        }

        section_map.insert(section.index(), new_id);
    }

    if mode != StripMode::All {
        for sym in obj.symbols() {
            if sym.index().0 == 0 {
                continue;
            }

            if !strip_should_keep(&sym, mode, reloc_symbols) {
                continue;
            }

            let name = sym.name_bytes()?;
            let section = match sym.section() {
                object::SymbolSection::Section(idx) => {
                    if let Some(&new_id) = section_map.get(&idx) {
                        object::write::SymbolSection::Section(new_id)
                    } else {
                        continue;
                    }
                }
                object::SymbolSection::Absolute => object::write::SymbolSection::Absolute,
                object::SymbolSection::Common => object::write::SymbolSection::Common,
                object::SymbolSection::Undefined => object::write::SymbolSection::Undefined,
                _ => continue,
            };

            builder.add_symbol(object::write::Symbol {
                name: name.to_vec(),
                value: sym.address(),
                size: sym.size(),
                kind: sym.kind(),
                scope: sym.scope(),
                weak: sym.is_weak(),
                section,
                flags: object::SymbolFlags::None,
            });
        }
    }

    let mut out_buf = Vec::new();
    builder.emit(&mut out_buf)?;
    Ok(out_buf)
}

fn should_remove_section(name: &str, mode: StripMode, remove_sections: &[String]) -> bool {
    if remove_sections.iter().any(|s| s == name) {
        return true;
    }
    match mode {
        StripMode::Debug | StripMode::Unneeded => is_debug_section(name),
        StripMode::All => is_debug_section(name) || name == ".symtab" || name == ".strtab",
    }
}

fn strip_should_keep(
    sym: &object::read::Symbol<'_, '_>,
    mode: StripMode,
    reloc_symbols: &HashSet<object::SymbolIndex>,
) -> bool {
    if sym.is_undefined() {
        return true;
    }
    match mode {
        StripMode::All => false,
        StripMode::Debug => sym.kind() != object::SymbolKind::File,
        StripMode::Unneeded => sym.is_global() || reloc_symbols.contains(&sym.index()),
    }
}

fn collect_reloc_symbols(data: &[u8]) -> HashSet<object::SymbolIndex> {
    let mut indices = HashSet::new();
    if let Ok(elf) = ElfFile::<object::elf::FileHeader64<object::Endianness>>::parse(data) {
        collect_reloc_symbols_elf(&elf, &mut indices);
    } else if let Ok(elf) = ElfFile::<object::elf::FileHeader32<object::Endianness>>::parse(data) {
        collect_reloc_symbols_elf(&elf, &mut indices);
    }
    indices
}

fn collect_reloc_symbols_elf<'data, Elf: FileHeader>(
    elf: &ElfFile<'data, Elf>,
    indices: &mut HashSet<object::SymbolIndex>,
) {
    let endian = elf.endian();
    let data = elf.data();
    if let Ok(sections) = elf.elf_header().sections(endian, data) {
        for section in sections.iter() {
            if let Ok(Some((rels, _))) = section.rel(endian, data) {
                for rel in rels {
                    let sym_idx = rel.r_sym(endian);
                    if sym_idx != 0 {
                        indices.insert(object::SymbolIndex(sym_idx as usize));
                    }
                }
            }
            if let Ok(Some((relas, _))) = section.rela(endian, data) {
                for rela in relas {
                    let sym_idx = rela.r_sym(endian, false);
                    if sym_idx != 0 {
                        indices.insert(object::SymbolIndex(sym_idx as usize));
                    }
                }
            }
        }
    }
}

fn set_file_times(path: &Path, mtime: SystemTime) -> Result<(), Box<dyn std::error::Error>> {
    let file = fs::File::options().write(true).open(path)?;
    file.set_modified(mtime)?;
    Ok(())
}

// ─── ADDR2LINE (stub) ─────────────────────────────────────────────────────────

fn tool_addr2line(args: &[String]) -> i32 {
    if check_version_help("addr2line", args) {
        return 0;
    }
    // Stub: read addresses from args or stdin, print ??:0
    let mut addrs: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "-e" | "--exe" => {
                i += 1; // skip filename
            }
            "-f" | "--functions" | "-C" | "--demangle" | "-i" | "--inlines" => {}
            _ if !arg.starts_with('-') => addrs.push(arg.clone()),
            _ => {}
        }
        i += 1;
    }

    if addrs.is_empty() {
        // Read from stdin
        let stdin = io::stdin();
        for line in stdin.lock().lines().map_while(Result::ok) {
            for addr in line.split_whitespace() {
                println!("??");
                println!("??:0");
                let _ = addr;
            }
        }
    } else {
        for _ in &addrs {
            println!("??");
            println!("??:0");
        }
    }
    0
}

// ─── C++FILT ──────────────────────────────────────────────────────────────────

fn tool_cxxfilt(args: &[String]) -> i32 {
    if check_version_help("c++filt", args) {
        return 0;
    }

    // Collect any positional arguments (mangled names)
    let mut names: Vec<String> = Vec::new();
    for arg in args {
        if !arg.starts_with('-') {
            names.push(arg.clone());
        }
    }

    let stdout = io::stdout();
    let mut out = stdout.lock();

    if names.is_empty() {
        // Read from stdin line by line
        let stdin = io::stdin();
        for line in stdin.lock().lines().map_while(Result::ok) {
            let demangled = demangle_line(&line);
            let _ = writeln!(out, "{demangled}");
        }
    } else {
        for name in &names {
            let demangled = demangle_symbol(name);
            let _ = writeln!(out, "{demangled}");
        }
    }
    0
}

fn demangle_line(line: &str) -> String {
    // Split line into tokens and demangle each
    let mut result = String::new();
    let mut chars = line.chars().peekable();

    while let Some(&ch) = chars.peek() {
        if ch.is_alphanumeric() || ch == '_' || ch == '$' {
            let mut token = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_alphanumeric() || c == '_' || c == '$' || c == '.' {
                    token.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            result.push_str(&demangle_symbol(&token));
        } else {
            result.push(ch);
            chars.next();
        }
    }
    result
}

fn demangle_symbol(sym: &str) -> String {
    // Basic Itanium C++ ABI demangling
    if let Some(rest) = sym.strip_prefix("_Z")
        && let Some(demangled) = try_demangle_itanium(rest)
    {
        return demangled;
    }
    sym.to_string()
}

fn try_demangle_itanium(mangled: &str) -> Option<String> {
    // Very basic: parse nested names like _ZN...E or simple names
    let chars: Vec<char> = mangled.chars().collect();
    let mut pos = 0;

    if pos < chars.len() && chars[pos] == 'N' {
        // Nested name
        pos += 1;
        let mut parts = Vec::new();
        while pos < chars.len() && chars[pos] != 'E' {
            if let Some((name, new_pos)) = parse_source_name(&chars, pos) {
                parts.push(name);
                pos = new_pos;
            } else {
                return None;
            }
        }
        if parts.is_empty() {
            return None;
        }
        Some(parts.join("::"))
    } else {
        // Simple name
        if let Some((name, _)) = parse_source_name(&chars, pos) {
            Some(name)
        } else {
            None
        }
    }
}

fn parse_source_name(chars: &[char], mut pos: usize) -> Option<(String, usize)> {
    // Parse <length><name>
    let mut len_str = String::new();
    while pos < chars.len() && chars[pos].is_ascii_digit() {
        len_str.push(chars[pos]);
        pos += 1;
    }
    if len_str.is_empty() {
        return None;
    }
    let len: usize = len_str.parse().ok()?;
    if pos + len > chars.len() {
        return None;
    }
    let name: String = chars[pos..pos + len].iter().collect();
    Some((name, pos + len))
}

// ─── AS (stub) ────────────────────────────────────────────────────────────────

fn tool_as(args: &[String]) -> i32 {
    if check_version_help("as", args) {
        return 0;
    }

    // Try to delegate to system assembler
    let candidates = ["as", "/usr/bin/as", "/usr/bin/gas"];
    for candidate in &candidates {
        let path = Path::new(candidate);
        if path.exists()
            && path
                .canonicalize()
                .ok()
                .map(|p| !p.ends_with("rust-binutils"))
                .unwrap_or(true)
        {
            let status = process::Command::new(candidate).args(args).status();
            match status {
                Ok(s) => return s.code().unwrap_or(1),
                Err(_) => continue,
            }
        }
    }

    eprintln!("as: assembler not implemented; install a system assembler (e.g., GNU as)");
    1
}

// ─── LD (stub) ────────────────────────────────────────────────────────────────

fn tool_ld(args: &[String]) -> i32 {
    if check_version_help("ld", args) {
        return 0;
    }

    // Try to delegate to system linker
    let candidates = ["ld", "/usr/bin/ld", "/usr/bin/ld.bfd", "/usr/bin/ld.gold"];
    for candidate in &candidates {
        let path = Path::new(candidate);
        if path.exists()
            && path
                .canonicalize()
                .ok()
                .map(|p| !p.ends_with("rust-binutils"))
                .unwrap_or(true)
        {
            let status = process::Command::new(candidate).args(args).status();
            match status {
                Ok(s) => return s.code().unwrap_or(1),
                Err(_) => continue,
            }
        }
    }

    eprintln!("ld: linker not implemented; install a system linker (e.g., GNU ld)");
    1
}
