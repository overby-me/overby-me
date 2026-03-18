use goblin::elf::dynamic::{DT_NEEDED, DT_RPATH, DT_RUNPATH, DT_SONAME, DT_STRSZ, DT_STRTAB};
use goblin::elf::program_header::PT_INTERP;
use goblin::elf::Elf;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug)]
enum Action {
    PrintInterpreter,
    SetInterpreter(String),
    PrintRpath,
    SetRpath(String),
    ShrinkRpath,
    RemoveRpath,
    PrintSoname,
    SetSoname(String),
    PrintNeeded,
    AddNeeded(String),
    RemoveNeeded(String),
    ReplaceNeeded(String, String),
}

struct Options {
    actions: Vec<Action>,
    output: Option<PathBuf>,
    page_size: Option<usize>,
    file: Option<PathBuf>,
}

fn parse_args() -> Options {
    let args: Vec<String> = std::env::args().collect();
    let mut actions = Vec::new();
    let mut output = None;
    let mut page_size = None;
    let mut file = None;
    let mut i = 1;

    while i < args.len() {
        match args[i].as_str() {
            "--version" => {
                println!("patchelf {VERSION}");
                process::exit(0);
            }
            "--help" | "-h" => {
                print_usage();
                process::exit(0);
            }
            "--print-interpreter" => actions.push(Action::PrintInterpreter),
            "--set-interpreter" => {
                i += 1;
                let path = expect_arg(&args, i, "--set-interpreter");
                actions.push(Action::SetInterpreter(path));
            }
            "--print-rpath" => actions.push(Action::PrintRpath),
            "--set-rpath" => {
                i += 1;
                let path = expect_arg(&args, i, "--set-rpath");
                actions.push(Action::SetRpath(path));
            }
            "--shrink-rpath" => actions.push(Action::ShrinkRpath),
            "--remove-rpath" => actions.push(Action::RemoveRpath),
            "--print-soname" => actions.push(Action::PrintSoname),
            "--set-soname" => {
                i += 1;
                let name = expect_arg(&args, i, "--set-soname");
                actions.push(Action::SetSoname(name));
            }
            "--print-needed" => actions.push(Action::PrintNeeded),
            "--add-needed" => {
                i += 1;
                let lib = expect_arg(&args, i, "--add-needed");
                actions.push(Action::AddNeeded(lib));
            }
            "--remove-needed" => {
                i += 1;
                let lib = expect_arg(&args, i, "--remove-needed");
                actions.push(Action::RemoveNeeded(lib));
            }
            "--replace-needed" => {
                i += 1;
                let old = expect_arg(&args, i, "--replace-needed (old)");
                i += 1;
                let new = expect_arg(&args, i, "--replace-needed (new)");
                actions.push(Action::ReplaceNeeded(old, new));
            }
            "--output" => {
                i += 1;
                output = Some(PathBuf::from(expect_arg(&args, i, "--output")));
            }
            "--page-size" => {
                i += 1;
                let s = expect_arg(&args, i, "--page-size");
                page_size = Some(s.parse::<usize>().unwrap_or_else(|_| {
                    eprintln!("patchelf: invalid page size: {s}");
                    process::exit(1);
                }));
            }
            arg if arg.starts_with('-') => {
                eprintln!("patchelf: unknown option: {arg}");
                process::exit(1);
            }
            _ => {
                file = Some(PathBuf::from(&args[i]));
            }
        }
        i += 1;
    }

    if actions.is_empty() && file.is_some() {
        eprintln!("patchelf: no operation specified");
        process::exit(1);
    }

    if file.is_none() && !actions.is_empty() {
        eprintln!("patchelf: no input file specified");
        process::exit(1);
    }

    Options {
        actions,
        output,
        page_size,
        file,
    }
}

fn expect_arg(args: &[String], i: usize, flag: &str) -> String {
    if i >= args.len() {
        eprintln!("patchelf: {flag} requires an argument");
        process::exit(1);
    }
    args[i].clone()
}

