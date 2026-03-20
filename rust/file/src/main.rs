use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut opts = FileOpts::default();
    let mut files = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--version" | "-v" => {
                println!("file (rust-file) {}", env!("CARGO_PKG_VERSION"));
                process::exit(0);
            }
            "--help" => {
                println!("Usage: file [OPTION...] [FILE...]");
                println!("Determine type of FILEs.");
                println!("  -b, --brief         do not prepend filenames to output");
                println!("  -i, --mime          output MIME type strings");
                println!("  --mime-type         output MIME type only");
                println!("  --mime-encoding     output MIME encoding only");
                println!("  -L, --dereference   follow symlinks");
                println!("  -h, --no-dereference  don't follow symlinks");
                println!("  -z, --uncompress    try to look inside compressed files");
                println!("  -k, --keep-going    don't stop at first match");
                println!("  -p, --preserve-date preserve access times");
                println!("  -s, --special-files read block/char special files");
                println!("  -0, --print0        use NUL as output separator");
                process::exit(0);
            }
            "-b" | "--brief" => opts.brief = true,
            "-i" | "--mime" => {
                opts.mime_type = true;
                opts.mime_encoding = true;
            }
            "--mime-type" => opts.mime_type = true,
            "--mime-encoding" => opts.mime_encoding = true,
            "-L" | "--dereference" => opts.dereference = true,
            "-h" | "--no-dereference" => opts.dereference = false,
            "-z" | "--uncompress" => {}
            "-k" | "--keep-going" => {}
            "-p" | "--preserve-date" => {}
            "-s" | "--special-files" => {}
            "-0" | "--print0" => opts.null_separator = true,
            "-m" | "--magic-file" => {
                i += 1; // skip magic file arg
            }
            "-e" | "--exclude" => {
                i += 1; // skip exclude arg
            }
            "--" => {
                files.extend_from_slice(&args[i + 1..]);
                break;
            }
            arg if arg.starts_with('-') && arg.len() > 1 => {
                // Handle combined flags like -bLi
                for ch in arg[1..].chars() {
                    match ch {
                        'b' => opts.brief = true,
                        'i' => {
                            opts.mime_type = true;
                            opts.mime_encoding = true;
                        }
                        'L' => opts.dereference = true,
                        'h' => opts.dereference = false,
                        'z' | 'k' | 'p' | 's' => {}
                        '0' => opts.null_separator = true,
                        _ => {}
                    }
                }
            }
            _ => files.push(args[i].clone()),
        }
        i += 1;
    }

    if files.is_empty() {
        eprintln!("Usage: file [-bLi] FILE...");
        process::exit(1);
    }

    for file in &files {
        let result = identify_file(file, &opts);
        let sep = if opts.null_separator { '\0' } else { '\n' };
        if opts.brief {
            print!("{result}{sep}");
        } else {
            print!("{file}: {result}{sep}");
        }
    }
}

#[derive(Default)]
struct FileOpts {
    brief: bool,
    mime_type: bool,
    mime_encoding: bool,
    dereference: bool,
    null_separator: bool,
}

