use object::read::Object;
use object::read::elf::{ElfFile, FileHeader, SectionHeader as _};
use object::read::elf::{Rel as _, Rela as _};
use object::{ObjectSection, ObjectSymbol};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use std::time::SystemTime;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StripMode {
    All,
    Debug,
    Unneeded,
}

#[derive(Debug)]
struct Args {
    mode: StripMode,
    preserve_dates: bool,
    output_file: Option<PathBuf>,
    remove_sections: Vec<String>,
    keep_symbols: Vec<String>,
    strip_symbols: Vec<String>,
    keep_file_symbols: bool,
    verbose: bool,
    files: Vec<PathBuf>,
}

fn print_help() {
    eprintln!(
        "Usage: strip [OPTION]... FILE...
Remove symbols and sections from files.

Options:
  -s, --strip-all            Remove all symbols (default)
  -g, -S, --strip-debug      Remove debugging symbols only
      --strip-unneeded       Remove symbols not needed for relocation
  -p, --preserve-dates       Preserve access and modification dates
  -o FILE, --output-file=FILE
                             Write output to FILE
  -R SECTION, --remove-section=SECTION
                             Remove SECTION from output
  -K NAME, --keep-symbol=NAME
                             Do not strip symbol NAME
  -N NAME, --strip-symbol=NAME
                             Strip symbol NAME
      --keep-file-symbols    Do not strip file symbol(s)
  -v, --verbose              Verbose mode
      --version              Display version information
  -h, --help                 Display this help"
    );
}

fn print_version() {
    eprintln!("strip (rust-strip) {VERSION}");
}