fn print_usage() {
    println!(
        "Usage: patchelf [OPTION]... FILE

Options:
  --print-interpreter         Print the ELF interpreter
  --set-interpreter PATH      Set the ELF interpreter
  --print-rpath               Print DT_RPATH/DT_RUNPATH
  --set-rpath PATH            Set DT_RUNPATH
  --shrink-rpath              Remove unused RPATH entries
  --remove-rpath              Remove DT_RPATH and DT_RUNPATH
  --print-soname              Print DT_SONAME
  --set-soname NAME           Set DT_SONAME
  --print-needed              Print DT_NEEDED entries
  --add-needed LIB            Add a DT_NEEDED entry
  --remove-needed LIB         Remove a DT_NEEDED entry
  --replace-needed OLD NEW    Replace a DT_NEEDED entry
  --output FILE               Write to FILE instead of modifying in-place
  --page-size SIZE            Set page alignment size
  --version                   Print version
  --help                      Print this help"
    );
}

fn main() {
    let opts = parse_args();

    let file = match &opts.file {
        Some(f) => f,
        None => {
            print_usage();
            process::exit(0);
        }
    };

    let data = fs::read(file).unwrap_or_else(|e| {
        eprintln!("patchelf: cannot read '{}': {e}", file.display());
        process::exit(1);
    });

    let elf = Elf::parse(&data).unwrap_or_else(|e| {
        eprintln!("patchelf: not a valid ELF file '{}': {e}", file.display());
        process::exit(1);
    });

    let mut modified_data: Option<Vec<u8>> = None;
    let mut needs_write = false;

    for action in &opts.actions {
        match action {
            Action::PrintInterpreter => {
                print_interpreter(&elf);
            }
            Action::SetInterpreter(new_interp) => {
                let buf = modified_data.get_or_insert_with(|| data.clone());
                set_interpreter(&elf, buf, new_interp);
                needs_write = true;
            }
            Action::PrintRpath => {
                print_rpath(&elf);
            }
            Action::SetRpath(new_rpath) => {
                let buf = modified_data.get_or_insert_with(|| data.clone());
                set_rpath(&elf, buf, new_rpath);
                needs_write = true;
            }
            Action::ShrinkRpath => {
                let buf = modified_data.get_or_insert_with(|| data.clone());
                shrink_rpath(&elf, buf, file);
                needs_write = true;
            }
            Action::RemoveRpath => {
                let buf = modified_data.get_or_insert_with(|| data.clone());
                remove_rpath(&elf, buf);
                needs_write = true;
            }
            Action::PrintSoname => {
                print_soname(&elf);
            }
            Action::SetSoname(new_soname) => {
                let buf = modified_data.get_or_insert_with(|| data.clone());
                set_soname(&elf, buf, new_soname);
                needs_write = true;
            }
            Action::PrintNeeded => {
                print_needed(&elf);
            }
            Action::AddNeeded(lib) => {
                let buf = modified_data.get_or_insert_with(|| data.clone());
                add_needed(&elf, buf, lib);
                needs_write = true;
            }
            Action::RemoveNeeded(lib) => {
                let buf = modified_data.get_or_insert_with(|| data.clone());
                remove_needed(&elf, buf, lib);
                needs_write = true;
            }
            Action::ReplaceNeeded(old, new) => {
                let buf = modified_data.get_or_insert_with(|| data.clone());
                replace_needed(&elf, buf, old, new);
                needs_write = true;
            }
        }
    }

    if needs_write {
        let buf = modified_data.as_ref().unwrap_or(&data);
        let out_path = opts.output.as_deref().unwrap_or(file);
        fs::write(out_path, buf).unwrap_or_else(|e| {
            eprintln!("patchelf: cannot write '{}': {e}", out_path.display());
            process::exit(1);
        });
    }

    let _ = opts.page_size; // reserved for future use
}

// --- Print operations ---

fn print_interpreter(elf: &Elf) {
    match &elf.interpreter {
        Some(interp) => println!("{interp}"),
        None => {
            eprintln!("patchelf: no PT_INTERP segment found");
            process::exit(1);
        }
    }
}

fn print_rpath(elf: &Elf) {
    // Prefer DT_RUNPATH, fall back to DT_RPATH
    if !elf.runpaths.is_empty() {
        println!("{}", elf.runpaths.join(":"));
        return;
    }
    if !elf.rpaths.is_empty() {
        println!("{}", elf.rpaths.join(":"));
        return;
    }
    // patchelf prints empty string if no rpath
    println!();
}

fn print_soname(elf: &Elf) {
    match &elf.soname {
        Some(name) => println!("{name}"),
        None => {
            eprintln!("patchelf: no DT_SONAME found");
            process::exit(1);
        }
    }
}