fn identify_file(path: &str, opts: &FileOpts) -> String {
    if path == "-" {
        let mut buf = Vec::new();
        if std::io::stdin().read_to_end(&mut buf).is_err() {
            return "cannot read stdin".to_string();
        }
        return identify_data(&buf, "standard input", opts);
    }

    let p = Path::new(path);

    // Check if path exists
    let meta = if opts.dereference {
        fs::metadata(p)
    } else {
        fs::symlink_metadata(p)
    };

    let meta = match meta {
        Ok(m) => m,
        Err(e) => {
            return format!("cannot open `{path}' (No such file or directory): {e}");
        }
    };

    // Symlink (when not dereferencing)
    if meta.file_type().is_symlink() {
        if let Ok(target) = fs::read_link(p) {
            if opts.mime_type {
                return "inode/symlink".to_string();
            }
            return format!("symbolic link to {}", target.display());
        }
    }

    // Directory
    if meta.is_dir() {
        if opts.mime_type {
            return mime_with_encoding("inode/directory", opts);
        }
        return "directory".to_string();
    }

    // Special files
    if !meta.is_file() {
        let ft = meta.file_type();
        if ft.is_symlink() {
            if opts.mime_type {
                return mime_with_encoding("inode/symlink", opts);
            }
            return "symbolic link".to_string();
        }
        // Block/char device, fifo, socket
        let mode = meta.mode();
        if mode & 0o170000 == 0o060000 {
            if opts.mime_type {
                return mime_with_encoding("inode/blockdevice", opts);
            }
            return "block special".to_string();
        }
        if mode & 0o170000 == 0o020000 {
            if opts.mime_type {
                return mime_with_encoding("inode/chardevice", opts);
            }
            return "character special".to_string();
        }
        if mode & 0o170000 == 0o010000 {
            if opts.mime_type {
                return mime_with_encoding("inode/fifo", opts);
            }
            return "fifo (named pipe)".to_string();
        }
        if mode & 0o170000 == 0o140000 {
            if opts.mime_type {
                return mime_with_encoding("inode/socket", opts);
            }
            return "socket".to_string();
        }
        return "special file".to_string();
    }

    // Empty file
    if meta.len() == 0 {
        if opts.mime_type {
            return mime_with_encoding("inode/x-empty", opts);
        }
        return "empty".to_string();
    }

    // Read file header
    let mut buf = [0u8; 8192];
    let n = match fs::File::open(p).and_then(|mut f| f.read(&mut buf)) {
        Ok(n) => n,
        Err(e) => return format!("cannot read: {e}"),
    };

    identify_data(&buf[..n], path, opts)
}