fn parse_args() -> Args {
    let mut args = Args {
        mode: StripMode::All,
        preserve_dates: false,
        output_file: None,
        remove_sections: Vec::new(),
        keep_symbols: Vec::new(),
        strip_symbols: Vec::new(),
        keep_file_symbols: false,
        verbose: false,
        files: Vec::new(),
    };

    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut i = 0;
    let mut mode_set = false;

    while i < raw.len() {
        let arg = &raw[i];
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                process::exit(0);
            }
            "--version" => {
                print_version();
                process::exit(0);
            }
            "-s" | "--strip-all" => {
                args.mode = StripMode::All;
                mode_set = true;
            }
            "-g" | "-S" | "--strip-debug" => {
                args.mode = StripMode::Debug;
                mode_set = true;
            }
            "--strip-unneeded" => {
                args.mode = StripMode::Unneeded;
                mode_set = true;
            }
            "-p" | "--preserve-dates" => {
                args.preserve_dates = true;
            }
            "-o" => {
                i += 1;
                if i >= raw.len() {
                    eprintln!("strip: -o requires an argument");
                    process::exit(1);
                }
                args.output_file = Some(PathBuf::from(&raw[i]));
            }
            "-R" => {
                i += 1;
                if i >= raw.len() {
                    eprintln!("strip: -R requires an argument");
                    process::exit(1);
                }
                args.remove_sections.push(raw[i].clone());
            }
            "-K" => {
                i += 1;
                if i >= raw.len() {
                    eprintln!("strip: -K requires an argument");
                    process::exit(1);
                }
                args.keep_symbols.push(raw[i].clone());
            }
            "-N" => {
                i += 1;
                if i >= raw.len() {
                    eprintln!("strip: -N requires an argument");
                    process::exit(1);
                }
                args.strip_symbols.push(raw[i].clone());
            }
            "--keep-file-symbols" => {
                args.keep_file_symbols = true;
            }
            "-v" | "--verbose" => {
                args.verbose = true;
            }
            _ if arg.starts_with("--output-file=") => {
                let val = arg.strip_prefix("--output-file=").unwrap();
                args.output_file = Some(PathBuf::from(val));
            }
            _ if arg.starts_with("--remove-section=") => {
                let val = arg.strip_prefix("--remove-section=").unwrap();
                args.remove_sections.push(val.to_string());
            }
            _ if arg.starts_with("--keep-symbol=") => {
                let val = arg.strip_prefix("--keep-symbol=").unwrap();
                args.keep_symbols.push(val.to_string());
            }
            _ if arg.starts_with("--strip-symbol=") => {
                let val = arg.strip_prefix("--strip-symbol=").unwrap();
                args.strip_symbols.push(val.to_string());
            }
            _ if arg.starts_with('-') && !arg.starts_with("--") && arg.len() > 2 => {
                let chars: Vec<char> = arg[1..].chars().collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        's' => {
                            args.mode = StripMode::All;
                            mode_set = true;
                        }
                        'g' | 'S' => {
                            args.mode = StripMode::Debug;
                            mode_set = true;
                        }
                        'p' => args.preserve_dates = true,
                        'v' => args.verbose = true,
                        'o' => {
                            if j + 1 < chars.len() {
                                let rest: String = chars[j + 1..].iter().collect();
                                args.output_file = Some(PathBuf::from(rest));
                                j = chars.len();
                                continue;
                            }
                            i += 1;
                            if i >= raw.len() {
                                eprintln!("strip: -o requires an argument");
                                process::exit(1);
                            }
                            args.output_file = Some(PathBuf::from(&raw[i]));
                        }
                        'R' => {
                            if j + 1 < chars.len() {
                                let rest: String = chars[j + 1..].iter().collect();
                                args.remove_sections.push(rest);
                                j = chars.len();
                                continue;
                            }
                            i += 1;
                            if i >= raw.len() {
                                eprintln!("strip: -R requires an argument");
                                process::exit(1);
                            }
                            args.remove_sections.push(raw[i].clone());
                        }
                        'K' => {
                            if j + 1 < chars.len() {
                                let rest: String = chars[j + 1..].iter().collect();
                                args.keep_symbols.push(rest);
                                j = chars.len();
                                continue;
                            }
                            i += 1;
                            if i >= raw.len() {
                                eprintln!("strip: -K requires an argument");
                                process::exit(1);
                            }
                            args.keep_symbols.push(raw[i].clone());
                        }
                        'N' => {
                            if j + 1 < chars.len() {
                                let rest: String = chars[j + 1..].iter().collect();
                                args.strip_symbols.push(rest);
                                j = chars.len();
                                continue;
                            }
                            i += 1;
                            if i >= raw.len() {
                                eprintln!("strip: -N requires an argument");
                                process::exit(1);
                            }
                            args.strip_symbols.push(raw[i].clone());
                        }
                        c => {
                            eprintln!("strip: unrecognized option '-{c}'");
                            process::exit(1);
                        }
                    }
                    j += 1;
                }
            }
            _ if arg.starts_with('-') && arg != "-" => {
                eprintln!("strip: unrecognized option '{arg}'");
                process::exit(1);
            }
            _ => {
                args.files.push(PathBuf::from(arg));
            }
        }
        i += 1;
    }

    if !mode_set && args.remove_sections.is_empty() && args.strip_symbols.is_empty() {
        args.mode = StripMode::All;
    }

    if args.files.is_empty() {
        eprintln!("strip: no input files");
        process::exit(1);
    }

    if args.output_file.is_some() && args.files.len() > 1 {
        eprintln!("strip: -o may not be used with multiple files");
        process::exit(1);
    }

    args
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

fn should_remove_section(name: &str, mode: StripMode, remove_sections: &[String]) -> bool {
    if remove_sections.iter().any(|s| s == name) {
        return true;
    }

    match mode {
        StripMode::Debug | StripMode::Unneeded => is_debug_section(name),
        StripMode::All => is_debug_section(name) || name == ".symtab" || name == ".strtab",
    }
}

fn should_keep_symbol(
    sym: &object::read::Symbol<'_, '_>,
    mode: StripMode,
    keep_symbols: &[String],
    strip_symbols: &[String],
    keep_file_symbols: bool,
    reloc_symbols: &HashSet<object::SymbolIndex>,
) -> bool {
    let name = sym.name().unwrap_or("");

    if keep_symbols.iter().any(|s| s == name) {
        return true;
    }

    if strip_symbols.iter().any(|s| s == name) {
        return false;
    }

    if sym.is_undefined() {
        return true;
    }

    if keep_file_symbols && sym.kind() == object::SymbolKind::File {
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
        collect_reloc_symbols_from_elf(&elf, &mut indices);
    } else if let Ok(elf) = ElfFile::<object::elf::FileHeader32<object::Endianness>>::parse(data) {
        collect_reloc_symbols_from_elf(&elf, &mut indices);
    }

    indices
}