fn print_needed(elf: &Elf) {
    for lib in &elf.libraries {
        println!("{lib}");
    }
}

// --- Helpers for finding dynamic string table info ---

/// Get the file offset and size of the dynamic string table (.dynstr).
fn dynstr_info(elf: &Elf, _data: &[u8]) -> (usize, usize) {
    let mut strtab_addr: Option<u64> = None;
    let mut strsz: Option<u64> = None;

    if let Some(dynamic) = &elf.dynamic {
        for dyn_entry in &dynamic.dyns {
            match dyn_entry.d_tag as u64 {
                DT_STRTAB => strtab_addr = Some(dyn_entry.d_val),
                DT_STRSZ => strsz = Some(dyn_entry.d_val),
                _ => {}
            }
        }
    }

    let strtab_vaddr = strtab_addr.unwrap_or_else(|| {
        eprintln!("patchelf: no DT_STRTAB found");
        process::exit(1);
    });
    let strtab_size = strsz.unwrap_or_else(|| {
        eprintln!("patchelf: no DT_STRSZ found");
        process::exit(1);
    });

    let strtab_offset = vaddr_to_offset(elf, strtab_vaddr).unwrap_or_else(|| {
        eprintln!("patchelf: cannot map DT_STRTAB vaddr to file offset");
        process::exit(1);
    });

    (strtab_offset, strtab_size as usize)
}

/// Convert a virtual address to a file offset using program headers.
fn vaddr_to_offset(elf: &Elf, vaddr: u64) -> Option<usize> {
    for phdr in &elf.program_headers {
        if phdr.p_type == goblin::elf::program_header::PT_LOAD {
            if vaddr >= phdr.p_vaddr && vaddr < phdr.p_vaddr + phdr.p_memsz {
                return Some((vaddr - phdr.p_vaddr + phdr.p_offset) as usize);
            }
        }
    }
    None
}

/// Read a null-terminated string from a buffer at the given offset.
fn read_str_at(data: &[u8], offset: usize) -> &str {
    let end = data[offset..]
        .iter()
        .position(|&b| b == 0)
        .map(|p| offset + p)
        .unwrap_or(data.len());
    std::str::from_utf8(&data[offset..end]).unwrap_or("")
}

/// Find the file offset of a string within the dynamic string table, given its value.
fn _find_dynstr_offset(data: &[u8], strtab_offset: usize, strtab_size: usize, needle: &str) -> Option<usize> {
    let strtab = &data[strtab_offset..strtab_offset + strtab_size];
    let needle_bytes = needle.as_bytes();
    // Search for null-terminated match
    let mut i = 0;
    while i < strtab.len() {
        let end = strtab[i..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| i + p)
            .unwrap_or(strtab.len());
        if &strtab[i..end] == needle_bytes {
            return Some(strtab_offset + i);
        }
        i = end + 1;
    }
    None
}

/// Write a string in-place in the buffer at the given offset, padding with null bytes
/// up to `max_len` (including the null terminator area).
fn write_str_inplace(data: &mut [u8], offset: usize, max_len: usize, new_val: &str) {
    let new_bytes = new_val.as_bytes();
    if new_bytes.len() + 1 > max_len {
        eprintln!(
            "patchelf: new value '{}' ({} bytes) does not fit in the available space ({} bytes)",
            new_val,
            new_bytes.len(),
            max_len - 1
        );
        process::exit(1);
    }
    data[offset..offset + new_bytes.len()].copy_from_slice(new_bytes);
    // Pad remaining space with null bytes
    for b in &mut data[offset + new_bytes.len()..offset + max_len] {
        *b = 0;
    }
}

/// Get the length of the old null-terminated string at `offset` (not counting the terminator),
/// but also account for any consecutive null padding until the next non-null byte or end.
fn available_space_at(data: &[u8], offset: usize, strtab_end: usize) -> usize {
    // Find end of the current string
    let str_end = data[offset..]
        .iter()
        .position(|&b| b == 0)
        .map(|p| offset + p)
        .unwrap_or(strtab_end);
    // Count consecutive null bytes after the string
    let mut pad_end = str_end + 1; // skip the null terminator
    while pad_end < strtab_end && data[pad_end] == 0 {
        pad_end += 1;
    }
    pad_end - offset
}

// --- Modify operations ---