fn identify_data(buf: &[u8], path: &str, opts: &FileOpts) -> String {
    if buf.is_empty() {
        if opts.mime_type {
            return mime_with_encoding("application/x-empty", opts);
        }
        return "empty".to_string();
    }

    // ELF
    if buf.len() >= 18 && &buf[0..4] == b"\x7fELF" {
        return identify_elf(buf, opts);
    }

    // Mach-O
    if buf.len() >= 4
        && (buf[..4] == [0xfe, 0xed, 0xfa, 0xce]
            || buf[..4] == [0xce, 0xfa, 0xed, 0xfe]
            || buf[..4] == [0xfe, 0xed, 0xfa, 0xcf]
            || buf[..4] == [0xcf, 0xfa, 0xed, 0xfe])
    {
        if opts.mime_type {
            return mime_with_encoding("application/x-mach-binary", opts);
        }
        return "Mach-O binary".to_string();
    }

    // Archives
    if buf.len() >= 8 && &buf[0..8] == b"!<arch>\n" {
        if opts.mime_type {
            return mime_with_encoding("application/x-archive", opts);
        }
        return "current ar archive".to_string();
    }

    // Compressed
    if buf.len() >= 2 && buf[0] == 0x1f && buf[1] == 0x8b {
        if opts.mime_type {
            return mime_with_encoding("application/gzip", opts);
        }
        return "gzip compressed data".to_string();
    }
    if buf.len() >= 3 && &buf[0..3] == b"BZh" {
        if opts.mime_type {
            return mime_with_encoding("application/x-bzip2", opts);
        }
        return "bzip2 compressed data".to_string();
    }
    if buf.len() >= 6 && buf[..6] == [0xFD, b'7', b'z', b'X', b'Z', 0x00] {
        if opts.mime_type {
            return mime_with_encoding("application/x-xz", opts);
        }
        return "XZ compressed data".to_string();
    }
    if buf.len() >= 4 && buf[..4] == [0x28, 0xb5, 0x2f, 0xfd] {
        if opts.mime_type {
            return mime_with_encoding("application/zstd", opts);
        }
        return "Zstandard compressed data".to_string();
    }

    // Tar
    if buf.len() >= 265 && &buf[257..262] == b"ustar" {
        if opts.mime_type {
            return mime_with_encoding("application/x-tar", opts);
        }
        return "POSIX tar archive".to_string();
    }

    // Images
    if buf.len() >= 8 && &buf[0..8] == b"\x89PNG\r\n\x1a\n" {
        if opts.mime_type {
            return mime_with_encoding("image/png", opts);
        }
        return "PNG image data".to_string();
    }
    if buf.len() >= 2 && buf[0] == 0xff && buf[1] == 0xd8 {
        if opts.mime_type {
            return mime_with_encoding("image/jpeg", opts);
        }
        return "JPEG image data".to_string();
    }
    if buf.len() >= 4 && &buf[0..4] == b"GIF8" {
        if opts.mime_type {
            return mime_with_encoding("image/gif", opts);
        }
        return "GIF image data".to_string();
    }

    // PDF
    if buf.len() >= 5 && &buf[0..5] == b"%PDF-" {
        if opts.mime_type {
            return mime_with_encoding("application/pdf", opts);
        }
        return "PDF document".to_string();
    }

    // ZIP
    if buf.len() >= 4 && buf[0] == b'P' && buf[1] == b'K' && buf[2] == 3 && buf[3] == 4 {
        if opts.mime_type {
            return mime_with_encoding("application/zip", opts);
        }
        return "Zip archive data".to_string();
    }

    // Java class
    if buf.len() >= 4 && buf[..4] == [0xca, 0xfe, 0xba, 0xbe] {
        if opts.mime_type {
            return mime_with_encoding("application/x-java-applet", opts);
        }
        return "Java class data".to_string();
    }

    // SQLite
    if buf.len() >= 16 && &buf[0..16] == b"SQLite format 3\0" {
        if opts.mime_type {
            return mime_with_encoding("application/x-sqlite3", opts);
        }
        return "SQLite 3.x database".to_string();
    }

    // Scripts (shebang)
    if buf.len() >= 2 && buf[0] == b'#' && buf[1] == b'!' {
        let first_line = buf
            .iter()
            .position(|&b| b == b'\n')
            .map(|pos| &buf[2..pos])
            .unwrap_or(&buf[2..buf.len().min(128)]);
        let interp = String::from_utf8_lossy(first_line).trim().to_string();
        let interp_name = interp
            .split_whitespace()
            .next()
            .unwrap_or(&interp)
            .rsplit('/')
            .next()
            .unwrap_or(&interp);

        if opts.mime_type {
            return mime_with_encoding("text/x-shellscript", opts);
        }

        return match interp_name {
            "bash" | "sh" | "dash" | "zsh" | "ksh" | "ash" => {
                format!("{interp_name} script, ASCII text executable")
            }
            "python" | "python3" | "python2" => "Python script, ASCII text executable".to_string(),
            "perl" => "Perl script text executable".to_string(),
            "ruby" => "Ruby script, ASCII text executable".to_string(),
            "node" | "nodejs" => "Node.js script text executable".to_string(),
            "env" => format!("script, ASCII text executable (env {interp})"),
            _ => format!("{interp_name} script, ASCII text executable"),
        };
    }

    // XML
    if buf.len() >= 5 && &buf[0..5] == b"<?xml" {
        if opts.mime_type {
            return mime_with_encoding("text/xml", opts);
        }
        return "XML document".to_string();
    }

    // HTML
    if buf.len() >= 14 {
        let lower: Vec<u8> = buf[..14.min(buf.len())]
            .iter()
            .map(|b| b.to_ascii_lowercase())
            .collect();
        if lower.starts_with(b"<!doctype html") || lower.starts_with(b"<html") {
            if opts.mime_type {
                return mime_with_encoding("text/html", opts);
            }
            return "HTML document, ASCII text".to_string();
        }
    }

    // Check if it's text
    let is_text = is_text_data(buf);

    if is_text {
        // Detect encoding
        let is_utf8 = std::str::from_utf8(buf).is_ok();

        if opts.mime_type {
            let charset = if is_utf8 { "utf-8" } else { "unknown-8bit" };
            if opts.mime_encoding {
                return format!("text/plain; charset={charset}");
            }
            return "text/plain".to_string();
        }

        // Try to identify text subtypes
        let text = String::from_utf8_lossy(buf);

        // C source
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        match ext {
            "c" | "h" => return "C source, ASCII text".to_string(),
            "cc" | "cpp" | "cxx" | "hpp" => return "C++ source, ASCII text".to_string(),
            "rs" => return "Rust source, ASCII text".to_string(),
            "py" => return "Python script, ASCII text".to_string(),
            "js" => return "JavaScript source, ASCII text".to_string(),
            "json" => return "JSON text data".to_string(),
            "yaml" | "yml" => return "YAML document, ASCII text".to_string(),
            "toml" => return "TOML document, ASCII text".to_string(),
            "md" => return "Markdown document, ASCII text".to_string(),
            "nix" => return "Nix expression, ASCII text".to_string(),
            _ => {}
        }

        // Makefile detection
        if path.contains("Makefile") || path.contains("makefile") || path.ends_with(".mk") {
            return "makefile script, ASCII text".to_string();
        }

        // M4/autoconf
        if text.contains("AC_INIT") || text.contains("AC_PREREQ") {
            return "M4 macro processor script, ASCII text".to_string();
        }

        if is_utf8 {
            return "ASCII text".to_string();
        }
        return "data".to_string();
    }

    // Binary data
    if opts.mime_type {
        return mime_with_encoding("application/octet-stream", opts);
    }
    "data".to_string()
}