fn collect_reloc_symbols_from_elf<'data, Elf: FileHeader>(
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

fn strip_file(path: &Path, args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let data = fs::read(path)?;

    let timestamps = if args.preserve_dates {
        Some((
            fs::metadata(path)?.accessed().ok(),
            fs::metadata(path)?.modified().ok(),
        ))
    } else {
        None
    };

    let obj = object::File::parse(&*data)?;

    let reloc_symbols = if args.mode == StripMode::Unneeded {
        collect_reloc_symbols(&data)
    } else {
        HashSet::new()
    };

    let out = rewrite_elf(&obj, args, &reloc_symbols)?;

    let output_path = args.output_file.as_deref().unwrap_or(path);
    fs::write(output_path, &out)?;

    if let Some((atime, mtime)) = timestamps
        && let (Some(_atime), Some(mtime)) = (atime, mtime)
    {
        set_file_times(output_path, mtime)?;
    }

    if args.verbose {
        eprintln!("strip: {}", path.display());
    }

    Ok(())
}

fn rewrite_elf(
    obj: &object::File<'_>,
    args: &Args,
    reloc_symbols: &HashSet<object::SymbolIndex>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    match obj.format() {
        object::BinaryFormat::Elf => {}
        _ => return Err("file format not recognized (not ELF)".into()),
    }

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

        if should_remove_section(name, args.mode, &args.remove_sections) {
            continue;
        }

        // The object write crate manages .symtab/.strtab itself
        if name == ".symtab" || name == ".strtab" {
            continue;
        }

        let section_kind = section.kind();
        let flags = section.flags();
        let name_bytes = section.name_bytes()?.to_vec();

        let new_id = builder.add_section(Vec::new(), name_bytes, section_kind);
        builder.section_mut(new_id).flags = flags;

        let section_data = section.uncompressed_data()?;
        if !section_data.is_empty() {
            builder.set_section_data(new_id, section_data.into_owned(), section.align());
        } else if section.align() > 1 {
            builder.set_section_data(new_id, Vec::new(), section.align());
        }

        section_map.insert(section.index(), new_id);
    }

    // Copy symbols unless strip-all with no explicit keep list
    if args.mode != StripMode::All || !args.keep_symbols.is_empty() {
        for symbol in obj.symbols() {
            if symbol.index().0 == 0 {
                continue;
            }

            if !should_keep_symbol(
                &symbol,
                args.mode,
                &args.keep_symbols,
                &args.strip_symbols,
                args.keep_file_symbols,
                reloc_symbols,
            ) {
                continue;
            }

            let name = symbol.name_bytes()?;

            let section = match symbol.section() {
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

            let new_sym = object::write::Symbol {
                name: name.to_vec(),
                value: symbol.address(),
                size: symbol.size(),
                kind: symbol.kind(),
                scope: symbol.scope(),
                weak: symbol.is_weak(),
                section,
                flags: object::SymbolFlags::None,
            };

            builder.add_symbol(new_sym);
        }
    }

    let mut out_buf = Vec::new();
    builder.emit(&mut out_buf)?;
    Ok(out_buf)
}

fn set_file_times(path: &Path, mtime: SystemTime) -> Result<(), Box<dyn std::error::Error>> {
    let file = fs::File::options().write(true).open(path)?;
    file.set_modified(mtime)?;
    Ok(())
}

fn main() {
    let args = parse_args();
    let mut errors = 0;

    for file in &args.files {
        if !file.exists() {
            eprintln!("strip: '{}': No such file", file.display());
            errors += 1;
            continue;
        }

        match strip_file(file, &args) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("strip: {}: {}", file.display(), e);
                errors += 1;
            }
        }
    }

    process::exit(if errors > 0 { 1 } else { 0 });
}