fn set_interpreter(elf: &Elf, data: &mut Vec<u8>, new_interp: &str) {
    let interp_phdr = elf
        .program_headers
        .iter()
        .find(|ph| ph.p_type == PT_INTERP)
        .unwrap_or_else(|| {
            eprintln!("patchelf: no PT_INTERP segment found");
            process::exit(1);
        });

    let offset = interp_phdr.p_offset as usize;
    let seg_size = interp_phdr.p_filesz as usize;
    let new_bytes = new_interp.as_bytes();

    if new_bytes.len() + 1 > seg_size {
        eprintln!(
            "patchelf: new interpreter '{}' ({} bytes) does not fit in PT_INTERP segment ({} bytes)",
            new_interp,
            new_bytes.len() + 1,
            seg_size
        );
        process::exit(1);
    }

    data[offset..offset + new_bytes.len()].copy_from_slice(new_bytes);
    // Null-terminate and zero-pad
    for b in &mut data[offset + new_bytes.len()..offset + seg_size] {
        *b = 0;
    }
}

fn set_rpath(elf: &Elf, data: &mut Vec<u8>, new_rpath: &str) {
    let (strtab_offset, strtab_size) = dynstr_info(elf, data);
    let strtab_end = strtab_offset + strtab_size;

    if let Some(dynamic) = &elf.dynamic {
        // Try DT_RUNPATH first, then DT_RPATH
        for tag in &[DT_RUNPATH, DT_RPATH] {
            for dyn_entry in &dynamic.dyns {
                if dyn_entry.d_tag as u64 == *tag {
                    let str_offset = strtab_offset + dyn_entry.d_val as usize;
                    let space = available_space_at(data, str_offset, strtab_end);
                    write_str_inplace(data, str_offset, space, new_rpath);
                    return;
                }
            }
        }
    }

    eprintln!("patchelf: no DT_RUNPATH or DT_RPATH found; cannot set rpath");
    process::exit(1);
}

fn shrink_rpath(elf: &Elf, data: &mut Vec<u8>, _file: &Path) {
    let (strtab_offset, strtab_size) = dynstr_info(elf, data);
    let strtab_end = strtab_offset + strtab_size;

    // Get the needed libraries
    let needed: HashSet<&str> = elf.libraries.iter().map(|s| s.as_ref()).collect();

    if let Some(dynamic) = &elf.dynamic {
        for tag in &[DT_RUNPATH, DT_RPATH] {
            for dyn_entry in &dynamic.dyns {
                if dyn_entry.d_tag as u64 == *tag {
                    let str_offset = strtab_offset + dyn_entry.d_val as usize;
                    let old_rpath = read_str_at(data, str_offset).to_string();
                    let space = available_space_at(data, str_offset, strtab_end);

                    // Filter rpath entries: keep only dirs that contain needed libs
                    let kept: Vec<&str> = old_rpath
                        .split(':')
                        .filter(|dir| {
                            if dir.is_empty() {
                                return false;
                            }
                            let dir_path = Path::new(dir);
                            for lib in &needed {
                                if dir_path.join(lib).exists() {
                                    return true;
                                }
                            }
                            false
                        })
                        .collect();

                    let new_rpath = kept.join(":");
                    write_str_inplace(data, str_offset, space, &new_rpath);
                    return;
                }
            }
        }
    }
    // No rpath to shrink is not an error
}

fn remove_rpath(elf: &Elf, data: &mut Vec<u8>) {
    let (strtab_offset, strtab_size) = dynstr_info(elf, data);
    let strtab_end = strtab_offset + strtab_size;

    if let Some(dynamic) = &elf.dynamic {
        for tag in &[DT_RUNPATH, DT_RPATH] {
            for dyn_entry in &dynamic.dyns {
                if dyn_entry.d_tag as u64 == *tag {
                    let str_offset = strtab_offset + dyn_entry.d_val as usize;
                    let space = available_space_at(data, str_offset, strtab_end);
                    // Zero out the string
                    write_str_inplace(data, str_offset, space, "");
                }
            }
        }
    }
}