fn identify_elf(buf: &[u8], opts: &FileOpts) -> String {
    if opts.mime_type {
        let mime = match buf.get(16) {
            Some(2) => "application/x-executable",
            Some(3) => "application/x-sharedlib",
            Some(1) => "application/x-object",
            Some(4) => "application/x-coredump",
            _ => "application/x-elf",
        };
        return mime_with_encoding(mime, opts);
    }

    let class = match buf.get(4) {
        Some(1) => "32-bit",
        Some(2) => "64-bit",
        _ => "unknown-class",
    };
    let endian = match buf.get(5) {
        Some(1) => "LSB",
        Some(2) => "MSB",
        _ => "unknown-endian",
    };
    let elf_type = match buf.get(16) {
        Some(1) => "relocatable",
        Some(2) => "executable",
        Some(3) => "shared object",
        Some(4) => "core file",
        _ => "unknown type",
    };

    // Read machine type (bytes 18-19, little endian for LSB)
    let machine = if buf.len() >= 20 {
        let m = if endian == "LSB" {
            u16::from_le_bytes([buf[18], buf[19]])
        } else {
            u16::from_be_bytes([buf[18], buf[19]])
        };
        match m {
            3 => "Intel 80386",
            0x3e => "x86-64",
            0x28 => "ARM",
            0xb7 => "ARM aarch64",
            0xf3 => "RISC-V",
            8 => "MIPS",
            0x15 => "PowerPC",
            0x16 => "PowerPC64",
            _ => "unknown arch",
        }
    } else {
        "unknown arch"
    };

    let mut desc = format!("ELF {class} {endian} {elf_type}, {machine}");

    // Check for dynamic linking
    if elf_type == "executable" || elf_type == "shared object" {
        desc.push_str(", dynamically linked");
        // Check for interpreter
        if let Some(interp) = find_elf_interp(buf) {
            desc.push_str(&format!(", interpreter {interp}"));
        }
    }

    // Check if stripped
    // (simplified: look for .symtab section — if absent, it's stripped)
    if !buf.windows(8).any(|w| w == b".symtab\0") {
        desc.push_str(", stripped");
    } else {
        desc.push_str(", not stripped");
    }

    desc
}

fn find_elf_interp(buf: &[u8]) -> Option<String> {
    // Search for PT_INTERP program header
    // Simplified: look for /lib or /nix/store in the first few KB
    let s = String::from_utf8_lossy(buf);
    for segment in s.split('\0') {
        if (segment.starts_with("/lib") || segment.starts_with("/nix/store"))
            && segment.contains("ld-")
        {
            return Some(segment.to_string());
        }
    }
    None
}

fn is_text_data(buf: &[u8]) -> bool {
    // Check if data looks like text (allow UTF-8 and common text bytes)
    let mut non_text = 0;
    for &b in buf {
        if b == 0 {
            return false; // NUL byte means binary
        }
        if b < 0x07 || (b > 0x0d && b < 0x20 && b != 0x1b) {
            non_text += 1;
        }
    }
    // Allow up to 2% non-text bytes (for UTF-8 continuation bytes etc.)
    non_text * 100 < buf.len() * 2
}

fn mime_with_encoding(mime: &str, opts: &FileOpts) -> String {
    if opts.mime_encoding && !mime.contains("charset=") {
        format!("{mime}; charset=binary")
    } else {
        mime.to_string()
    }
}