fn set_soname(elf: &Elf, data: &mut Vec<u8>, new_soname: &str) {
    let (strtab_offset, strtab_size) = dynstr_info(elf, data);
    let strtab_end = strtab_offset + strtab_size;

    if let Some(dynamic) = &elf.dynamic {
        for dyn_entry in &dynamic.dyns {
            if dyn_entry.d_tag as u64 == DT_SONAME {
                let str_offset = strtab_offset + dyn_entry.d_val as usize;
                let space = available_space_at(data, str_offset, strtab_end);
                write_str_inplace(data, str_offset, space, new_soname);
                return;
            }
        }
    }

    eprintln!("patchelf: no DT_SONAME found");
    process::exit(1);
}

fn add_needed(_elf: &Elf, _data: &mut Vec<u8>, lib: &str) {
    // Adding a DT_NEEDED entry requires adding a new dynamic entry and potentially
    // growing the string table. This is a complex operation that requires rewriting
    // sections. For now, report it as unsupported.
    eprintln!("patchelf: --add-needed '{lib}' is not yet supported in this version");
    process::exit(1);
}

fn remove_needed(elf: &Elf, data: &mut Vec<u8>, lib: &str) {
    // To "remove" a DT_NEEDED, we can overwrite the dynamic entry's tag with DT_NULL.
    // This is a simplistic approach but works for the common case.
    let is_64 = elf.is_64;
    let is_le = elf.little_endian;

    let (strtab_offset, _strtab_size) = dynstr_info(elf, data);

    if let Some(dynamic) = &elf.dynamic {
        // Find the file offset of the .dynamic section
        let dyn_phdr = elf
            .program_headers
            .iter()
            .find(|ph| ph.p_type == goblin::elf::program_header::PT_DYNAMIC)
            .unwrap_or_else(|| {
                eprintln!("patchelf: no PT_DYNAMIC segment found");
                process::exit(1);
            });

        let dyn_offset = dyn_phdr.p_offset as usize;
        let entry_size = if is_64 { 16usize } else { 8usize };

        for (idx, dyn_entry) in dynamic.dyns.iter().enumerate() {
            if dyn_entry.d_tag as u64 == DT_NEEDED {
                let str_off = strtab_offset + dyn_entry.d_val as usize;
                let name = read_str_at(data, str_off);
                if name == lib {
                    // Overwrite this entry's d_tag with DT_NULL (0)
                    let entry_offset = dyn_offset + idx * entry_size;
                    if is_64 {
                        let tag_bytes = 0u64.to_le_bytes();
                        let val_bytes = 0u64.to_le_bytes();
                        if is_le {
                            data[entry_offset..entry_offset + 8]
                                .copy_from_slice(&tag_bytes);
                            data[entry_offset + 8..entry_offset + 16]
                                .copy_from_slice(&val_bytes);
                        } else {
                            data[entry_offset..entry_offset + 8]
                                .copy_from_slice(&0u64.to_be_bytes());
                            data[entry_offset + 8..entry_offset + 16]
                                .copy_from_slice(&0u64.to_be_bytes());
                        }
                    } else {
                        if is_le {
                            data[entry_offset..entry_offset + 4]
                                .copy_from_slice(&0u32.to_le_bytes());
                            data[entry_offset + 4..entry_offset + 8]
                                .copy_from_slice(&0u32.to_le_bytes());
                        } else {
                            data[entry_offset..entry_offset + 4]
                                .copy_from_slice(&0u32.to_be_bytes());
                            data[entry_offset + 4..entry_offset + 8]
                                .copy_from_slice(&0u32.to_be_bytes());
                        }
                    }
                    return;
                }
            }
        }
    }

    eprintln!("patchelf: DT_NEEDED entry '{lib}' not found");
    process::exit(1);
}

fn replace_needed(elf: &Elf, data: &mut Vec<u8>, old: &str, new: &str) {
    let (strtab_offset, strtab_size) = dynstr_info(elf, data);
    let strtab_end = strtab_offset + strtab_size;

    if let Some(dynamic) = &elf.dynamic {
        for dyn_entry in &dynamic.dyns {
            if dyn_entry.d_tag as u64 == DT_NEEDED {
                let str_offset = strtab_offset + dyn_entry.d_val as usize;
                let name = read_str_at(data, str_offset).to_string();
                if name == old {
                    let space = available_space_at(data, str_offset, strtab_end);
                    write_str_inplace(data, str_offset, space, new);
                    return;
                }
            }
        }
    }

    eprintln!("patchelf: DT_NEEDED entry '{old}' not found");
    process::exit(1);
}
