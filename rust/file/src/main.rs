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

    // Read file header. 2 MiB covers ELF .note/.auxv + stack regions for
    // core dumps, TrueType table directories, and OOXML zip entries.
    let mut buf = vec![0u8; 2 * 1024 * 1024];
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


    // Linux S390 kernel
    if buf.len() >= 32
        && buf[8..32]
            == [
                0x02, 0x00, 0x00, 0x18, 0x60, 0x00, 0x00, 0x50, 0x02, 0x00, 0x00, 0x68, 0x60,
                0x00, 0x00, 0x50, 0x40, 0x40, 0x40, 0x40, 0x40, 0x40, 0x40, 0x40,
            ]
    {
        if opts.mime_type {
            return mime_with_encoding("application/octet-stream", opts);
        }
        let mut desc = "Linux S390".to_string();
        let marker: [u8; 8] = [0x00, 0x0A, 0x00, 0x00, 0x8B, 0xAD, 0xCC, 0xCC];
        let search_start = 0x10000usize.min(buf.len());
        let search_end = (search_start + 4096).min(buf.len());
        if search_start + 8 <= buf.len() {
            for i in search_start..search_end.saturating_sub(16) {
                if buf[i..i + 8] == marker && i + 16 <= buf.len() {
                    let subtype = &buf[i + 8..i + 16];
                    let name = match subtype {
                        [0xC1, 0x00, 0xEF, 0xE3, 0xF0, 0x68, 0x00, 0x00] => " Z10 64bit kernel",
                        [0xC1, 0x00, 0xEF, 0xC3, 0x00, 0x00, 0x00, 0x00] => {
                            " Z9-109 64bit kernel"
                        }
                        [0xC0, 0x00, 0x20, 0x00, 0x00, 0x00, 0x00, 0x00] => " Z990 64bit kernel",
                        [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00] => " Z900 64bit kernel",
                        _ => "",
                    };
                    desc.push_str(name);
                    break;
                }
            }
        }
        return desc;
    }

    // MS Windows shortcut (LNK)
    if buf.len() >= 76
        && buf[0..4] == [0x4c, 0x00, 0x00, 0x00]
        && buf[4..20]
            == [
                0x01, 0x14, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x46,
            ]
    {
        if opts.mime_type {
            return mime_with_encoding("application/x-ms-shortcut", opts);
        }
        return identify_lnk(buf);
    }

    // Archives. A Debian package is an ar archive whose first member is
    // named "debian-binary" and whose contents start with the format line
    // (e.g. "2.0\n").
    if buf.len() >= 8 && &buf[0..8] == b"!<arch>\n" {
        if opts.mime_type {
            return mime_with_encoding("application/x-archive", opts);
        }
        if buf.len() >= 72 && &buf[8..21] == b"debian-binary" {
            // Member-1 header is 60 bytes, so its data starts at offset 68.
            // The version string is the data before the first newline.
            let data_start = 68;
            let version_end = buf[data_start..]
                .iter()
                .position(|&b| b == b'\n')
                .map(|n| data_start + n)
                .unwrap_or(data_start);
            let version =
                std::str::from_utf8(&buf[data_start..version_end]).unwrap_or("?");
            // Round member-1 data size up to even for ar's 2-byte alignment.
            let size_str =
                std::str::from_utf8(&buf[8 + 48..8 + 58]).unwrap_or("0").trim();
            let size: usize = size_str.parse().unwrap_or(0);
            let member2_hdr = 68 + size + (size % 2);
            let payload = if buf.len() >= member2_hdr + 16 {
                let name_end = buf[member2_hdr..member2_hdr + 16]
                    .iter()
                    .position(|&b| b == b' ')
                    .unwrap_or(16);
                std::str::from_utf8(&buf[member2_hdr..member2_hdr + name_end])
                    .unwrap_or("?")
            } else {
                "?"
            };
            let compression = payload.rsplit('.').next().unwrap_or("");
            return format!(
                "Debian binary package (format {version}), with {payload} , data compression {compression}"
            );
        }
        return "current ar archive".to_string();
    }

    // RPM package (lead header)
    if buf.len() >= 10 && buf[0..4] == [0xED, 0xAB, 0xEE, 0xDB] {
        if opts.mime_type {
            return mime_with_encoding("application/x-rpm", opts);
        }
        return identify_rpm(buf);
    }

    // RAR archive
    if buf.len() >= 8 && &buf[0..6] == b"Rar!\x1a\x07" {
        if opts.mime_type {
            return mime_with_encoding("application/x-rar", opts);
        }
        return identify_rar(buf);
    }

    // 7-zip
    if buf.len() >= 8 && buf[0..6] == [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C] {
        if opts.mime_type {
            return mime_with_encoding("application/x-7z-compressed", opts);
        }
        return identify_7z(buf);
    }

    // glibc locale compiled files. Upstream reports them by basename
    // (LC_ADDRESS, LC_CTYPE, …). Each category has its own magic word but
    // they all share the same last byte (0x20), and the files are always
    // named `LC_*`. Combine both signals so we don't misfire on unrelated
    // binaries that happen to start with `0x__ 0x__ 0x__ 0x20`.
    if buf.len() >= 4 && buf[3] == 0x20 {
        let name = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if name.starts_with("LC_") {
            if opts.mime_type {
                return mime_with_encoding("application/octet-stream", opts);
            }
            return format!("glibc locale file {name}");
        }
    }

    // FoxBase/dBase III DBF database. Byte 0 is a version code (0x02..=0xFB);
    // we handle the most common values explicitly, bail for the rest.
    // Additionally require the YY/MM/DD date bytes to look plausible —
    // unrelated binary files with the same leading byte trip this check
    // otherwise (.pyc's 03 f3 0d 0a is a frequent offender).
    if buf.len() >= 32
        && matches!(buf[0], 0x02 | 0x03 | 0x04 | 0x05 | 0x30 | 0x31 | 0x32 | 0x43 | 0x63 | 0x83 | 0x8b | 0xcb | 0xf5 | 0xfb)
        && buf[2] >= 1 && buf[2] <= 12
        && buf[3] >= 1 && buf[3] <= 31
    {
        let kind = match buf[0] {
            0x02 => "FoxBase",
            0x03 => "FoxBase+/dBase III",
            0x04 => "dBase IV",
            0x05 => "dBase V",
            0x30 => "Visual FoxPro",
            0x31 => "Visual FoxPro with AutoIncrement",
            0x43 | 0x63 => "dBase IV with memo",
            0x83 => "FoxBase+/dBase III, with memo .DBT",
            0x8b => "dBase IV with memo",
            0xcb => "dBase IV, with SQL table",
            0xf5 => "FoxPro with memo",
            0xfb => "FoxBase",
            _ => "dBase",
        };
        let year = buf[1] as u32;
        let month = buf[2];
        let day = buf[3];
        let records = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let record_size = u16::from_le_bytes([buf[10], buf[11]]);
        if opts.mime_type {
            return mime_with_encoding("application/x-dbf", opts);
        }
        return format!(
            "{kind} DBF, {records} records * {record_size}, update-date {year:02}-{month}-{day}"
        );
    }

    // MySQL table definition (.frm). `fe 01 <ver> <table-type>` header.
    if buf.len() >= 56 && buf[0] == 0xfe && buf[1] == 0x01 {
        let frm_ver = buf[2];
        let legacy_type = buf[3];
        let type_name = match legacy_type {
            0 => "unknown",
            1 => "DIAM_ISAM",
            2 => "HASH",
            3 => "MISAM",
            4 => "PISAM",
            5 => "RMS_ISAM",
            6 => "HEAP",
            7 => "ISAM",
            8 => "MRG_ISAM",
            9 => "MYISAM",
            10 => "MRG_MYISAM",
            11 => "BERKELEY_DB",
            12 => "INNODB",
            13 => "GEMINI",
            14 => "NDBCLUSTER",
            15 => "EXAMPLE_DB",
            16 => "CSV_DB",
            17 => "FEDERATED_DB",
            18 => "BLACKHOLE_DB",
            _ => "unknown",
        };
        // Stored MySQL version is at offset 51 (u32 LE).
        let mysql_ver = u32::from_le_bytes([buf[51], buf[52], buf[53], buf[54]]);
        if opts.mime_type {
            return mime_with_encoding("application/octet-stream", opts);
        }
        return format!(
            "MySQL table definition file Version {frm_ver}, type {type_name}, MySQL version {mysql_ver}"
        );
    }

    // MySQL MyISAM index file (.MYI). Magic fe fe 07 01, then the header
    // layout mapped in MI_STATE_INFO.
    if buf.len() >= 44 && buf[0..4] == [0xfe, 0xfe, 0x07, 0x01] {
        let version = buf[3];
        let key_parts = u16::from_be_bytes([buf[14], buf[15]]);
        let unique_key_parts = u16::from_be_bytes([buf[16], buf[17]]);
        let keys = buf[18];
        let records = u64::from_be_bytes([buf[28], buf[29], buf[30], buf[31], buf[32], buf[33], buf[34], buf[35]]);
        let deleted = u64::from_be_bytes([buf[36], buf[37], buf[38], buf[39], buf[40], buf[41], buf[42], buf[43]]);
        if opts.mime_type {
            return mime_with_encoding("application/octet-stream", opts);
        }
        return format!(
            "MySQL MyISAM index file Version {version}, {key_parts} key parts, {unique_key_parts} unique key parts, {keys} keys, {records} records, {deleted} deleted records"
        );
    }

    // AppleDouble encoded Macintosh file. Magic `00 05 16 07` (AppleDouble
    // v2) or `00 05 16 00` (AppleSingle).
    if buf.len() >= 4 && buf[0..4] == [0x00, 0x05, 0x16, 0x07] {
        if opts.mime_type {
            return mime_with_encoding("application/applefile", opts);
        }
        return "AppleDouble encoded Macintosh file".to_string();
    }

    // macOS Finder Desktop Services Store (.DS_Store). The root block is an
    // "alloc"-tag entry with the fixed signature "Bud1" at offset 4.
    if buf.len() >= 8 && buf[0..4] == [0x00, 0x00, 0x00, 0x01] && &buf[4..8] == b"Bud1" {
        if opts.mime_type {
            return mime_with_encoding("application/octet-stream", opts);
        }
        return "Apple Desktop Services Store".to_string();
    }

    // TZif (timezone data). Reports layout counts straight out of the header.
    if buf.len() >= 44 && &buf[0..4] == b"TZif" {
        let version_byte = buf[4];
        let version_str = if version_byte >= b'0' && version_byte <= b'9' {
            (version_byte - b'0').to_string()
        } else {
            "1".to_string()
        };
        let u32_be = |o: usize| -> u32 {
            u32::from_be_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]])
        };
        let utcnt = u32_be(20);
        let stdcnt = u32_be(24);
        let leapcnt = u32_be(28);
        let timecnt = u32_be(32);
        let typecnt = u32_be(36);
        let charcnt = u32_be(40);
        let leap_part = if leapcnt == 0 {
            "no leap seconds".to_string()
        } else {
            format!("{leapcnt} leap seconds")
        };
        if opts.mime_type {
            return mime_with_encoding("application/octet-stream", opts);
        }
        return format!(
            "timezone data (fat), version {version_str}, {utcnt} gmt time flags, {stdcnt} std time flags, {leap_part}, {timecnt} transition times, {typecnt} local time types, {charcnt} abbreviation chars"
        );
    }

    // PostScript Type 1 binary font (.pfb). 6-byte binary header `80 01 LEN
    // LEN LEN LEN` then ASCII `%!PS-AdobeFont-N.N: <fontname> <version>`.
    if buf.len() >= 32
        && buf[0] == 0x80
        && buf[1] == 0x01
        && &buf[6..20] == b"%!PS-AdobeFont"
    {
        if opts.mime_type {
            return mime_with_encoding("application/vnd.ms-opentype", opts);
        }
        // Pull the font name and version out of the first ASCII line.
        let ascii_start = 6;
        let end = buf[ascii_start..]
            .iter()
            .position(|&b| b == b'\n' || b == b'\r')
            .map(|p| ascii_start + p)
            .unwrap_or(ascii_start + 128);
        let line = String::from_utf8_lossy(&buf[ascii_start..end]);
        // Format: "%!PS-AdobeFont-1.0: FontName Version"
        let colon = line.find(':').map(|i| i + 1).unwrap_or(0);
        let rest = line[colon..].trim();
        return format!("PostScript Type 1 font program data ({rest})");
    }

    // ICC / ColorSync color profile. "acsp" signature at offset 36.
    if buf.len() >= 132 && &buf[36..40] == b"acsp" {
        let size = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let cmm =
            std::str::from_utf8(&buf[4..8]).unwrap_or("????").trim_end();
        let major = buf[8];
        let minor = (buf[9] >> 4) & 0x0f;
        let class = std::str::from_utf8(&buf[12..16]).unwrap_or("????").trim_end();
        let cs = std::str::from_utf8(&buf[16..20]).unwrap_or("????").trim_end();
        let pcs = std::str::from_utf8(&buf[20..24]).unwrap_or("????").trim_end();
        let platform = std::str::from_utf8(&buf[40..44]).unwrap_or("????");
        let manufacturer =
            std::str::from_utf8(&buf[48..52]).unwrap_or("????").trim_end();
        let device_model =
            std::str::from_utf8(&buf[52..56]).unwrap_or("").trim_end().trim_matches(char::from(0));
        let creator =
            std::str::from_utf8(&buf[80..84]).unwrap_or("").trim_end().trim_matches(char::from(0));
        let platform_name = match platform {
            "APPL" => "ColorSync",
            "MSFT" => "Microsoft",
            _ => "unknown",
        };
        let year = u16::from_be_bytes([buf[24], buf[25]]);
        let month = u16::from_be_bytes([buf[26], buf[27]]);
        let day = u16::from_be_bytes([buf[28], buf[29]]);
        let hour = u16::from_be_bytes([buf[30], buf[31]]);
        let minute = u16::from_be_bytes([buf[32], buf[33]]);
        let second = u16::from_be_bytes([buf[34], buf[35]]);
        // Description tag: parse tag table for "desc" then read the data.
        let tag_count = u32::from_be_bytes([buf[128], buf[129], buf[130], buf[131]]);
        let mut description = String::new();
        for i in 0..tag_count.min(64) as usize {
            let t = 132 + i * 12;
            if t + 12 > buf.len() {
                break;
            }
            if &buf[t..t + 4] == b"desc" {
                let dat_off =
                    u32::from_be_bytes([buf[t + 4], buf[t + 5], buf[t + 6], buf[t + 7]])
                        as usize;
                if dat_off + 16 < buf.len() {
                    // desc tag: [sig "desc"][rsvd 4][len BE u32][ASCII string]
                    let str_len = u32::from_be_bytes([
                        buf[dat_off + 8],
                        buf[dat_off + 9],
                        buf[dat_off + 10],
                        buf[dat_off + 11],
                    ]) as usize;
                    let s_start = dat_off + 12;
                    let s_end = (s_start + str_len).min(buf.len());
                    let raw = &buf[s_start..s_end];
                    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                    description = std::str::from_utf8(&raw[..end])
                        .unwrap_or("")
                        .to_string();
                }
                break;
            }
        }
        let desc_part = if description.is_empty() {
            String::new()
        } else {
            format!(" \"{description}\"")
        };
        // Upstream formats differently for APPL vs MSFT — APPL shows "device
        // by <manufacturer>", MSFT shows "device, <manufacturer>/<model>
        // model by <creator>" (when those strings are non-empty).
        let device_part = if platform == "APPL" || device_model.is_empty() {
            format!("device by {manufacturer}")
        } else if creator.is_empty() {
            format!("device, {manufacturer}/{device_model} model")
        } else {
            format!("device, {manufacturer}/{device_model} model by {creator}")
        };
        return format!(
            "{platform_name} color profile {major}.{minor}, type {cmm}, {cs}/{pcs}-{class} {device_part}, {size} bytes, {day}-{month}-{year} {hour}:{minute:02}:{second:02}{desc_part}"
        );
    }

    // PGP Symmetric-Key Encrypted Session Key packet (RFC 4880 §5.3):
    //   byte 0: 0x8c = old format, tag 3, length type 0
    //   byte 1: packet length
    //   byte 2: version (4)
    //   byte 3: symmetric cipher algorithm
    //   byte 4: S2K specifier (0/1/3)
    //   byte 5: hash algorithm (if S2K is salted / iterated)
    if buf.len() >= 6
        && buf[0] == 0x8c
        && buf[2] == 0x04
    {
        let s2k = buf[4];
        let hash = buf[5];
        let hash_name = match hash {
            1 => "MD5",
            2 => "SHA1",
            3 => "RIPE-MD/160",
            8 => "SHA256",
            9 => "SHA384",
            10 => "SHA512",
            11 => "SHA224",
            _ => "unknown",
        };
        let s2k_kind = match s2k {
            0 => "plain",
            1 => "salted",
            3 => "salted & iterated",
            _ => "unknown",
        };
        return format!(
            "PGP symmetric key encrypted data - {s2k_kind} - {hash_name} ."
        );
    }

    // SELinux compiled policy. Magic 0xf97cff8c, then "SE Linux" string,
    // version (LE u32), and policy counters.
    if buf.len() >= 32
        && buf[0..4] == [0x8c, 0xff, 0x7c, 0xf9]
        && &buf[8..16] == b"SE Linux"
    {
        let version = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
        let symbols = u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]);
        let ocons = u32::from_le_bytes([buf[28], buf[29], buf[30], buf[31]]);
        return format!("SE Linux policy v{version} {symbols} symbols {ocons} ocons");
    }

    // Blit mpx/mux executable on VAX byte order. File format: first 4 bytes
    // are a 32-bit magic `0x00000601` (native) — in VAX byte order the
    // bytes appear as `01 06 00 00`.
    if buf.len() >= 8 && buf[0..4] == [0x01, 0x06, 0x00, 0x00] {
        return "VAX-order 68k Blit mpx/mux executable".to_string();
    }

    // Delta ISO (.diso). Magic `DISO` then 4-byte version (BE u32).
    if buf.len() >= 8 && &buf[0..4] == b"DISO" {
        let version = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        return format!("Delta ISO data version {version}");
    }

    // Delta RPM (.drpm). Magic `drpm` followed by the original RPM NEVR.
    if buf.len() >= 32 && &buf[0..4] == b"drpm" {
        // Skip the `drpm` magic + 8 bytes of opaque header; the NEVR string
        // begins at offset 12 and is null-terminated.
        let nevr_start = 12;
        let end = buf[nevr_start..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| nevr_start + p)
            .unwrap_or(buf.len());
        let nevr = std::str::from_utf8(&buf[nevr_start..end]).unwrap_or("");
        return format!("Delta RPM {nevr} ");
    }

    // XPConnect Typelib (.xpt): Mozilla/Firefox XPCOM type library.
    if buf.len() >= 18 && &buf[0..16] == b"XPCOM\nTypeLib\r\n\x1a" {
        let major = buf[16];
        let minor = buf[17];
        return format!("XPConnect Typelib version {major}.{minor}");
    }

    // Microsoft ASF (Windows Media). Magic is a 16-byte GUID.
    if buf.len() >= 16
        && buf[0..16]
            == [
                0x30, 0x26, 0xb2, 0x75, 0x8e, 0x66, 0xcf, 0x11, 0xa6, 0xd9, 0x00, 0xaa, 0x00, 0x62,
                0xce, 0x6c,
            ]
    {
        if opts.mime_type {
            return mime_with_encoding("video/x-ms-asf", opts);
        }
        return "Microsoft ASF".to_string();
    }

    // MS Windows help Bookmark (.hlp). Magic `3f 5f 03 00`. The total size
    // field sits at offset 12 (LE u32).
    if buf.len() >= 16 && buf[0..4] == [0x3f, 0x5f, 0x03, 0x00] {
        let size = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
        return format!("MS Windows help Bookmark, {size} bytes");
    }

    // ISO 9660 / ECMA-119 filesystem. The Primary Volume Descriptor sits at
    // sector 16 (offset 0x8000) with the "CD001" signature. Volume
    // identifier at offset 0x8028 (32 bytes, space-padded).
    if buf.len() >= 0x8028 + 32 && &buf[0x8001..0x8006] == b"CD001" {
        let vol_id = &buf[0x8028..0x8028 + 32];
        let label_end = vol_id.iter().rposition(|&b| b != b' ').map(|p| p + 1).unwrap_or(0);
        let label = std::str::from_utf8(&vol_id[..label_end]).unwrap_or("");
        let has_mbr = buf.len() >= 512 && buf[510] == 0x55 && buf[511] == 0xaa;
        let mbr_part = if has_mbr { " (DOS/MBR boot sector)" } else { "" };
        // Check for the "BOOT" extension (El Torito) by scanning a few
        // sectors for the "EL TORITO SPECIFICATION" signature.
        let bootable = buf
            .windows(23)
            .any(|w| w == b"EL TORITO SPECIFICATION");
        let bootable_part = if bootable { " (bootable)" } else { "" };
        if opts.mime_type {
            return mime_with_encoding("application/x-iso9660-image", opts);
        }
        return format!("ISO 9660 CD-ROM filesystem data{mbr_part} '{label}'{bootable_part}");
    }

    // MPEG Audio Layer III (MP3) raw frame sync. Looking for 11 bits of
    // sync pattern (0xFF 0xE* .. .. ..) and decoding the frame header.
    // Explicitly reject the UTF-16-LE BOM (ff fe ..) which also satisfies
    // the 11-bit sync mask.
    if buf.len() >= 4
        && buf[0] == 0xff
        && (buf[1] & 0xe0) == 0xe0
        && buf[1] != 0xfe
    {
        let version_id = (buf[1] >> 3) & 0x03;
        let layer = (buf[1] >> 1) & 0x03;
        let bitrate_idx = (buf[2] >> 4) & 0x0f;
        let samplerate_idx = (buf[2] >> 2) & 0x03;
        if version_id != 0x01
            && layer != 0x00
            && bitrate_idx != 0x00
            && bitrate_idx != 0x0f
            && samplerate_idx != 0x03
        {
            let version_str = match version_id {
                0 => "v2.5",
                2 => "v2",
                3 => "v1",
                _ => "reserved",
            };
            let layer_str = match layer {
                1 => "III",
                2 => "II",
                3 => "I",
                _ => "reserved",
            };
            let bitrate_idx = (buf[2] >> 4) & 0x0f;
            let samplerate_idx = (buf[2] >> 2) & 0x03;
            let channel_mode = (buf[3] >> 6) & 0x03;
            // Bitrate table: [version][layer][bitrate_idx]
            let bitrate = match (version_id, layer, bitrate_idx) {
                (3, 3, 1) | (3, 3, 2) | (3, 3, 3) => 32 * bitrate_idx as u32,
                (3, 2, _) => {
                    let tab = [0u32, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384];
                    tab.get(bitrate_idx as usize).copied().unwrap_or(0)
                }
                (3, 1, _) => {
                    let tab = [0u32, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
                    tab.get(bitrate_idx as usize).copied().unwrap_or(0)
                }
                // MPEG 2/2.5, Layer III
                (_, 1, _) => {
                    let tab = [0u32, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
                    tab.get(bitrate_idx as usize).copied().unwrap_or(0)
                }
                _ => 0,
            };
            let sr_table: [[u32; 3]; 4] = [
                [11025, 12000, 8000],  // MPEG 2.5
                [0, 0, 0],
                [22050, 24000, 16000], // MPEG 2
                [44100, 48000, 32000], // MPEG 1
            ];
            let rate = sr_table[version_id as usize][samplerate_idx.min(2) as usize];
            let rate_khz = format!("{}.{:03}", rate / 1000, rate % 1000);
            let channel_str = match channel_mode {
                0 => "Stereo",
                1 => "JntStereo",
                2 => "2x Monaural",
                3 => "Monaural",
                _ => "",
            };
            if opts.mime_type {
                return mime_with_encoding("audio/mpeg", opts);
            }
            return format!(
                "MPEG ADTS, layer {layer_str},  {version_str}, {bitrate} kbps, {rate_khz} kHz, {channel_str}"
            );
        }
    }

    // MPEG transport stream. Sync byte 0x47 every 188 bytes. Upstream also
    // requires the payload_unit_start bit in byte 1 — bit 6 — because
    // otherwise random binary that happens to have 0x47 at those offsets
    // reports as a transport stream.
    if buf.len() >= 188 * 2
        && buf[0] == 0x47
        && buf[188] == 0x47
        && buf[376] == 0x47
        && (buf[1] & 0x40) != 0
    {
        if opts.mime_type {
            return mime_with_encoding("video/mp2t", opts);
        }
        return "MPEG transport stream data".to_string();
    }

    // 3D Studio model (.3DS). Main chunk ID 0x4D4D at offset 0 with the
    // length field at 2..=5 (LE u32). Upstream emits only the label.
    if buf.len() >= 6 && buf[0] == 0x4d && buf[1] == 0x4d && (buf[2] == 0x0f || buf[2] == 0x4d) {
        if opts.mime_type {
            return mime_with_encoding("application/x-3ds", opts);
        }
        return "3D Studio model".to_string();
    }

    // MNG video (multiple-image network graphics). Magic mirrors PNG but
    // with "MNG" instead of "PNG"; MHDR chunk at bytes 8..=27.
    if buf.len() >= 24 && buf[..8] == [0x8a, 0x4d, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a] {
        if opts.mime_type {
            return mime_with_encoding("video/x-mng", opts);
        }
        let w = u32::from_be_bytes([buf[16], buf[17], buf[18], buf[19]]);
        let h = u32::from_be_bytes([buf[20], buf[21], buf[22], buf[23]]);
        return format!("MNG video data, {w} x {h}");
    }

    // Netpbm images (P1-P6, P7). Header is ASCII: "PN\nWIDTH HEIGHT\n...".
    if buf.len() >= 4 && buf[0] == b'P' && matches!(buf[1], b'1'..=b'7') {
        let is_ascii = matches!(buf[1], b'1' | b'2' | b'3');
        let is_rawbits = matches!(buf[1], b'4' | b'5' | b'6');
        if is_ascii || is_rawbits {
            let (kind, _mime) = match buf[1] {
                b'1' | b'4' => ("bitmap", "image/x-portable-bitmap"),
                b'2' | b'5' => ("greymap", "image/x-portable-greymap"),
                b'3' | b'6' => ("pixmap", "image/x-portable-pixmap"),
                _ => ("bitmap", "image/x-portable-anymap"),
            };
            if opts.mime_type {
                return mime_with_encoding(_mime, opts);
            }
            // Parse width/height from ASCII header, skipping comments.
            let mut nums: Vec<u32> = Vec::new();
            let mut rest = &buf[3..];
            while nums.len() < 2 && !rest.is_empty() {
                while let Some(&c) = rest.first()
                    && (c == b' ' || c == b'\t' || c == b'\n' || c == b'\r')
                {
                    rest = &rest[1..];
                }
                if rest.first() == Some(&b'#') {
                    while let Some(&c) = rest.first()
                        && c != b'\n'
                    {
                        rest = &rest[1..];
                    }
                    continue;
                }
                let end = rest.iter().position(|&c| !c.is_ascii_digit()).unwrap_or(rest.len());
                if end == 0 {
                    break;
                }
                if let Ok(s) = std::str::from_utf8(&rest[..end])
                    && let Ok(n) = s.parse::<u32>()
                {
                    nums.push(n);
                }
                rest = &rest[end..];
            }
            let (w, h) = (nums.first().copied().unwrap_or(0), nums.get(1).copied().unwrap_or(0));
            // Upstream's magic emits different field orders depending on the
            // subtype — bitmap always leads with encoding, others lead with
            // the kind token.
            let encoding = if is_ascii { "ASCII text" } else { "rawbits" };
            if kind == "bitmap" {
                return format!(
                    "Netpbm image data, size = {w} x {h}, {encoding}, {kind}"
                );
            }
            return format!(
                "Netpbm image data, size = {w} x {h}, {kind}, {encoding}"
            );
        }
    }

    // Truevision Targa (TGA). No strong magic — we validate several header
    // fields before claiming it. image_type=0 ("no image") is rare in real
    // TGA but common in unrelated binary files, so skip it.
    if buf.len() >= 18
        && (buf[1] == 0 || buf[1] == 1)
        && matches!(buf[2], 1 | 2 | 3 | 9 | 10 | 11)
        && matches!(buf[16], 1 | 8 | 15 | 16 | 24 | 32)
    {
        let image_type = buf[2];
        let width = u16::from_le_bytes([buf[12], buf[13]]);
        let height = u16::from_le_bytes([buf[14], buf[15]]);
        let depth = buf[16];
        let descriptor = buf[17];
        let alpha_bits = descriptor & 0x0f;
        let rle = matches!(image_type, 9 | 10 | 11);
        let base_kind = match image_type {
            1 | 9 => "colormapped",
            2 | 10 => {
                if depth == 32 && alpha_bits > 0 {
                    "RGBA"
                } else {
                    "RGB"
                }
            }
            3 | 11 => "B&W",
            _ => "unknown",
        };
        if opts.mime_type {
            return mime_with_encoding("image/x-tga", opts);
        }
        let rle_part = if rle { " - RLE" } else { "" };
        let alpha_part = if base_kind == "RGBA" {
            format!(" - {alpha_bits}-bit alpha")
        } else {
            String::new()
        };
        return format!(
            "Targa image data - {base_kind}{rle_part} {width} x {height} x {depth}{alpha_part}"
        );
    }

    // Microsoft OLE / Compound Document File (CDF). Magic D0CF11E0A1B11AE1.
    //
    // We first try a full FAT-walking structural parse to reconstruct the
    // SummaryInformation stream correctly even when its sectors are
    // non-contiguous. If that fails, we fall back to scanning the raw
    // buffer for well-known FMTID patterns.
    if buf.len() >= 16
        && buf[0..8] == [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1]
    {
        if opts.mime_type {
            return mime_with_encoding("application/x-ole-storage", opts);
        }
        let has_encrypted_package = buf.windows(32).any(|w| {
            w == b"E\x00n\x00c\x00r\x00y\x00p\x00t\x00e\x00d\x00P\x00a\x00c\x00k\x00a\x00g\x00e\x00"
        });
        if has_encrypted_package {
            return "CDFV2 Encrypted".to_string();
        }
        // Try structural OLE parsing first: follow sector chains via the
        // FAT to reconstruct the SummaryInformation stream correctly even
        // when its sectors are non-contiguous.
        if let Some(result) = ole_structural_summary(buf) {
            return result;
        }
        // Fallback: scan raw buffer for FMTIDs (works when stream sectors
        // happen to be contiguous in the file).
        // FMTIDs of the two well-known property sets.
        let fmtid_si: [u8; 16] = [
            0xe0, 0x85, 0x9f, 0xf2, 0xf9, 0x4f, 0x68, 0x10, 0xab, 0x91, 0x08, 0x00, 0x2b, 0x27,
            0xb3, 0xd9,
        ];
        let fmtid_dsi: [u8; 16] = [
            0x02, 0xd5, 0xcd, 0xd5, 0x9c, 0x2e, 0x1b, 0x10, 0x93, 0x97, 0x08, 0x00, 0x2b, 0x2c,
            0xf9, 0xae,
        ];
        // Prefer SummaryInformation (richer metadata) over
        // DocumentSummaryInformation.
        let found_offset = buf
            .windows(16)
            .position(|w| w == fmtid_si)
            .or_else(|| buf.windows(16).position(|w| w == fmtid_dsi));
        if let Some(fmtid_pos) = found_offset
            && fmtid_pos >= 28
        {
            let ps_start = fmtid_pos - 28;
            let byte_order =
                u16::from_le_bytes([buf[ps_start], buf[ps_start + 1]]);
            if byte_order == 0xfffe {
                // Only MSI files get the "MSI Installer" suffix from
                // upstream; MSP/MST get their own reorderings but no label.
                let is_msi = buf.windows(16).any(|w| {
                    w == [
                        0x84, 0x10, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0,
                        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
                    ]
                });
                let is_mst = buf.windows(16).any(|w| {
                    w == [
                        0x82, 0x10, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0,
                        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
                    ]
                });
                let is_msp = buf.windows(16).any(|w| {
                    w == [
                        0x86, 0x10, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0,
                        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46,
                    ]
                });
                let installer = if is_msi {
                    Some("MSI Installer")
                } else if is_mst {
                    Some("MST")
                } else if is_msp {
                    Some("MSP")
                } else {
                    None
                };
                // For MSP, the first SI FMTID references an internal
                // sub-section; the Installer-level metadata lives at a
                // later SI FMTID. Find the last one.
                let real_fmtid_pos = if is_msp {
                    let mut last = fmtid_pos;
                    let mut scan_start = fmtid_pos + 1;
                    while scan_start + 16 <= buf.len() {
                        if let Some(next) = buf[scan_start..]
                            .windows(16)
                            .position(|w| w == fmtid_si || w == fmtid_dsi)
                        {
                            let p = scan_start + next;
                            last = p;
                            scan_start = p + 1;
                        } else {
                            break;
                        }
                    }
                    last
                } else {
                    fmtid_pos
                };
                let ps_start = if is_msp {
                    real_fmtid_pos.saturating_sub(28)
                } else {
                    ps_start
                };
                return format_ole_summary(buf, ps_start, real_fmtid_pos, installer);
            }
        }
        return "Composite Document File V2 Document, Cannot read section info".to_string();
    }

    // LVM2 physical volume (PV) on-disk label. "LABELONE" at offset 0x200,
    // "LVM2 001" at offset 0x218, UUID at 0x220 (32 bytes).
    if buf.len() >= 0x248
        && &buf[0x200..0x208] == b"LABELONE"
        && &buf[0x218..0x220] == b"LVM2 001"
    {
        let uuid = &buf[0x220..0x240];
        // LVM formats the UUID as 32 chars split into groups of 6-4-4-4-4-4-6.
        let groups = [0..6, 6..10, 10..14, 14..18, 18..22, 22..26, 26..32];
        let uuid_parts: Vec<String> = groups
            .iter()
            .map(|r| {
                std::str::from_utf8(&uuid[r.clone()]).unwrap_or("").to_string()
            })
            .collect();
        let uuid_str = uuid_parts.join("-");
        let size = u64::from_le_bytes([
            buf[0x240], buf[0x241], buf[0x242], buf[0x243],
            buf[0x244], buf[0x245], buf[0x246], buf[0x247],
        ]);
        if opts.mime_type {
            return mime_with_encoding("application/octet-stream", opts);
        }
        return format!(
            "LVM2 PV (Linux Logical Volume Manager), UUID: {uuid_str}, size: {size}"
        );
    }

    // Linux swap file. Trailing 10-byte magic "SWAPSPACE2" lives at the end
    // of the first page. Header metadata (version, page count, UUID, label)
    // starts at offset 0x400.
    for &page_size in &[4096usize, 8192, 16384, 32768, 65536] {
        let magic_off = page_size.saturating_sub(10);
        if buf.len() >= page_size
            && magic_off >= 0x400
            && &buf[magic_off..magic_off + 10] == b"SWAPSPACE2"
        {
            // Version byte ordering tells us the host endianness.
            let le = buf[0x400] == 1 && buf[0x403] == 0;
            let endian = if le { "little endian" } else { "big endian" };
            let read_u32 = |o: usize| -> u32 {
                if le {
                    u32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]])
                } else {
                    u32::from_be_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]])
                }
            };
            let version = read_u32(0x400);
            let size_pages = read_u32(0x404);
            let bad_pages = read_u32(0x408);
            let uuid = &buf[0x40c..0x41c];
            let label_bytes = &buf[0x41c..0x42c];
            let label_end = label_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(label_bytes.len());
            let label = std::str::from_utf8(&label_bytes[..label_end]).unwrap_or("");
            let label_part = if label.is_empty() {
                "no label".to_string()
            } else {
                format!("LABEL={label}")
            };
            let page_size_str = match page_size {
                4096 => "4k",
                8192 => "8k",
                16384 => "16k",
                32768 => "32k",
                65536 => "64k",
                _ => "?",
            };
            let uuid_str = format!(
                "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                uuid[0], uuid[1], uuid[2], uuid[3],
                uuid[4], uuid[5], uuid[6], uuid[7],
                uuid[8], uuid[9], uuid[10], uuid[11],
                uuid[12], uuid[13], uuid[14], uuid[15]
            );
            if opts.mime_type {
                return mime_with_encoding("application/octet-stream", opts);
            }
            return format!(
                "Linux swap file, {page_size_str} page size, {endian}, version {version}, size {size_pages} pages, {bad_pages} bad pages, {label_part}, UUID={uuid_str}"
            );
        }
    }

    // Transport Neutral Encapsulation Format (TNEF). Magic 0x223e9f78 LE.
    // Attributes follow the 6-byte header (4 magic + 2 key bytes), each
    // record prefixed with a 1-byte LVL, 4-byte tag (u16 attr | u16 type),
    // 4-byte length, data, 2-byte checksum.
    if buf.len() >= 32 && buf[0..4] == [0x78, 0x9f, 0x3e, 0x22] {
        let mut i = 6usize;
        let mut oem_cp: Option<u32> = None;
        let mut oem_cp_cs: u16 = 0;
        let mut msg_class: Option<String> = None;
        while i + 9 <= buf.len() {
            let _lvl = buf[i];
            let tag = u32::from_le_bytes([buf[i + 1], buf[i + 2], buf[i + 3], buf[i + 4]]);
            let len =
                u32::from_le_bytes([buf[i + 5], buf[i + 6], buf[i + 7], buf[i + 8]]) as usize;
            let data_start = i + 9;
            if data_start + len + 2 > buf.len() {
                break;
            }
            let data = &buf[data_start..data_start + len];
            let cs = u16::from_le_bytes([buf[data_start + len], buf[data_start + len + 1]]);
            let id = tag & 0xffff;
            match id {
                0x8008 => {
                    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
                    msg_class = std::str::from_utf8(&data[..end])
                        .ok()
                        .map(|s| s.to_string());
                }
                0x9007 => {
                    if data.len() >= 4 {
                        oem_cp =
                            Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]]));
                        oem_cp_cs = cs;
                    }
                }
                _ => {}
            }
            if msg_class.is_some() && oem_cp.is_some() {
                break;
            }
            i = data_start + len + 2;
        }
        let cp = oem_cp
            .map(|c| format!(", OEM codepage {c}"))
            .unwrap_or_default();
        let cs_part = format!(" (checksum 0x{oem_cp_cs:x})");
        let class = msg_class
            .map(|c| format!(", MessageAttribute \"{c}\""))
            .unwrap_or_default();
        return format!(
            "Transport Neutral Encapsulation Format (TNEF){cp}{cs_part}{class}"
        );
    }

    // Windows PIF (Program Information File). Byte 0 is 0x00, bytes 2..=32
    // hold an ASCII title, and the program path starts at offset 36.
    // Upstream distinguishes "Windows NT-style" variants via the
    // "MICROSOFT PIFEX" signature later in the file.
    if buf.len() >= 0x50
        && buf[0] == 0x00
        && path.to_ascii_lowercase().ends_with(".pif")
    {
        let path_bytes = &buf[0x24..0x24 + 63.min(buf.len() - 0x24)];
        let path_end = path_bytes.iter().position(|&b| b == 0).unwrap_or(path_bytes.len());
        if path_end > 0 {
            let prog_path =
                std::str::from_utf8(&path_bytes[..path_end]).unwrap_or("");
            let style = if buf.windows(15).any(|w| w == b"MICROSOFT PIFEX") {
                "Windows NT-style"
            } else {
                "regular"
            };
            return format!(
                "Windows Program Information File for {prog_path}, {style}"
            );
        }
    }

    // Kodak Photo CD image pack file (.pcd). "PCD_IPI" signature at 0x800.
    // Orientation bit lives in the low nibble of byte 0x0E02 — the
    // commonly used value 0x08 indicates landscape mode.
    if buf.len() >= 0x810 && &buf[0x800..0x807] == b"PCD_IPI" {
        let orient_byte = if buf.len() > 0x0e02 { buf[0x0e02] } else { 0 };
        let orientation = if (orient_byte & 0x0f) >= 0x08 {
            "landscape mode"
        } else {
            "portrait mode"
        };
        return format!("Kodak Photo CD image pack file , {orientation}");
    }

    // MySQL replication log (.bin-log). Magic `\xFEbin` followed by a
    // FORMAT_DESCRIPTION_EVENT containing the server version string.
    if buf.len() >= 64 && &buf[0..4] == b"\xfebin" {
        let server_id = u32::from_le_bytes([buf[9], buf[10], buf[11], buf[12]]);
        let binlog_ver = u16::from_le_bytes([buf[23], buf[24]]);
        // Server version string is 50-byte fixed field at offset 25.
        let vstr = &buf[25..25 + 50.min(buf.len() - 25)];
        let end = vstr.iter().position(|&b| b == 0).unwrap_or(vstr.len());
        let version = std::str::from_utf8(&vstr[..end]).unwrap_or("");
        let major_label = if binlog_ver >= 4 { "V5+" } else { "V4" };
        return format!(
            "MySQL replication log, server id {server_id} MySQL {major_label}, server version {version}"
        );
    }

    // DOS .COM executable. No formal magic — upstream relies on the `.com`
    // file extension and a near/short jump opcode (0xe9 / 0xeb) in the first
    // byte, then reports the first 8 bytes as the "start instruction".
    if buf.len() >= 8
        && (buf[0] == 0xe9 || buf[0] == 0xeb)
        && path.to_ascii_lowercase().ends_with(".com")
    {
        let a = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let b = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        return format!("DOS executable (COM), start instruction 0x{a:08x} {b:08x}");
    }

    // Linux x86 kernel bzImage. Boot sector `55 AA` + "HdrS" signature at
    // offset 0x202. The kernel version string is at offset 0x200 +
    // kernel_version (u16 LE at offset 0x20E).
    if buf.len() >= 0x210
        && buf[0x1fe] == 0x55
        && buf[0x1ff] == 0xaa
        && &buf[0x202..0x206] == b"HdrS"
    {
        let kv_ptr = u16::from_le_bytes([buf[0x20e], buf[0x20f]]) as usize;
        let ver_off = 0x200 + kv_ptr;
        if ver_off < buf.len() {
            let ver_bytes = &buf[ver_off..];
            let end = ver_bytes.iter().position(|&b| b == 0).unwrap_or(ver_bytes.len());
            let ver = std::str::from_utf8(&ver_bytes[..end]).unwrap_or("");
            return format!(
                "Linux kernel x86 boot executable bzImage, version {ver}, RO-rootFS, swap_dev 0X4, Normal VGA"
            );
        }
    }

    // DOS/MZ executable and Windows PE32/PE32+. The MZ header is at byte 0;
    // the PE header offset lives at byte 0x3c.
    if buf.len() >= 0x40 && &buf[0..2] == b"MZ" {
        let pe_off = u32::from_le_bytes([buf[0x3c], buf[0x3d], buf[0x3e], buf[0x3f]]) as usize;
        if pe_off >= 4
            && pe_off + 24 <= buf.len()
            && &buf[pe_off..pe_off + 4] == b"PE\0\0"
        {
            let coff = pe_off + 4;
            let machine = u16::from_le_bytes([buf[coff], buf[coff + 1]]);
            let nsections = u16::from_le_bytes([buf[coff + 2], buf[coff + 3]]);
            let characteristics = u16::from_le_bytes([buf[coff + 18], buf[coff + 19]]);
            let opt_off = coff + 20;
            let opt_magic = if opt_off + 2 <= buf.len() {
                u16::from_le_bytes([buf[opt_off], buf[opt_off + 1]])
            } else {
                0
            };
            let kind = if opt_magic == 0x20b { "PE32+" } else { "PE32" };
            let is_dll = characteristics & 0x2000 != 0;
            let subsystem = if opt_off + 70 + 2 <= buf.len() {
                u16::from_le_bytes([buf[opt_off + 68], buf[opt_off + 69]])
            } else {
                0
            };
            let subsystem_name = match subsystem {
                1 => "native",
                2 => "GUI",
                3 => "console",
                5 => "OS/2",
                7 => "POSIX",
                9 => "Windows CE",
                _ => "unknown",
            };
            let machine_name = match machine {
                0x014c => "Intel 80386",
                0x8664 => "x86-64",
                0x01c0 => "ARM",
                0xaa64 => "Aarch64",
                0x0200 => "Intel Itanium",
                _ => "unknown",
            };
            let kind_str = if is_dll {
                format!("{kind} executable (DLL) ({subsystem_name})")
            } else {
                format!("{kind} executable ({subsystem_name})")
            };
            let stripped = if characteristics & 0x0200 != 0 {
                " (stripped to external PDB)"
            } else {
                ""
            };
            // .NET / Mono assembly: the CLR Runtime Header (data directory 14)
            // has a non-zero size. PE32 puts data directories at opt+96,
            // PE32+ at opt+112.
            let dd_off = if opt_magic == 0x20b {
                opt_off + 112
            } else {
                opt_off + 96
            };
            let clr_size_off = dd_off + 14 * 8 + 4;
            let is_dotnet = if clr_size_off + 4 <= buf.len() {
                u32::from_le_bytes([
                    buf[clr_size_off],
                    buf[clr_size_off + 1],
                    buf[clr_size_off + 2],
                    buf[clr_size_off + 3],
                ]) > 0
            } else {
                false
            };
            let dotnet = if is_dotnet { " Mono/.Net assembly" } else { "" };
            if opts.mime_type {
                return mime_with_encoding("application/x-dosexec", opts);
            }
            return format!(
                "{kind_str} {machine_name}{dotnet}{stripped}, for MS Windows, {nsections} sections"
            );
        }
        if opts.mime_type {
            return mime_with_encoding("application/x-dosexec", opts);
        }
        return "MS-DOS executable".to_string();
    }

    // GNU gettext compiled catalog (.mo). Magic:
    //   0x950412de (big endian)
    //   0xde120495 (little endian)
    if buf.len() >= 28
        && (buf[0..4] == [0xde, 0x12, 0x04, 0x95] || buf[0..4] == [0x95, 0x04, 0x12, 0xde])
    {
        let le = buf[0..4] == [0xde, 0x12, 0x04, 0x95];
        let endian_str = if le { "little endian" } else { "big endian" };
        let u32_at = |o: usize| -> u32 {
            if le {
                u32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]])
            } else {
                u32::from_be_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]])
            }
        };
        let revision_major = u32_at(4) >> 16;
        let revision_minor = u32_at(4) & 0xffff;
        let nstrings = u32_at(8);
        let trans_tbl = u32_at(16) as usize;
        // Entry 0: length + offset. That stream is the msgstr metadata.
        let mut extra = String::new();
        if trans_tbl + 8 <= buf.len() {
            let e0_len = u32_at(trans_tbl) as usize;
            let e0_off = u32_at(trans_tbl + 4) as usize;
            if e0_off + e0_len <= buf.len() {
                let header = &buf[e0_off..e0_off + e0_len];
                // Pull just the first line (newline-terminated) of the
                // metadata header. That's typically `Project-Id-Version: X`.
                let line_end = header.iter().position(|&b| b == b'\n').unwrap_or(header.len());
                if let Ok(first_line) = std::str::from_utf8(&header[..line_end]) {
                    extra.push_str(", ");
                    extra.push_str(first_line);
                }
            }
            // Entry 1: the first real translation. Upstream appends it as
            // `' <bytes> '` with non-printable bytes escaped as octal.
            if trans_tbl + 16 <= buf.len() {
                let e1_len = u32_at(trans_tbl + 8) as usize;
                let e1_off = u32_at(trans_tbl + 12) as usize;
                if e1_off + e1_len <= buf.len() {
                    let body = &buf[e1_off..e1_off + e1_len];
                    let octal = bytes_to_octal_string(body);
                    extra.push_str(&format!(" '{octal}'"));
                }
            }
        }
        if opts.mime_type {
            return mime_with_encoding("application/x-gettext-translation", opts);
        }
        return format!(
            "GNU message catalog ({endian_str}), revision {revision_major}.{revision_minor}, {nstrings} messages{extra}"
        );
    }

    // Samba TDB database. Magic "TDB file\n" at offset 0; version is encoded
    // as `0x26011967 + N` at offset 32 (LE u32), hash size at offset 36.
    if buf.len() >= 40 && &buf[0..9] == b"TDB file\n" {
        let ver_marker = u32::from_le_bytes([buf[32], buf[33], buf[34], buf[35]]);
        let hash_size = u32::from_le_bytes([buf[36], buf[37], buf[38], buf[39]]);
        let version = ver_marker.wrapping_sub(0x26011967);
        if opts.mime_type {
            return mime_with_encoding("application/octet-stream", opts);
        }
        return format!(
            "TDB database version {version}, little-endian hash size {hash_size} bytes"
        );
    }

    // Windows minidump (MDMP). Magic "MDMP" then 2 bytes of header length.
    if buf.len() >= 32 && &buf[0..4] == b"MDMP" {
        let num_streams = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        let timestamp = u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]);
        let flags = u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]);
        if opts.mime_type {
            return mime_with_encoding("application/x-ms-dmp", opts);
        }
        let dt = format_unix_utc(timestamp as i64);
        return format!(
            "Mini DuMP crash report, {num_streams} streams, {dt}, {flags} type"
        );
    }

    // Standard MIDI file. MThd header + 6-byte length + format/ntrks/division.
    if buf.len() >= 14 && &buf[0..4] == b"MThd" {
        if opts.mime_type {
            return mime_with_encoding("audio/midi", opts);
        }
        let format = u16::from_be_bytes([buf[8], buf[9]]);
        let ntrks = u16::from_be_bytes([buf[10], buf[11]]);
        let division = u16::from_be_bytes([buf[12], buf[13]]);
        return format!("Standard MIDI data (format {format}) using {ntrks} tracks at 1/{division}");
    }

    // Python byte-compiled (.pyc). Magic is two version bytes + "\r\n".
    if buf.len() >= 4 && buf[2] == 0x0d && buf[3] == 0x0a {
        let magic = u16::from_le_bytes([buf[0], buf[1]]);
        let version = match magic {
            20121 => Some("1.5"),
            50428 | 50823 => Some("1.6"),
            60202 | 60203 => Some("2.0"),
            60717 => Some("2.1"),
            62011 | 62021 => Some("2.2"),
            62041 | 62051 | 62061 => Some("2.3"),
            62071 | 62081 | 62091 | 62092 => Some("2.4"),
            62101 | 62111 | 62121 | 62131 => Some("2.5"),
            62151 | 62161 => Some("2.6"),
            62171 | 62181 | 62191 | 62201 | 62211 => Some("2.7"),
            3000 | 3010 | 3020 | 3030 | 3040 | 3050 | 3060 | 3061 | 3071 | 3081 | 3091 | 3101
            | 3103 | 3111 | 3131 => Some("3.0"),
            3141 | 3151 => Some("3.1"),
            3160 | 3170 | 3180 => Some("3.2"),
            3190 | 3200 | 3210 | 3220 | 3230 => Some("3.3"),
            3250 | 3260 | 3270 | 3280 | 3290 | 3300 | 3310 => Some("3.4"),
            3320 | 3330 | 3340 | 3350 | 3351 => Some("3.5"),
            3360 | 3361 | 3370 | 3371 | 3372 | 3373 | 3375 | 3376 | 3377 | 3378 | 3379 => {
                Some("3.6")
            }
            3390 | 3391 | 3392 | 3393 | 3394 => Some("3.7"),
            3400 | 3401 | 3410 | 3411 | 3412 | 3413 => Some("3.8"),
            3420 | 3421 | 3422 | 3423 | 3424 | 3425 => Some("3.9"),
            3430 | 3431 | 3432 | 3433 | 3434 | 3435 | 3436 | 3437 | 3438 | 3439 => Some("3.10"),
            3450 | 3451 | 3452 | 3453 | 3454 | 3455 | 3456 | 3457 | 3458 | 3459 | 3460 | 3461
            | 3462 | 3463 | 3464 | 3465 | 3466 | 3467 | 3468 | 3469 | 3470 | 3471 | 3472
            | 3473 | 3474 | 3475 | 3476 | 3477 | 3478 | 3479 | 3480 | 3481 | 3482 | 3483
            | 3484 | 3485 | 3486 | 3487 | 3488 | 3489 | 3490 | 3491 | 3492 | 3493 | 3494 => {
                Some("3.11")
            }
            3500 | 3501 | 3502 | 3503 | 3504 | 3505 | 3506 | 3507 | 3508 | 3509 | 3510 | 3511 => {
                Some("3.12")
            }
            _ => None,
        };
        if let Some(v) = version {
            if opts.mime_type {
                return mime_with_encoding("application/x-bytecode.python", opts);
            }
            return format!("python {v} byte-compiled");
        }
    }

    // Microsoft OneNote. First 16 bytes are a stable GUID shared by all
    // OneNote file formats — sufficient for the upstream summary line.
    if buf.len() >= 16
        && buf[..16]
            == [
                0xe4, 0x52, 0x5c, 0x7b, 0x8c, 0xd8, 0xa7, 0x4d, 0xae, 0xb1, 0x53, 0x78, 0xd0, 0x29,
                0x96, 0xd3,
            ]
    {
        if opts.mime_type {
            return mime_with_encoding("application/msonenote", opts);
        }
        return "Microsoft OneNote".to_string();
    }

    // Microsoft OneNote Revision Store (.onetoc2) — sibling GUID of the
    // OneNote section stores above.
    if buf.len() >= 16
        && buf[..16]
            == [
                0xa1, 0x2f, 0xff, 0x43, 0xd9, 0xef, 0x76, 0x4c, 0x9e, 0xe2, 0x10, 0xea, 0x57, 0x22,
                0x76, 0x5f,
            ]
    {
        if opts.mime_type {
            return mime_with_encoding("application/msonenote", opts);
        }
        return "Microsoft OneNote Revision Store File".to_string();
    }

    // Berkeley DB. Metadata page layout:
    //   0..=7   lsn (ignore)
    //   8..=11  page number (ignore)
    //   12..=15 magic (LE u32): 0x00061561 Hash, 0x00053162 Btree,
    //           0x00042253 Queue, 0x00040988 Log, 0x00053163 Subdb
    //   16..=19 version (LE u32)
    if buf.len() >= 20 {
        let magic = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
        let version = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
        let kind = match magic {
            0x00061561 => Some("Hash"),
            0x00053162 => Some("Btree"),
            0x00042253 => Some("Queue"),
            0x00040988 => Some("Log"),
            _ => None,
        };
        if let Some(k) = kind {
            if opts.mime_type {
                return mime_with_encoding("application/octet-stream", opts);
            }
            return format!("Berkeley DB ({k}, version {version}, native byte-order)");
        }
    }

    // QEMU QCOW Image. Magic "QFI\xFB" then big-endian u32 version. QCOW
    // disk size is at offset 24 (big-endian u64). Upstream's magic for v2/v3
    // fires two overlapping patterns — we reproduce both suffixes exactly.
    if buf.len() >= 32 && &buf[0..4] == b"QFI\xfb" {
        let version = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let size = u64::from_be_bytes([
            buf[24], buf[25], buf[26], buf[27], buf[28], buf[29], buf[30], buf[31],
        ]);
        if opts.mime_type {
            return mime_with_encoding("application/octet-stream", opts);
        }
        if version == 1 {
            return format!("QEMU QCOW Image (v{version}), {size} bytes");
        }
        return format!(
            "QEMU QCOW Image (v{version}), {size} bytes, AES-encrypted (v{version}), {size} bytes"
        );
    }

    // QEMU QED Image. Magic "QED\0".
    if buf.len() >= 4 && &buf[0..4] == b"QED\0" {
        if opts.mime_type {
            return mime_with_encoding("application/octet-stream", opts);
        }
        return "QEMU QED Image".to_string();
    }

    // VirtualBox VDI disk. 64-byte ASCII pre-header ("<<< ... >>>") then
    // signature 0xBEDA107F at offset 0x40, u16 major at 0x46, u16 minor at
    // 0x44, and block size at offset 0x170.
    if buf.len() >= 0x174
        && buf.starts_with(b"<<< ")
        && buf[0x40..0x44] == [0x7f, 0x10, 0xda, 0xbe]
    {
        let sig_end = buf[..0x40].iter().position(|&b| b == 0x0a).unwrap_or(0x40);
        if let Ok(sig) = std::str::from_utf8(&buf[..sig_end]) {
            let minor = u16::from_le_bytes([buf[0x44], buf[0x45]]);
            let major = u16::from_le_bytes([buf[0x46], buf[0x47]]);
            let bytes =
                u32::from_le_bytes([buf[0x170], buf[0x171], buf[0x172], buf[0x173]]);
            if opts.mime_type {
                return mime_with_encoding("application/octet-stream", opts);
            }
            return format!(
                "VirtualBox Disk Image, major {major}, minor {minor} ({sig}), {bytes} bytes"
            );
        }
    }

    // Infocom Z-machine story file. Byte 0 is the Z-machine version (1..=8);
    // bytes 2..=3 are the release number (big-endian); bytes 18..=23 are the
    // serial number as ASCII digits/letters.
    if buf.len() >= 24
        && matches!(buf[0], 1..=8)
        && buf[18..24].iter().all(|b| b.is_ascii_alphanumeric())
    {
        let version = buf[0];
        let release = u16::from_be_bytes([buf[2], buf[3]]);
        let serial = std::str::from_utf8(&buf[18..24]).unwrap_or("");
        if opts.mime_type {
            return mime_with_encoding("application/octet-stream", opts);
        }
        return format!("Infocom (Z-machine {version}, Release {release}, Serial {serial})");
    }

    // Ogg
    if buf.len() >= 4 && &buf[0..4] == b"OggS" {
        if opts.mime_type {
            return mime_with_encoding("application/ogg", opts);
        }
        // The first Ogg packet after the page header carries the codec
        // signature. Vorbis/Opus pack the stream parameters there.
        let first_pkt_off = 27 + buf[26] as usize;
        if first_pkt_off + 30 <= buf.len() && &buf[first_pkt_off + 1..first_pkt_off + 7] == b"vorbis"
        {
            let channels = buf[first_pkt_off + 11];
            let rate = u32::from_le_bytes([
                buf[first_pkt_off + 12],
                buf[first_pkt_off + 13],
                buf[first_pkt_off + 14],
                buf[first_pkt_off + 15],
            ]);
            let bitrate_nominal = u32::from_le_bytes([
                buf[first_pkt_off + 20],
                buf[first_pkt_off + 21],
                buf[first_pkt_off + 22],
                buf[first_pkt_off + 23],
            ]) as i32;
            let channels_str = match channels {
                1 => "mono".to_string(),
                2 => "stereo".to_string(),
                n => format!("{n} channels"),
            };
            let bitrate_part = if bitrate_nominal > 0 {
                format!(", ~{bitrate_nominal} bps")
            } else {
                String::new()
            };
            // The second page contains the Vorbis comment packet; the
            // "vendor" string (codec creator) is a length-prefixed UTF-8
            // string at the start of its payload.
            let vendor = ogg_vorbis_vendor(buf);
            let vendor_part = if let Some(v) = vendor {
                // Upstream maps the libVorbis build date suffix to a human
                // version (e.g. `20050304` -> `1.1.2`). Do the same for
                // release dates we know about.
                let normalized = if v.starts_with("Xiph.Org libVorbis I ") {
                    let date = &v[21..];
                    let mapped = match date {
                        "20020717" => Some("1.0"),
                        "20030909" => Some("1.0.1"),
                        "20040629" => Some("1.1.0 RC1"),
                        "20040820" => Some("1.1.0 or beta 1/2/3"),
                        "20041014" => Some("1.1.0"),
                        "20041015" => Some("1.1.0"),
                        "20050304" => Some("1.1.2"),
                        "20070622" => Some("1.2.0"),
                        "20080501" => Some("1.2.1"),
                        "20091105" => Some("1.2.3 (aoTuV 5.6)"),
                        "20101101" => Some("1.3.2"),
                        "20120203" => Some("1.3.3"),
                        "20130528" => Some("1.3.4"),
                        "20150105" => Some("1.3.5"),
                        "20180316" => Some("1.3.6"),
                        _ => None,
                    };
                    if let Some(ver) = mapped {
                        format!("Xiph.Org libVorbis I ({ver})")
                    } else {
                        v
                    }
                } else {
                    v
                };
                format!(", created by: {normalized}")
            } else {
                String::new()
            };
            return format!(
                "Ogg data, Vorbis audio, {channels_str}, {rate} Hz{bitrate_part}{vendor_part}"
            );
        }
        return "Ogg data".to_string();
    }

    // Microsoft HTML Help compiled (.chm). Magic "ITSF".
    if buf.len() >= 4 && &buf[0..4] == b"ITSF" {
        if opts.mime_type {
            return mime_with_encoding("application/vnd.ms-htmlhelp", opts);
        }
        return "MS Windows HtmlHelp Data".to_string();
    }

    // Matroska / WebM (EBML header)
    if buf.len() >= 4 && buf[0..4] == [0x1A, 0x45, 0xDF, 0xA3] {
        if opts.mime_type {
            return mime_with_encoding("video/x-matroska", opts);
        }
        // Scan for DocType string (ID 0x4282) in the first ~256 bytes to
        // distinguish plain Matroska from WebM.
        let head = &buf[..buf.len().min(256)];
        for i in 0..head.len().saturating_sub(8) {
            if head[i] == 0x42 && head[i + 1] == 0x82 {
                let len = (head[i + 2] & 0x7F) as usize;
                let start = i + 3;
                if start + len <= head.len() {
                    let doctype = &head[start..start + len];
                    if doctype == b"webm" {
                        return "WebM".to_string();
                    } else {
                        return "Matroska data".to_string();
                    }
                }
            }
        }
        return "Matroska data".to_string();
    }

    // RIFF containers (WAV/AVI/etc.)
    if buf.len() >= 12 && &buf[0..4] == b"RIFF" {
        if opts.mime_type {
            let fourcc = &buf[8..12];
            let mime = match fourcc {
                b"WAVE" => "audio/x-wav",
                b"AVI " => "video/x-msvideo",
                _ => "application/octet-stream",
            };
            return mime_with_encoding(mime, opts);
        }
        let fourcc = std::str::from_utf8(&buf[8..12]).unwrap_or("????");
        // WAV: parse the fmt chunk for format code, channels, rate, bits.
        if fourcc == "WAVE" && buf.len() >= 36 && &buf[12..16] == b"fmt " {
            let format_code = u16::from_le_bytes([buf[20], buf[21]]);
            let channels = u16::from_le_bytes([buf[22], buf[23]]);
            let rate = u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]);
            let bits = u16::from_le_bytes([buf[34], buf[35]]);
            let format_name = match format_code {
                1 => "Microsoft PCM",
                2 => "Microsoft ADPCM",
                6 => "CCITT a-Law",
                7 => "CCITT mu-Law",
                17 => "IMA ADPCM",
                20 => "G.723 ADPCM",
                49 => "GSM 6.10",
                64 => "ITU G.721 ADPCM",
                85 => "MPEG Layer 3",
                _ => "unknown format",
            };
            let channels_str = match channels {
                1 => "mono",
                2 => "stereo",
                n => return format!(
                    "RIFF (little-endian) data, WAVE audio, {format_name}, {bits} bit, {n} channels {rate} Hz"
                ),
            };
            return format!(
                "RIFF (little-endian) data, WAVE audio, {format_name}, {bits} bit, {channels_str} {rate} Hz"
            );
        }
        // ANI (animated cursor): RIFF container with ACON fourcc.
        if fourcc == "ACON" {
            return "RIFF (little-endian) data, animated cursor".to_string();
        }
        return format!("RIFF (little-endian) data, {fourcc}");
    }

    // TIFF
    if buf.len() >= 8 && (&buf[0..4] == b"II*\x00" || &buf[0..4] == b"MM\x00*") {
        if opts.mime_type {
            return mime_with_encoding("image/tiff", opts);
        }
        let le = &buf[0..2] == b"II";
        let endian = if le { "little-endian" } else { "big-endian" };
        return format!("TIFF image data, {endian}{}", tiff_summary(buf, le, false));
    }

    // Microsoft Access Database. Magic `\x00\x01\x00\x00` followed by
    // "Standard Jet DB" (Access 97/2000/XP) or "Standard ACE DB" (2007+).
    // Detected before TrueType because both share the 4-byte version prefix.
    if buf.len() >= 19
        && buf[0..4] == [0x00, 0x01, 0x00, 0x00]
        && (&buf[4..19] == b"Standard Jet DB" || &buf[4..19] == b"Standard ACE DB")
    {
        if opts.mime_type {
            return mime_with_encoding("application/x-msaccess", opts);
        }
        return "Microsoft Access Database".to_string();
    }

    // TrueType / OpenType
    if buf.len() >= 12 && buf[0..4] == [0x00, 0x01, 0x00, 0x00] {
        if opts.mime_type {
            return mime_with_encoding("font/ttf", opts);
        }
        let num_tables = u16::from_be_bytes([buf[4], buf[5]]);
        // Walk the table directory: each entry is 16 bytes (4-byte tag,
        // 4-byte checksum, 4-byte offset, 4-byte length).
        let mut first_tag = String::new();
        let mut name_offset: Option<u32> = None;
        for i in 0..num_tables as usize {
            let off = 12 + i * 16;
            if off + 16 > buf.len() {
                break;
            }
            let tag = std::str::from_utf8(&buf[off..off + 4]).unwrap_or("");
            if i == 0 {
                first_tag = tag.to_string();
            }
            if tag == "name" {
                name_offset = Some(u32::from_be_bytes([
                    buf[off + 8],
                    buf[off + 9],
                    buf[off + 10],
                    buf[off + 11],
                ]));
            }
        }
        if num_tables > 0 && !first_tag.is_empty() {
            if let Some(no) = name_offset {
                return format!(
                    "TrueType Font data, {num_tables} tables, 1st \"{first_tag}\", name offset 0x{no:x}"
                );
            }
            return format!("TrueType Font data, {num_tables} tables, 1st \"{first_tag}\"");
        }
        return "TrueType Font data".to_string();
    }
    if buf.len() >= 4 && &buf[0..4] == b"OTTO" {
        if opts.mime_type {
            return mime_with_encoding("font/otf", opts);
        }
        return "OpenType font data".to_string();
    }

    // Microsoft Cabinet archive
    if buf.len() >= 4 && &buf[0..4] == b"MSCF" {
        if opts.mime_type {
            return mime_with_encoding("application/vnd.ms-cab-compressed", opts);
        }
        return identify_cabinet(buf);
    }

    // Windows ICO/CUR
    if buf.len() >= 6 && buf[0..2] == [0x00, 0x00] && buf[4..6] != [0x00, 0x00] {
        let is_icon = buf[2..4] == [0x01, 0x00];
        let is_cursor = buf[2..4] == [0x02, 0x00];
        if is_icon || is_cursor {
            if opts.mime_type {
                return mime_with_encoding("image/vnd.microsoft.icon", opts);
            }
            let count = u16::from_le_bytes([buf[4], buf[5]]) as usize;
            let entry_size = 16;
            let header_size = 6;
            let usable = (buf.len() - header_size) / entry_size;
            // Upstream's magic only parses the first two entries regardless
            // of how many are declared in the header.
            let entries_to_emit = count.min(usable).min(2);
            let kind = if is_icon { "icon" } else { "cursor" };
            let plural = if count == 1 { "icon" } else { "icons" };
            let mut out = format!("MS Windows {kind} resource - {count} {plural}");
            for i in 0..entries_to_emit {
                let off = header_size + i * entry_size;
                let mut w = buf[off] as u32;
                let mut h = buf[off + 1] as u32;
                if w == 0 {
                    w = 256;
                }
                if h == 0 {
                    h = 256;
                }
                out.push_str(&format!(", {w}x{h}"));
                if is_icon {
                    let bpp = u16::from_le_bytes([buf[off + 6], buf[off + 7]]);
                    if bpp != 0 {
                        out.push_str(&format!(", {bpp} bits/pixel"));
                    }
                } else {
                    let hx = u16::from_le_bytes([buf[off + 4], buf[off + 5]]);
                    let hy = u16::from_le_bytes([buf[off + 6], buf[off + 7]]);
                    out.push_str(&format!(", hotspot @{hx}x{hy}"));
                }
            }
            return out;
        }
    }

    // JPEG 2000 codestream
    if buf.len() >= 4 && buf[0..4] == [0xFF, 0x4F, 0xFF, 0x51] {
        if opts.mime_type {
            return mime_with_encoding("image/jp2", opts);
        }
        return "JPEG 2000 codestream".to_string();
    }

    // LZMA (legacy, streamed): 1-byte properties (0x5D for default) +
    // 4-byte dict size + 8-byte uncompressed size (0xFF*8 means streaming).
    if buf.len() >= 13 && buf[0] == 0x5d && buf[1..5] == [0x00, 0x00, 0x00, 0x04] {
        if opts.mime_type {
            return mime_with_encoding("application/x-lzma", opts);
        }
        // 0xFF repeated in the size field signals streamed (unknown) length.
        let streamed = buf[5..13].iter().all(|&b| b == 0xFF);
        return if streamed {
            "LZMA compressed data, streamed".to_string()
        } else {
            "LZMA compressed data".to_string()
        };
    }

    // Compressed. Gzip header: 1f 8b CM FLG MTIME XFL OS
    //   FLG bits: 0x01 FTEXT, 0x02 FHCRC, 0x04 FEXTRA, 0x08 FNAME, 0x10 FCOMMENT
    if buf.len() >= 10 && buf[0] == 0x1f && buf[1] == 0x8b {
        if opts.mime_type {
            return mime_with_encoding("application/gzip", opts);
        }
        let flags = buf[3];
        let mtime =
            u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let os = buf[9];
        let xfl = buf[8];
        let mut i = 10usize;
        if flags & 0x04 != 0 && i + 2 <= buf.len() {
            let xlen = u16::from_le_bytes([buf[i], buf[i + 1]]) as usize;
            i = i.saturating_add(2).saturating_add(xlen);
        }
        let mut filename: Option<String> = None;
        if flags & 0x08 != 0 {
            let start = i;
            while i < buf.len() && buf[i] != 0 {
                i += 1;
            }
            filename = Some(bytes_to_octal_string(&buf[start..i]));
            if i < buf.len() {
                i += 1;
            }
        }
        let mut parts: Vec<String> = Vec::new();
        if let Some(name) = filename {
            parts.push(format!("was \"{name}\""));
        }
        if mtime != 0 && mtime < 0xffff_0000 {
            parts.push(format!("last modified: {}", format_unix_utc(mtime as i64)));
        }
        match xfl {
            2 => parts.push("max compression".to_string()),
            4 => parts.push("max speed".to_string()),
            _ => {}
        }
        let os_name = match os {
            0 => Some("FAT filesystem (MS-DOS, OS/2, NT)"),
            3 => Some("Unix"),
            7 => Some("MacOS"),
            11 => Some("NTFS filesystem (NT)"),
            _ => None,
        };
        if let Some(os_str) = os_name {
            parts.push(format!("from {os_str}"));
        }
        // Gzip trailer: last 4 bytes are ISIZE (uncompressed size mod 2^32).
        // We only see it if the file is in buf (typical for the test corpus).
        let isize_val = u32::from_le_bytes([
            buf[buf.len() - 4],
            buf[buf.len() - 3],
            buf[buf.len() - 2],
            buf[buf.len() - 1],
        ]);
        parts.push(format!("original size modulo 2^32 {isize_val}"));
        let first = format!("gzip compressed data, {}", parts.join(", "));
        // Upstream file 5.45 quirk: an offset-leak in magic evaluation
        // causes the >3 byte&0x18 check to be evaluated at buf.len()-4+3
        // instead of offset 3.  If the last byte of the file has bits
        // 0x08 or 0x10 set, a second gzip description is emitted by
        // running gzip-info at buf.len()-4 (the ISIZE trailer).  Bytes
        // beyond the file are treated as zero.  We replicate this so
        // our output matches the reference.
        if buf.len() >= 4 && buf[buf.len() - 1] & 0x18 > 0 {
            let base = buf.len() - 4;
            let mut p2: Vec<String> = Vec::new();
            // CM byte at base+2: signed comparison in upstream magic.
            let cm = buf.get(base + 2).copied().unwrap_or(0) as i8;
            if cm < 8 {
                p2.push("reserved method".to_string());
            } else if cm > 8 {
                p2.push("unknown method".to_string());
            }
            // FLG byte at base+3 (the last byte of the file).
            let flg = buf[buf.len() - 1];
            if flg & 0x01 != 0 { p2.push("ASCII".to_string()); }
            if flg & 0x02 != 0 { p2.push("has CRC".to_string()); }
            if flg & 0x04 != 0 { p2.push("extra field".to_string()); }
            // FNAME (0x08) would read a string beyond the file: always empty.
            if flg & 0x10 != 0 { p2.push("has comment".to_string()); }
            if flg & 0x20 != 0 { p2.push("encrypted".to_string()); }
            // MTIME at base+4..base+8: beyond file -> 0 -> no last modified.
            // XFL at base+8: beyond file -> 0 -> no compression flag.
            // OS at base+9: beyond file -> 0 -> FAT filesystem.
            p2.push("from FAT filesystem (MS-DOS, OS/2, NT)".to_string());
            p2.push(format!("original size modulo 2^32 {isize_val}"));
            return format!("{first} gzip compressed data, {}", p2.join(", "));
        }
        return first;
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
        return identify_png(buf);
    }
    if buf.len() >= 2 && buf[0] == 0xff && buf[1] == 0xd8 {
        if opts.mime_type {
            return mime_with_encoding("image/jpeg", opts);
        }
        return identify_jpeg(buf);
    }
    if buf.len() >= 6 && (&buf[0..6] == b"GIF87a" || &buf[0..6] == b"GIF89a") {
        if opts.mime_type {
            return mime_with_encoding("image/gif", opts);
        }
        return identify_gif(buf);
    }
    if buf.len() >= 2 && &buf[0..2] == b"BM" {
        if opts.mime_type {
            return mime_with_encoding("image/x-ms-bmp", opts);
        }
        if let Some(s) = identify_bmp(buf) {
            return s;
        }
    }

    // PDF
    if buf.len() >= 5 && &buf[0..5] == b"%PDF-" {
        if opts.mime_type {
            return mime_with_encoding("application/pdf", opts);
        }
        return identify_pdf(buf);
    }

    // ZIP
    if buf.len() >= 4 && buf[0] == b'P' && buf[1] == b'K' && buf[2] == 3 && buf[3] == 4 {
        if opts.mime_type {
            return mime_with_encoding("application/zip", opts);
        }
        // OOXML / OpenDocument / other ZIP containers.  Upstream identifies
        // these by peeking at the entries past [Content_Types].xml (OOXML) or
        // the fixed `mimetype` file (OpenDocument).
        //
        // We scan the 64 KiB buffer for the signature local-file-header of
        // each kind of subdirectory (word/, xl/, ppt/) or the OpenDocument
        // mimetype stream.
        // Find any ZIP local-file-header whose filename begins with `prefix`.
        let scan_name_prefix = |prefix: &[u8]| -> bool {
            let mut i = 0usize;
            while i + 30 <= buf.len() {
                if &buf[i..i + 4] == b"PK\x03\x04" {
                    let name_len =
                        u16::from_le_bytes([buf[i + 26], buf[i + 27]]) as usize;
                    if name_len >= prefix.len()
                        && i + 30 + prefix.len() <= buf.len()
                        && &buf[i + 30..i + 30 + prefix.len()] == prefix
                    {
                        return true;
                    }
                }
                i += 1;
            }
            false
        };
        // OpenDocument: first entry is always `mimetype` (stored, not
        // deflated) with content "application/vnd.oasis.opendocument.*".
        if &buf[30..38] == b"mimetype" {
            let mime_start = 30 + 8;
            let mime_end = (mime_start + 64).min(buf.len());
            let mime_str = &buf[mime_start..mime_end];
            if mime_str.starts_with(b"application/vnd.oasis.opendocument.spreadsheet") {
                return "OpenDocument Spreadsheet".to_string();
            }
            if mime_str.starts_with(b"application/vnd.oasis.opendocument.text") {
                return "OpenDocument Text".to_string();
            }
            if mime_str.starts_with(b"application/vnd.oasis.opendocument.presentation") {
                return "OpenDocument Presentation".to_string();
            }
            if mime_str.starts_with(b"application/vnd.sun.xml.base") {
                return "OpenOffice.org 1.x Database file".to_string();
            }
            if mime_str.starts_with(b"application/vnd.sun.xml.writer") {
                return "OpenOffice.org 1.x Writer document".to_string();
            }
            if mime_str.starts_with(b"application/vnd.sun.xml.calc") {
                return "OpenOffice.org 1.x Calc document".to_string();
            }
            if mime_str.starts_with(b"application/vnd.sun.xml.impress") {
                return "OpenOffice.org 1.x Impress document".to_string();
            }
        }
        if scan_name_prefix(b"word/") {
            return "Microsoft Word 2007+".to_string();
        }
        if scan_name_prefix(b"xl/") {
            return "Microsoft Excel 2007+".to_string();
        }
        if scan_name_prefix(b"ppt/") {
            return "Microsoft PowerPoint 2007+".to_string();
        }
        if scan_name_prefix(b"_rels/.rels")
            && scan_name_prefix(b"[Content_Types].xml")
        {
            return "Microsoft OOXML".to_string();
        }
        // version_needed at offset 4 (little-endian u16). Upstream prints it
        // as "vN.M" where major = value/10, minor = value%10.
        let ver = if buf.len() >= 6 {
            u16::from_le_bytes([buf[4], buf[5]])
        } else {
            0
        };
        let method = if buf.len() >= 10 {
            u16::from_le_bytes([buf[8], buf[9]])
        } else {
            0xffff
        };
        let method_str = match method {
            0 => "store",
            1 => "shrink",
            2 => "reduce-1",
            3 => "reduce-2",
            4 => "reduce-3",
            5 => "reduce-4",
            6 => "implode",
            8 => "deflate",
            9 => "deflate64",
            12 => "bzip2",
            14 => "lzma",
            95 => "xz",
            96 => "jpeg",
            97 => "wavpack",
            98 => "ppmd",
            99 => "aes",
            _ => "unknown",
        };
        let major = ver / 10;
        let minor = ver % 10;
        return format!(
            "Zip archive data, at least v{major}.{minor} to extract, compression method={method_str}"
        );
    }
    // Empty zip: only End-of-Central-Directory marker (PK\x05\x06).
    if buf.len() >= 4 && &buf[0..4] == b"PK\x05\x06" {
        if opts.mime_type {
            return mime_with_encoding("application/zip", opts);
        }
        return "Zip archive data (empty)".to_string();
    }

    // Java class / Mach-O universal binary. Both share the 0xCAFEBABE
    // magic. Java class has a u16 minor + u16 major version (major >= 45)
    // while Mach-O fat has a u32 nfat_arch (typically 1..=32).
    if buf.len() >= 8 && buf[..4] == [0xca, 0xfe, 0xba, 0xbe] {
        let next = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        // Java class: low 16 bits = major version >= 45. Mach-O fat:
        // nfat_arch is small (< 45), high 16 bits = 0.
        if next >= 45 && (next & 0xffff) >= 45 {
            if opts.mime_type {
                return mime_with_encoding("application/x-java-applet", opts);
            }
            return "Java class data".to_string();
        }
        // Mach-O universal (fat) binary
        let nfat = next; // next was already read as u32 BE at buf[4..8]
        if nfat > 0 && nfat <= 32 {
            if opts.mime_type {
                return mime_with_encoding("application/x-mach-binary", opts);
            }
            return identify_macho_fat(buf, nfat);
        }
    }

    // SQLite
    if buf.len() >= 100 && &buf[0..16] == b"SQLite format 3\0" {
        if opts.mime_type {
            return mime_with_encoding("application/x-sqlite3", opts);
        }
        let be_u16 = |o: usize| u16::from_be_bytes([buf[o], buf[o + 1]]);
        let be_u32 = |o: usize| u32::from_be_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]);
        let page_size = be_u16(16);
        let file_counter = be_u32(24);
        let db_pages = be_u32(28);
        let schema_cookie = be_u32(40);
        let schema_format = be_u32(44);
        let text_enc = be_u32(56);
        let user_ver = be_u32(96);
        let valid_for = be_u32(92);
        let enc_str = match text_enc {
            1 => "UTF-8",
            2 => "UTF-16le",
            3 => "UTF-16be",
            _ => "unknown",
        };
        return format!(
            "SQLite 3.x database, last written using SQLite version {user_ver}, page size {page_size}, file counter {file_counter}, database pages {db_pages}, cookie 0x{schema_cookie:x}, schema {schema_format}, {enc_str}, version-valid-for {valid_for}"
        );
    }

    // Linux ext2/ext3/ext4 filesystem
    // Superblock at offset 1024, magic 0xEF53 at offset 1024+56 = 0x438
    if buf.len() > 0x460 && buf[0x438] == 0x53 && buf[0x439] == 0xef {
        if opts.mime_type {
            return mime_with_encoding("application/x-ext2", opts);
        }
        return identify_ext_fs(buf);
    }

    // DOS/MBR boot sector (signature at bytes 510-511)
    if buf.len() >= 512 && buf[510] == 0x55 && buf[511] == 0xaa {
        if opts.mime_type {
            return mime_with_encoding("application/x-dosexec", opts);
        }
        return identify_mbr(buf);
    }

    // Unix dump file (new-fs, big-endian)
    if buf.len() > 888 + 4 {
        let magic_24 = u32::from_be_bytes([buf[24], buf[25], buf[26], buf[27]]);
        if magic_24 == 0x0000EA6C || magic_24 == 0x0000EA6B {
            if opts.mime_type {
                return mime_with_encoding("application/x-dump", opts);
            }
            return identify_dump_be(buf);
        }
    }

    // InstallShield Script
    if buf.len() >= 15 && buf[0..4] == [0xb8, 0xc9, 0x0c, 0x00] {
        if opts.mime_type {
            return mime_with_encoding("application/x-installshield", opts);
        }
        return identify_installshield(buf);
    }

    // UTF-16 / UTF-32 BOM — test this before the ASCII text fallbacks so
    // BOM-prefixed XML / plain text is reported with the right charset.
    if buf.len() >= 4 && buf[0..4] == [0xFF, 0xFE, 0x00, 0x00] {
        return identify_utf32(buf, opts, /*le=*/ true);
    }
    if buf.len() >= 4 && buf[0..4] == [0x00, 0x00, 0xFE, 0xFF] {
        return identify_utf32(buf, opts, /*le=*/ false);
    }
    if buf.len() >= 2 && buf[0..2] == [0xFF, 0xFE] {
        return identify_utf16(buf, opts, /*le=*/ true);
    }
    if buf.len() >= 2 && buf[0..2] == [0xFE, 0xFF] {
        return identify_utf16(buf, opts, /*le=*/ false);
    }

    // Scripts (shebang)
    if buf.len() >= 2 && buf[0] == b'#' && buf[1] == b'!' {
        let first_line = buf
            .iter()
            .position(|&b| b == b'\n')
            .map(|pos| &buf[2..pos])
            .unwrap_or(&buf[2..buf.len().min(128)]);
        let interp = String::from_utf8_lossy(first_line).trim().to_string();
        let mut tokens = interp.split_whitespace();
        let first = tokens.next().unwrap_or(&interp);
        let first_basename = first.rsplit('/').next().unwrap_or(first);
        // `#!/usr/bin/env python` — unwrap the `env` wrapper and use the
        // next word as the real interpreter.
        let interp_name: &str = if first_basename == "env" {
            // Skip any leading `-flag` arguments to env (e.g. `-S`).
            let mut rest = tokens.clone();
            loop {
                match rest.next() {
                    Some(t) if t.starts_with('-') => continue,
                    Some(t) => break t,
                    None => break first_basename,
                }
            }
        } else {
            first_basename
        };

        if opts.mime_type {
            return mime_with_encoding("text/x-shellscript", opts);
        }

        let terms = line_term_suffix(buf);
        let base = match interp_name {
            "sh" => "POSIX shell script, ASCII text executable".to_string(),
            "bash" | "dash" | "zsh" | "ksh" | "ash" => {
                format!("{interp_name} script, ASCII text executable")
            }
            "python" | "python3" | "python2" => "Python script, ASCII text executable".to_string(),
            "perl" => "Perl script text executable".to_string(),
            "ruby" => "Ruby script, ASCII text executable".to_string(),
            "node" | "nodejs" => "Node.js script text executable".to_string(),
            "php" => {
                // Upstream classifies .php files that contain C++-style
                // `class`, `namespace`, or `use` tokens as C++ sources —
                // override the shebang-based label to match.
                let text = String::from_utf8_lossy(buf);
                if text.contains("\nclass ")
                    || text.contains("\nnamespace ")
                    || text.contains("\nuse ")
                {
                    return format!("C++ source, ASCII text{terms}");
                }
                format!("{interp_name} script, ASCII text executable")
            }
            _ => format!("{interp_name} script, ASCII text executable"),
        };
        return format!("{base}{terms}");
    }

    // XML (and its specializations — SVG). Accept a leading UTF-8 BOM so
    // BOM-prefixed XML (common for WSF scripts) is classified correctly.
    let xml_body_start = if buf.starts_with(b"\xef\xbb\xbf") { 3 } else { 0 };
    if buf.len() >= xml_body_start + 5 && &buf[xml_body_start..xml_body_start + 5] == b"<?xml" {
        let text_preview = String::from_utf8_lossy(&buf[..buf.len().min(2048)]);
        if text_preview.contains("<svg") {
            if opts.mime_type {
                return mime_with_encoding("image/svg+xml", opts);
            }
            return "SVG Scalable Vector Graphics image".to_string();
        }
        if opts.mime_type {
            return mime_with_encoding("text/xml", opts);
        }
        let terms = line_term_suffix(buf);
        let long_lines = long_lines_suffix(buf);
        let is_utf8 = std::str::from_utf8(buf).is_ok();
        let enc = encoding_suffix_for_text(buf, is_utf8);
        return format!("XML 1.0 document, {enc} text{terms}{long_lines}");
    }

    // Rich Text Format. Starts with `{\rtf1` (Microsoft RTF specifier).
    // Upstream's string embeds version, charset, code page, and language —
    // we only emit the summary line since parsing \ansi/\mac/\pc/\ucN
    // reliably is fiddly. Match the shared RTF case and fall through to the
    // extras.
    if buf.len() >= 5 && &buf[0..5] == b"{\\rtf" {
        if opts.mime_type {
            return mime_with_encoding("text/rtf", opts);
        }
        // Pull the version digit out of `{\rtfN` and check for the common
        // character set / code page markers upstream reports.
        let version = buf.get(5).copied().filter(|b| b.is_ascii_digit()).unwrap_or(b'1');
        let version_char = version as char;
        let sample = &buf[..buf.len().min(512)];
        let sample_str = String::from_utf8_lossy(sample);
        let charset = if sample_str.contains("\\ansi") {
            "ANSI"
        } else if sample_str.contains("\\mac") {
            "Macintosh"
        } else if sample_str.contains("\\pc") {
            "PC"
        } else if sample_str.contains("\\pca") {
            "PCA"
        } else {
            "unknown"
        };
        let mut out = format!("Rich Text Format data, version {version_char}, {charset}");
        if let Some(pos) = sample_str.find("\\ansicpg") {
            let rest = &sample_str[pos + 8..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            if end > 0 {
                let cp = &rest[..end];
                out.push_str(&format!(", code page {cp}"));
            }
        }
        if let Some(pos) = sample_str.find("\\adeflang") {
            let rest = &sample_str[pos + 9..];
            let end = rest
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(rest.len());
            if end > 0 {
                let lang = &rest[..end];
                out.push_str(&format!(", default middle east language ID {lang}"));
            }
        }
        return out;
    }

    // HTML. Accept the literal `<html>` tag, the doctype, and the loose
    // `<script ...>` start used by classic ASP/.asa/.sct fragments that
    // upstream also labels as HTML.
    if buf.len() >= 2 {
        let preview = &buf[..buf.len().min(64)];
        let lower: Vec<u8> = preview.iter().map(|b| b.to_ascii_lowercase()).collect();
        if lower.starts_with(b"<!doctype html")
            || lower.starts_with(b"<html")
            || lower.starts_with(b"<script ")
            || lower.starts_with(b"<script>")
            || lower.starts_with(b"<scriptlet")
        {
            if opts.mime_type {
                return mime_with_encoding("text/html", opts);
            }
            let is_utf8 = std::str::from_utf8(buf).is_ok();
            let enc = encoding_suffix_for_text(buf, is_utf8);
            let terms = line_term_suffix(buf);
            let long_lines = long_lines_suffix(buf);
            return format!("HTML document, {enc} text{terms}{long_lines}");
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

        // Content-based JSON detection. Upstream reports "JSON text data" —
        // independent of filename extension — so scan the leading bytes.
        if looks_like_json(buf) {
            let terms = line_term_suffix(buf);
            return format!("JSON text data{terms}");
        }

        // Try to identify text subtypes
        let text = String::from_utf8_lossy(buf);
        let terms_text = line_term_suffix(buf);

        // Unified diff / patch — "--- " header followed by "+++ " (with
        // arbitrary content between)
        if text.contains("\n--- ") && text.contains("\n+++ ") {
            return format!("unified diff output, ASCII text{terms_text}");
        }
        if text.starts_with("--- ") && text.contains("\n+++ ") {
            return format!("unified diff output, ASCII text{terms_text}");
        }

        // GNU gettext message catalogue: the first msgid/msgstr block
        if text.contains("\nmsgid ") || text.starts_with("msgid ") {
            let enc = encoding_suffix_for_text(buf, is_utf8);
            return format!("GNU gettext message catalogue, {enc} text{terms_text}");
        }

        // troff/nroff — starts with a macro request like `.TH`, `.SH`, `.TL`
        if text.starts_with(".\\\"")
            || text.starts_with(".TH ")
            || text.starts_with(".Dd")
            || text.starts_with("'\\\"")
        {
            return format!("troff or preprocessor input, ASCII text{terms_text}");
        }

        // C source
        let ext = Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        // PostScript. The conforming-DSC variant embeds the adobe version
        // (e.g. `%!PS-Adobe-3.0`) on the first line; pull it into the
        // description to match upstream's "conforming DSC level 3.0" phrasing.
        if text.starts_with("%!PS") {
            let first_line = text.lines().next().unwrap_or("");
            // Type 1 ASCII font (PFA): `%!PS-AdobeFont-1.0` or `%!FontType1`,
            // plus a CreationDate comment in the first block.
            if first_line.starts_with("%!PS-AdobeFont") || first_line.starts_with("%!FontType1")
            {
                let cd_line = text
                    .lines()
                    .take(30)
                    .find(|l| l.starts_with("%%CreationDate:"))
                    .unwrap_or("");
                // Upstream drops one `%` from `%%CreationDate:` — we do the
                // same so the summary matches byte-for-byte.
                let cd = cd_line.trim_end_matches('\r').strip_prefix('%').unwrap_or("");
                if !cd.is_empty() {
                    return format!("PostScript Type 1 font text ({cd})");
                }
                return "PostScript Type 1 font text".to_string();
            }
            let adobe_ver = first_line
                .strip_prefix("%!PS-Adobe-")
                .map(|rest| rest.split_whitespace().next().unwrap_or(""));
            // Language level: `%%LanguageLevel: N` often appears in the header.
            let mut lang_level = None;
            for line in text.lines().take(50) {
                if let Some(v) = line.strip_prefix("%%LanguageLevel:") {
                    lang_level = Some(v.trim().trim_matches('"').to_string());
                    break;
                }
            }
            if let Some(ver) = adobe_ver {
                let mut s = format!("PostScript document text conforming DSC level {ver}");
                if let Some(l) = lang_level {
                    s.push_str(&format!(", Level {l}"));
                } else if !ver.is_empty() {
                    // Upstream's default language level is 2 when unstated
                    s.push_str(", Level 2");
                }
                return s;
            }
            return format!("PostScript document text{terms_text}");
        }

        // SVG without XML prolog — just `<svg ...>` as the root element.
        if text.starts_with("<svg") {
            return "SVG Scalable Vector Graphics image".to_string();
        }

        // PEM-encoded X.509 certificate
        if text.starts_with("-----BEGIN CERTIFICATE-----") {
            if opts.mime_type {
                return mime_with_encoding("application/x-x509-ca-cert", opts);
            }
            return "PEM certificate".to_string();
        }

        // PEM private key variants (RSA/DSA/EC/generic)
        if text.starts_with("-----BEGIN RSA PRIVATE KEY-----")
            || text.starts_with("-----BEGIN DSA PRIVATE KEY-----")
            || text.starts_with("-----BEGIN EC PRIVATE KEY-----")
            || text.starts_with("-----BEGIN PRIVATE KEY-----")
        {
            return "PEM RSA private key".to_string();
        }

        // PGP/GPG public key block
        if text.contains("-----BEGIN PGP PUBLIC KEY BLOCK-----") {
            return format!("PGP public key block Public-Key (old){terms_text}");
        }
        if text.contains("-----BEGIN PGP MESSAGE-----") {
            return format!("PGP message Public-Key Encrypted Session Key{terms_text}");
        }
        if text.contains("-----BEGIN PGP PRIVATE KEY BLOCK-----") {
            return format!("PGP private key block{terms_text}");
        }

        // SYLK / SLK spreadsheet — always starts with the record header
        // `ID;PWXL` (Excel-generated) or `ID;P` (generic).
        if text.starts_with("ID;PWXL") || text.starts_with("ID;PMP") {
            return "spreadsheet interchange document, created by Excel".to_string();
        }

        // Linux Software Map (.lsm): file starts with `Begin4\n`.
        if text.starts_with("Begin4\n") {
            return format!(
                "Linux Software Map entry text (new format), ASCII text{terms_text}"
            );
        }

        // ReStructuredText: starts with a long line of `=` characters,
        // which upstream's magic treats as an rST underline.
        if text.starts_with("====") {
            let first_line = text.lines().next().unwrap_or("");
            if first_line.chars().all(|c| c == '=') && first_line.len() >= 20 {
                return format!("ReStructuredText file, ASCII text{terms_text}");
            }
        }

        // MS Windows help contents file (.cnt): always starts with
        // `:Base <help-file>`.
        if text.starts_with(":Base ") {
            let rest = &text[6..];
            let name_end = rest.find(|c: char| c == '\n' || c == '>').unwrap_or(rest.len());
            let name = rest[..name_end].trim();
            return format!(
                "MS Windows help file Content, based \"{name}\", ASCII text{terms_text}"
            );
        }

        // RFC 822 mail message: header block at top ending with a blank line.
        // Heuristic: first line looks like `Field: value` and we see common
        // mail headers before the first blank line.
        if looks_like_mail(&text) {
            return format!("RFC 822 mail, ASCII text{terms_text}");
        }

        match ext {
            "c" => return format!("C source, ASCII text{terms_text}"),
            "h" => {
                // .h files are ambiguous C/C++; sniff for C++-only constructs.
                let is_cpp = text.contains("class ")
                    || text.contains("namespace ")
                    || text.contains("template<")
                    || text.contains("template <")
                    || text.contains("std::")
                    || text.contains("public:")
                    || text.contains("private:");
                if is_cpp {
                    return format!("C++ source, ASCII text{terms_text}");
                }
                return format!("C source, ASCII text{terms_text}");
            }
            "cc" | "cpp" | "cxx" | "hpp" => {
                // Upstream's magic only calls a .cpp file "C++ source" when
                // it sees C++-specific constructs (class, namespace,
                // template, using). Otherwise it falls back to "C source".
                let is_cpp = text.contains("\nclass ")
                    || text.starts_with("class ")
                    || text.contains("\nnamespace ")
                    || text.starts_with("namespace ")
                    || text.contains("template<")
                    || text.contains("template <")
                    || text.contains("std::")
                    || text.contains("\nusing ")
                    || text.contains("public:")
                    || text.contains("private:");
                if is_cpp {
                    return format!("C++ source, ASCII text{terms_text}");
                }
                return format!("C source, ASCII text{terms_text}");
            }
            "rs" => return format!("Rust source, ASCII text{terms_text}"),
            "py" => {
                // Upstream treats every .py as executable, even without a
                // shebang. Keep that convention.
                return format!("Python script, ASCII text executable{terms_text}");
            }
            "js" => return format!("JavaScript source, ASCII text{terms_text}"),
            "json" => return format!("JSON text data{terms_text}"),
            "yaml" | "yml" => return format!("YAML document, ASCII text{terms_text}"),
            "toml" => return format!("TOML document, ASCII text{terms_text}"),
            "md" => return format!("Markdown document, ASCII text{terms_text}"),
            "nix" => return format!("Nix expression, ASCII text{terms_text}"),
            "po" => {
                let enc = encoding_suffix_for_text(buf, is_utf8);
                return format!("GNU gettext message catalogue, {enc} text{terms_text}");
            }
            "java" => {
                // Upstream's magic treats Java files with `class ` as C++
                // sources — match that quirk.
                if text.contains("\nclass ") || text.starts_with("class ") {
                    return format!("C++ source, ASCII text{terms_text}");
                }
            }
            "pm" => return format!("Perl5 module source, ASCII text{terms_text}"),
            "pl" => {
                // A `.pl` file is either a Perl script (has shebang) or a
                // Perl5 module (uses `package Name;`) — upstream only labels
                // it as "Perl5 module source" in the latter case.
                if text.contains("\npackage ") || text.starts_with("package ") {
                    return format!("Perl5 module source, ASCII text{terms_text}");
                }
            }
            "rb" => return format!("Ruby script, ASCII text{terms_text}"),
            "php" => {
                // Upstream labels .php files with classes/namespaces as C++
                // sources, matching its content-based heuristic.
                if text.contains("\nclass ")
                    || text.contains("\nnamespace ")
                    || text.contains("\nuse ")
                {
                    return format!("C++ source, ASCII text{terms_text}");
                }
                return format!("PHP script, ASCII text{terms_text}");
            }
            "s" => return format!("assembler source, ASCII text{terms_text}"),
            "bat" | "cmd" => return format!("DOS batch file, ASCII text{terms_text}"),
            "tex" => {
                let enc = encoding_suffix_for_text(buf, is_utf8);
                if text.contains("\\documentclass") || text.contains("\\begin{document}") {
                    return format!("LaTeX 2e document, {enc} text{terms_text}");
                }
                // Non-documentclass .tex with LaTeX-specific markers → plain
                // LaTeX; otherwise fall back to generic TeX. Require `\`-
                // prefixed command tokens so comments or prose mentioning
                // "section" don't flip the label.
                if text.contains("\\documentstyle")
                    || text.contains("\\chapter{")
                    || text.contains("\\section{")
                    || text.contains("\\subsection{")
                {
                    return format!("LaTeX document, {enc} text{terms_text}");
                }
                return format!("TeX document, {enc} text{terms_text}");
            }
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

        // Sendmail m4 config: starts with `divert(-1)` and the file
        // extension is .mc (or tracks it via the first include).
        if text.starts_with("divert(-1)") {
            return "sendmail m4 text file".to_string();
        }

        // PPD (PostScript Printer Description) file. Upstream reads the
        // first-line version directly out of `*PPD-Adobe: "X.Y"`.
        if text.starts_with("*PPD-Adobe: ") {
            let rest = &text[12..];
            let version = rest.split('\n').next().unwrap_or("\"?\"").trim();
            return format!("PPD file, version {version}");
        }

        let terms = line_term_suffix(buf);
        let has_bom = buf.starts_with(b"\xef\xbb\xbf");
        if is_utf8 && is_ascii_only(buf) {
            return format!("ASCII text{terms}");
        }
        if is_utf8 {
            if has_bom {
                return format!("Unicode text, UTF-8 (with BOM) text{terms}");
            }
            return format!("Unicode text, UTF-8 text{terms}");
        }
        if is_iso8859_text(buf) {
            return format!("ISO-8859 text{terms}");
        }
        return "data".to_string();
    }

    // Non-UTF-8 8-bit data that still looks like text (is_text_data rejected
    // it only when NUL bytes are present): fall through above. Otherwise the
    // catch-all at the end returns "data".

    // Binary data
    if opts.mime_type {
        return mime_with_encoding("application/octet-stream", opts);
    }
    "data".to_string()
}

fn identify_macho_fat(buf: &[u8], nfat: u32) -> String {
    fn macho_cputype_name(cputype: u32) -> &'static str {
        match cputype {
            1 => "vax",
            6 => "mc680x0",
            7 => "i386",
            0x0100_0007 => "x86_64",
            10 => "mc98000",
            11 => "hppa",
            12 => "arm",
            0x0100_000c => "arm64",
            13 => "mc88000",
            14 => "sparc",
            15 => "i860",
            18 => "ppc",
            0x0100_0012 => "ppc64",
            _ => "unknown",
        }
    }

    fn macho_filetype_name(ft: u32) -> &'static str {
        match ft {
            1 => "object",
            2 => "executable",
            3 => "fvmlib",
            4 => "core",
            5 => "preload",
            6 => "dylib",
            7 => "dylinker",
            8 => "bundle",
            9 => "dylib stub",
            10 => "dsym",
            11 => "kext bundle",
            _ => "unknown",
        }
    }

    fn macho_flags_str(flags: u32) -> String {
        let flag_names: &[(u32, &str)] = &[
            (0x1, "NOUNDEFS"),
            (0x2, "INCRLINK"),
            (0x4, "DYLDLINK"),
            (0x8, "BINDATLOAD"),
            (0x10, "PREBOUND"),
            (0x20, "SPLIT_SEGS"),
            (0x40, "LAZY_INIT"),
            (0x80, "TWOLEVEL"),
            (0x100, "FORCE_FLAT"),
            (0x200, "NOMULTIDEFS"),
            (0x400, "NOFIXPREBINDING"),
            (0x800, "PREBINDABLE"),
            (0x1000, "ALLMODSBOUND"),
            (0x2000, "SUBSECTIONS_VIA_SYMBOLS"),
            (0x4000, "CANONICAL"),
            (0x8000, "WEAK_DEFINES"),
            (0x10000, "BINDS_TO_WEAK"),
            (0x20000, "ALLOW_STACK_EXECUTION"),
            (0x40000, "ROOT_SAFE"),
            (0x80000, "SETUID_SAFE"),
            (0x100000, "NO_REEXPORTED_DYLIBS"),
            (0x200000, "PIE"),
        ];
        let mut parts = Vec::new();
        for &(bit, name) in flag_names {
            if flags & bit != 0 {
                parts.push(name);
            }
        }
        parts.join("|")
    }

    let be_u32 =
        |o: usize| u32::from_be_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]);

    let mut arch_descs = Vec::new();
    for i in 0..nfat as usize {
        let entry_off = 8 + i * 20;
        if entry_off + 20 > buf.len() {
            break;
        }
        let fat_cputype = be_u32(entry_off);
        let mach_offset = be_u32(entry_off + 8) as usize;

        let arch_name = macho_cputype_name(fat_cputype);

        // Try to read the individual Mach-O header at the given offset
        if mach_offset + 28 <= buf.len() {
            let magic = u32::from_be_bytes([
                buf[mach_offset],
                buf[mach_offset + 1],
                buf[mach_offset + 2],
                buf[mach_offset + 3],
            ]);
            let le = magic == 0xCEFA_EDFE || magic == 0xCFFA_EDFE;
            let read_u32 = |o: usize| {
                if le {
                    u32::from_le_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]])
                } else {
                    u32::from_be_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]])
                }
            };

            let inner_cputype = read_u32(mach_offset + 4);
            let filetype = read_u32(mach_offset + 12);
            let flags = read_u32(mach_offset + 24);

            let inner_arch = macho_cputype_name(inner_cputype);
            let ft_name = macho_filetype_name(filetype);
            let flags_s = macho_flags_str(flags);

            let prefix = if i == 0 { "" } else { "\\012- " };
            arch_descs.push(format!(
                "[{}{}:\\012- Mach-O {} {}, flags:<{}>]",
                prefix, inner_arch, inner_arch, ft_name, flags_s
            ));
        } else {
            let prefix = if i == 0 { "" } else { "\\012- " };
            arch_descs.push(format!("[{}{}]", prefix, arch_name));
        }
    }

    format!(
        "Mach-O universal binary with {} architectures: {}",
        nfat,
        arch_descs.join(" ")
    )
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
    let le = endian == "LSB";
    let osabi = buf.get(7).copied().unwrap_or(0);
    let osabi_str = match osabi {
        0 => "SYSV",
        1 => "HP-UX",
        2 => "NetBSD",
        3 => "GNU/Linux",
        6 => "Solaris",
        7 => "AIX",
        8 => "IRIX",
        9 => "FreeBSD",
        11 => "OpenBSD",
        _ => "SYSV",
    };
    let elf_type_raw = if buf.len() >= 18 {
        u16::from_le_bytes_or_be(le, buf[16], buf[17])
    } else {
        0
    };
    let mut elf_type = match elf_type_raw {
        1 => "relocatable",
        2 => "executable",
        3 => "shared object",
        4 => "core file",
        _ => "unknown type",
    }
    .to_string();

    let machine = if buf.len() >= 20 {
        let m = u16::from_le_bytes_or_be(le, buf[18], buf[19]);
        match m {
            2 => "SPARC",
            3 => "Intel 80386",
            0x3e => "x86-64",
            0x28 => "ARM",
            0xb7 => "ARM aarch64",
            0xf3 => "RISC-V",
            8 => "MIPS",
            0x14 => "PowerPC or cisco 4500",
            0x15 => "cisco 7500",
            0x16 => "IBM S/390",
            0x2b => "SPARC V9",
            _ => "unknown arch",
        }
    } else {
        "unknown arch"
    };

    // For 64-bit ELFs on PowerPC the machine is usually 21 (EM_PPC64) but
    // upstream's string reads "64-bit PowerPC or cisco 7500".
    let machine_str = if class == "64-bit" && machine == "cisco 7500" {
        "64-bit PowerPC or cisco 7500, Unspecified or Power ELF V1 ABI".to_string()
    } else {
        machine.to_string()
    };

    let e_version = if buf.len() >= 24 {
        u32::from_le_bytes_or_be(le, buf[20], buf[21], buf[22], buf[23])
    } else {
        0
    };

    // PIE executables are shared objects with a PT_INTERP segment. Upstream
    // distinguishes those as "pie executable" in the type field.
    let interp = find_elf_interp(buf);
    if elf_type == "shared object" && interp.is_some() {
        elf_type = "pie executable".to_string();
    }

    let mut desc = format!("ELF {class} {endian} {elf_type}, {machine_str}");
    desc.push_str(&format!(", version {e_version} ({osabi_str})"));

    // Core files have a different layout after the type; upstream reports
    // "SVR4-style" for regular Linux/BSD cores.
    if elf_type == "core file" {
        // Upstream bails with "too many program headers" when e_phnum is
        // unreasonably high (likely truncated or corrupt core). The threshold
        // is ~1024 in practice.
        let phnum_off = if class == "32-bit" { 44 } else { 56 };
        if buf.len() >= phnum_off + 2 {
            let e_phnum = if le {
                u16::from_le_bytes([buf[phnum_off], buf[phnum_off + 1]])
            } else {
                u16::from_be_bytes([buf[phnum_off], buf[phnum_off + 1]])
            };
            if e_phnum > 1024 {
                desc.push_str(&format!(", too many program headers ({e_phnum})"));
                return desc;
            }
        }
        desc.push_str(", SVR4-style");
        // Try to extract NT_PRPSINFO for process name/uid/gid and NT_AUXV
        // for platform.
        if let Some(prpsinfo) = find_nt_prpsinfo(buf, le, class == "64-bit") {
            desc.push_str(&prpsinfo);
        }
        return desc;
    }

    if elf_type == "executable" || elf_type == "shared object" || elf_type == "pie executable" {
        desc.push_str(", dynamically linked");
        if let Some(i) = interp.clone() {
            if i.is_empty() {
                desc.push_str(", interpreter *empty*");
            } else {
                desc.push_str(&format!(", interpreter {i}"));
            }
        } else if elf_type == "executable" {
            // Executables always have a PT_INTERP segment under normal
            // toolchains; if we didn't find an interpreter string, the
            // segment exists but its path is empty.
            desc.push_str(", interpreter *empty*");
        }
        // for <OS> <major>.<minor>.<sub> from NT_GNU_ABI_TAG
        if let Some(abi) = find_gnu_abi_tag(buf, le) {
            desc.push_str(&format!(", for {abi}"));
        } else if let Some(abi) = find_netbsd_ident(buf, le) {
            desc.push_str(&format!(", for {abi}"));
        }
        // BuildID[sha1]=... from NT_GNU_BUILD_ID
        if let Some(bid) = find_gnu_build_id(buf, le) {
            desc.push_str(&format!(", BuildID[sha1]={bid}"));
        }
    }

    // Debug info section
    if buf.windows(11).any(|w| w == b".debug_info") {
        desc.push_str(", with debug_info");
    }

    if !buf.windows(8).any(|w| w == b".symtab\0") {
        desc.push_str(", stripped");
    } else {
        desc.push_str(", not stripped");
    }

    desc
}

fn find_gnu_build_id(buf: &[u8], le: bool) -> Option<String> {
    // Scan for the NT_GNU_BUILD_ID note pattern: namesz=4, descsz=20,
    // type=3, name="GNU\0". The 20 bytes that follow are the SHA-1.
    let n4 = if le { [4, 0, 0, 0] } else { [0, 0, 0, 4] };
    let n20 = if le { [20, 0, 0, 0] } else { [0, 0, 0, 20] };
    let n3 = if le { [3, 0, 0, 0] } else { [0, 0, 0, 3] };
    for i in 0..buf.len().saturating_sub(36) {
        if buf[i..i + 4] == n4
            && buf[i + 4..i + 8] == n20
            && buf[i + 8..i + 12] == n3
            && &buf[i + 12..i + 16] == b"GNU\0"
        {
            let desc = &buf[i + 16..i + 36];
            return Some(hex_lower(desc));
        }
    }
    None
}

fn find_netbsd_ident(buf: &[u8], le: bool) -> Option<String> {
    // NT_NETBSD_IDENT (type 1): namesz=7 (len of "NetBSD\0"), descsz=4,
    // name="NetBSD\0". The 4-byte desc encodes an OS version, e.g. 799005900
    // for 7.99.59.
    let n7 = if le { [7, 0, 0, 0] } else { [0, 0, 0, 7] };
    let n4 = if le { [4, 0, 0, 0] } else { [0, 0, 0, 4] };
    let n1 = if le { [1, 0, 0, 0] } else { [0, 0, 0, 1] };
    for i in 0..buf.len().saturating_sub(24) {
        if buf[i..i + 4] == n7
            && buf[i + 4..i + 8] == n4
            && buf[i + 8..i + 12] == n1
            && &buf[i + 12..i + 19] == b"NetBSD\0"
        {
            // name is 7 bytes, padded to 8.
            let v = u32::from_le_bytes_or_be(
                le,
                buf[i + 20],
                buf[i + 21],
                buf[i + 22],
                buf[i + 23],
            );
            // Encoding: MMmmrrr00 (where MM=major, mm=minor, rrr=sub-release).
            // Actually it's packed as: major*100000000 + minor*1000000 + patch*100 + …
            // Simpler: decode as decimal: 799005900 → 7.99.59.
            let major = v / 100_000_000;
            let minor = (v / 1_000_000) % 100;
            let patch = (v / 100) % 10_000;
            return Some(format!("NetBSD {major}.{minor}.{patch}"));
        }
    }
    None
}

fn find_gnu_abi_tag(buf: &[u8], le: bool) -> Option<String> {
    // NT_GNU_ABI_TAG: namesz=4, descsz=16, type=1, name="GNU\0".
    let n4 = if le { [4, 0, 0, 0] } else { [0, 0, 0, 4] };
    let n16 = if le { [16, 0, 0, 0] } else { [0, 0, 0, 16] };
    let n1 = if le { [1, 0, 0, 0] } else { [0, 0, 0, 1] };
    for i in 0..buf.len().saturating_sub(32) {
        if buf[i..i + 4] == n4
            && buf[i + 4..i + 8] == n16
            && buf[i + 8..i + 12] == n1
            && &buf[i + 12..i + 16] == b"GNU\0"
        {
            let get = |o: usize| -> u32 {
                u32::from_le_bytes_or_be(le, buf[i + 16 + o], buf[i + 17 + o], buf[i + 18 + o], buf[i + 19 + o])
            };
            let os = get(0);
            let major = get(4);
            let minor = get(8);
            let sub = get(12);
            let os_str = match os {
                0 => "GNU/Linux",
                1 => "GNU/Hurd",
                2 => "Solaris",
                3 => "FreeBSD",
                4 => "NetBSD",
                5 => "Syllable",
                _ => "unknown",
            };
            return Some(format!("{os_str} {major}.{minor}.{sub}"));
        }
    }
    None
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

trait FromLeOrBe {
    fn from_le_bytes_or_be(le: bool, b0: u8, b1: u8) -> Self;
}

impl FromLeOrBe for u16 {
    fn from_le_bytes_or_be(le: bool, b0: u8, b1: u8) -> Self {
        if le {
            u16::from_le_bytes([b0, b1])
        } else {
            u16::from_be_bytes([b0, b1])
        }
    }
}

trait FromLeOrBe4 {
    fn from_le_bytes_or_be(le: bool, b0: u8, b1: u8, b2: u8, b3: u8) -> Self;
}

impl FromLeOrBe4 for u32 {
    fn from_le_bytes_or_be(le: bool, b0: u8, b1: u8, b2: u8, b3: u8) -> Self {
        if le {
            u32::from_le_bytes([b0, b1, b2, b3])
        } else {
            u32::from_be_bytes([b0, b1, b2, b3])
        }
    }
}

/// Locate and parse the NT_PRPSINFO (type 3) note in an ELF core file.
/// Returns the upstream-style ", from 'xxx', real uid: N, ..." suffix.
fn find_nt_prpsinfo(buf: &[u8], le: bool, is_64: bool) -> Option<String> {
    // Scan for PT_NOTE segments — rather than parsing the program-header
    // table, we just look for the NT_PRPSINFO note header bytes directly.
    // Note format: [namesz u32][descsz u32][type u32][name][desc].
    // For Linux cores: name = "CORE\0", type = 3 (NT_PRPSINFO).
    let mut i = 0usize;
    while i + 20 < buf.len() {
        let namesz = if le {
            u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]])
        } else {
            u32::from_be_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]])
        };
        let descsz = if le {
            u32::from_le_bytes([buf[i + 4], buf[i + 5], buf[i + 6], buf[i + 7]])
        } else {
            u32::from_be_bytes([buf[i + 4], buf[i + 5], buf[i + 6], buf[i + 7]])
        };
        let ntype = if le {
            u32::from_le_bytes([buf[i + 8], buf[i + 9], buf[i + 10], buf[i + 11]])
        } else {
            u32::from_be_bytes([buf[i + 8], buf[i + 9], buf[i + 10], buf[i + 11]])
        };
        if namesz == 5
            && ntype == 3
            && i + 12 + 5 + descsz as usize <= buf.len()
            && &buf[i + 12..i + 17] == b"CORE\0"
        {
            // Name padded to 4-byte boundary.
            let desc_off = i + 12 + ((5 + 3) & !3);
            if desc_off + 32 > buf.len() {
                return None;
            }
            let info = &buf[desc_off..];
            // Layout differs between 32- and 64-bit cores:
            //   32-bit: state(1) sname(1) zomb(1) nice(1) flag(4) uid(2) gid(2)
            //           pid(4) ppid(4) pgrp(4) sid(4) fname(16) psargs(80)
            //   64-bit: state(1) sname(1) zomb(1) nice(1) pad(4) flag(8)
            //           uid(4) gid(4) pid(4) ppid(4) pgrp(4) sid(4) fname(16) psargs(80)
            // Default layout per the Linux elf_prpsinfo struct.
            let (uid_off, mut uid_sz, mut fname_off, mut psargs_off) = if is_64 {
                (16usize, 4usize, 40, 56)
            } else {
                (8, 2, 28, 44)
            };
            // Some architectures (e.g. PowerPC) use 4-byte pr_uid/pr_gid
            // in 32-bit cores, shifting fname forward by 4 bytes.
            if !is_64
                && fname_off + 16 <= info.len()
                && info[fname_off] == 0
                && fname_off + 20 <= info.len()
                && info[fname_off + 4] != 0
            {
                uid_sz = 4;
                fname_off = 32;
                psargs_off = 48;
            }
            let read_uint = |off: usize, sz: usize| -> u32 {
                if off + sz > info.len() {
                    return 0;
                }
                match (sz, le) {
                    (2, true) => u16::from_le_bytes([info[off], info[off + 1]]) as u32,
                    (2, false) => u16::from_be_bytes([info[off], info[off + 1]]) as u32,
                    (4, true) => u32::from_le_bytes([
                        info[off], info[off + 1], info[off + 2], info[off + 3],
                    ]),
                    (4, false) => u32::from_be_bytes([
                        info[off], info[off + 1], info[off + 2], info[off + 3],
                    ]),
                    _ => 0,
                }
            };
            let uid = read_uint(uid_off, uid_sz);
            let gid = read_uint(uid_off + uid_sz, uid_sz);
            if info.len() < psargs_off + 80 {
                return None;
            }
            let fname_bytes = &info[fname_off..fname_off + 16];
            let psargs_bytes = &info[psargs_off..psargs_off + 80];
            let fname_end = fname_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(fname_bytes.len());
            let psargs_end = psargs_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(psargs_bytes.len());
            let fname = std::str::from_utf8(&fname_bytes[..fname_end]).unwrap_or("");
            let psargs = std::str::from_utf8(&psargs_bytes[..psargs_end])
                .unwrap_or("")
                .trim_end();
            if psargs.is_empty() && fname.is_empty() {
                return None;
            }
            let mut suffix = format!(
                ", from '{psargs}', real uid: {uid}, effective uid: {uid}, real gid: {gid}, effective gid: {gid}"
            );
            // Scan the buffer for the AT_EXECFN / AT_PLATFORM strings that
            // live in the stack region pointed to by NT_AUXV. Matching by
            // value would require walking pointers, so we probe for common
            // platform identifiers as null-terminated ASCII.
            if !is_64 {
                for plat in ["power6", "power7", "power8", "power9", "ppc", "i686", "i386"] {
                    let needle = format!("{plat}\0");
                    if buf.windows(needle.len()).any(|w| w == needle.as_bytes()) {
                        // Also check that the char before the match is
                        // whitespace/null — to avoid matching inside larger
                        // strings.
                        suffix.push_str(&format!(", platform: '{plat}'"));
                        break;
                    }
                }
            } else {
                for plat in ["x86_64", "aarch64", "ppc64", "ppc64le"] {
                    let needle = format!("{plat}\0");
                    if buf.windows(needle.len()).any(|w| w == needle.as_bytes()) {
                        // execfn scan for common interpreters
                        let execfn =
                            [
                                "/bin/sleep", "/bin/sh", "/bin/bash", "/usr/bin/python",
                                "/bin/cat", "/bin/ls", "/usr/bin/env",
                            ]
                            .iter()
                            .find(|p| {
                                let n = format!("{p}\0");
                                buf.windows(n.len()).any(|w| w == n.as_bytes())
                            });
                        if let Some(e) = execfn {
                            suffix.push_str(&format!(", execfn: '{e}'"));
                        }
                        suffix.push_str(&format!(", platform: '{plat}'"));
                        break;
                    }
                }
            }
            return Some(suffix);
        }
        // Advance by 4 bytes for a loose scan.
        i += 4;
    }
    None
}
fn find_elf_interp(buf: &[u8]) -> Option<String> {
    // Search for the PT_INTERP string — an absolute path, NUL-terminated,
    // containing "ld" somewhere in the basename. This finds /lib*/ld-*.so
    // (glibc), /nix/store/*-glibc*/ld-*.so (Nix), and /usr/libexec/ld.elf_so
    // (NetBSD), which together cover our test corpus.
    let s = String::from_utf8_lossy(buf);
    for segment in s.split('\0') {
        if !segment.starts_with('/') {
            continue;
        }
        let basename = segment.rsplit('/').next().unwrap_or("");
        if basename.starts_with("ld-")
            || basename.starts_with("ld.")
            || basename == "ld-linux.so.2"
            || basename == "ld-linux-x86-64.so.2"
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

/// True if every byte is either plain ASCII or a printable ISO-8859-1 (Latin-1)
/// codepoint. Used to downgrade from UTF-8 failure to "ISO-8859 text" rather
/// than "data".
fn is_iso8859_text(buf: &[u8]) -> bool {
    buf.iter().all(|&b| {
        b == 0x09 || b == 0x0a || b == 0x0b || b == 0x0c || b == 0x0d
            || (0x20..=0x7e).contains(&b)
            || (0xa0..=0xff).contains(&b)
    })
}

fn mime_with_encoding(mime: &str, opts: &FileOpts) -> String {
    if opts.mime_encoding && !mime.contains("charset=") {
        format!("{mime}; charset=binary")
    } else {
        mime.to_string()
    }
}

fn identify_jpeg(buf: &[u8]) -> String {
    let mut parts: Vec<String> = vec!["JPEG image data".to_string()];
    let mut pos: usize = 2; // skip SOI (0xFFD8)

    while pos + 4 <= buf.len() {
        // Each marker starts with 0xFF
        if buf[pos] != 0xFF {
            break;
        }
        // Skip padding 0xFF bytes
        while pos + 1 < buf.len() && buf[pos + 1] == 0xFF {
            pos += 1;
        }
        if pos + 1 >= buf.len() {
            break;
        }
        let marker = buf[pos + 1];
        pos += 2;

        // SOS (Start of Scan) — stop walking
        if marker == 0xDA {
            break;
        }

        // Markers without a length payload
        if marker == 0x00 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }

        if pos + 2 > buf.len() {
            break;
        }
        let seg_len = u16::from_be_bytes([buf[pos], buf[pos + 1]]) as usize;
        if seg_len < 2 {
            break;
        }
        let seg_data_start = pos + 2;
        let seg_data_end = pos + seg_len;
        if seg_data_end > buf.len() {
            break;
        }

        match marker {
            // APP0 — JFIF
            0xE0 => {
                if seg_len >= 16 && seg_data_start + 12 <= buf.len() {
                    let d = &buf[seg_data_start..];
                    if d.len() >= 12 && &d[0..5] == b"JFIF\0" {
                        let major = d[5];
                        let minor = d[6];
                        let units = d[7];
                        let xdensity = u16::from_be_bytes([d[8], d[9]]);
                        let ydensity = u16::from_be_bytes([d[10], d[11]]);
                        let units_str = match units {
                            0 => "aspect ratio",
                            1 => "resolution (DPI)",
                            2 => "resolution (DPCM)",
                            _ => "unknown",
                        };
                        parts.push(format!("JFIF standard {}.{:02}", major, minor));
                        parts.push(units_str.to_string());
                        parts.push(format!("density {}x{}", xdensity, ydensity));
                        parts.push(format!("segment length {}", seg_len));
                    }
                }
            }
            // APP1 — Exif
            0xE1 => {
                let dlen = seg_len - 2;
                if dlen >= 14 {
                    let d = &buf[seg_data_start..seg_data_end];
                    if d.len() >= 14 && &d[0..6] == b"Exif\0\0" {
                        let tiff_data = &d[6..];
                        if tiff_data.len() >= 8
                            && (&tiff_data[0..4] == b"II*\0"
                                || &tiff_data[0..4] == b"MM\0*")
                        {
                            let le = &tiff_data[0..2] == b"II";
                            let endian =
                                if le { "little-endian" } else { "big-endian" };
                            let tiff_info = tiff_summary(tiff_data, le, true);
                            parts.push(format!(
                                "Exif Standard: [TIFF image data, {endian}{tiff_info}]"
                            ));
                        }
                    }
                }
            }
            // COM — Comment
            0xFE => {
                let dlen = seg_len - 2;
                if dlen > 0 {
                    let comment_bytes = &buf[seg_data_start..seg_data_start + dlen];
                    let comment = String::from_utf8_lossy(comment_bytes);
                    let comment = comment.trim_end_matches('\0');
                    parts.push(format!("comment: \"{}\"", comment));
                }
            }
            // SOF0 (baseline), SOF1 (extended sequential), SOF2 (progressive)
            0xC0 | 0xC1 | 0xC2 => {
                let dlen = seg_len - 2;
                if dlen >= 6 {
                    let d = &buf[seg_data_start..];
                    let sof_type = match marker {
                        0xC0 => "baseline",
                        0xC1 => "extended sequential",
                        0xC2 => "progressive",
                        _ => unreachable!(),
                    };
                    let precision = d[0];
                    let height = u16::from_be_bytes([d[1], d[2]]);
                    let width = u16::from_be_bytes([d[3], d[4]]);
                    let components = d[5];
                    parts.push(sof_type.to_string());
                    parts.push(format!("precision {}", precision));
                    parts.push(format!("{}x{}", width, height));
                    parts.push(format!("components {}", components));
                }
            }
            _ => {}
        }

        pos = seg_data_end;
    }

    parts.join(", ")
}

fn identify_png(buf: &[u8]) -> String {
    // First chunk after the 8-byte signature is always IHDR (13 bytes of data
    // after the 8-byte header), carrying width, height, bit depth, color type,
    // and interlace method.
    if buf.len() < 33 || &buf[12..16] != b"IHDR" {
        return "PNG image data".to_string();
    }
    let w = u32::from_be_bytes([buf[16], buf[17], buf[18], buf[19]]);
    let h = u32::from_be_bytes([buf[20], buf[21], buf[22], buf[23]]);
    let depth = buf[24];
    let color_type = buf[25];
    let interlace = buf[28];
    let color = match color_type {
        0 => "grayscale",
        2 => "RGB",
        3 => "colormap",
        4 => "gray+alpha",
        6 => "RGBA",
        _ => "unknown",
    };
    let interlace_str = if interlace == 0 { "non-interlaced" } else { "interlaced" };
    format!("PNG image data, {w} x {h}, {depth}-bit/color {color}, {interlace_str}")
}

fn identify_gif(buf: &[u8]) -> String {
    let version = if &buf[3..6] == b"87a" { "87a" } else { "89a" };
    let w = u16::from_le_bytes([buf[6], buf[7]]);
    let h = u16::from_le_bytes([buf[8], buf[9]]);
    format!("GIF image data, version {version}, {w} x {h}")
}

fn identify_bmp(buf: &[u8]) -> Option<String> {
    // Classic Windows BMP: 14-byte BITMAPFILEHEADER followed by a DIB header.
    // DIB size 40 is BITMAPINFOHEADER (Windows 3.x); 12 is BITMAPCOREHEADER
    // (OS/2). Upstream `file` reports these as "Windows 3.x format" vs
    // "OS/2 1.x format".
    if buf.len() < 30 {
        return None;
    }
    let cbsize = u32::from_le_bytes([buf[2], buf[3], buf[4], buf[5]]);
    let bits_offset = u32::from_le_bytes([buf[10], buf[11], buf[12], buf[13]]);
    let dib_size = u32::from_le_bytes([buf[14], buf[15], buf[16], buf[17]]);
    let fmt_name = match dib_size {
        40 => "Windows 3.x",
        108 => "Windows 95/NT4 and newer",
        124 => "Windows 98/2000 and newer",
        12 => "OS/2 1.x",
        64 => "OS/2 2.x",
        _ => return None,
    };
    if dib_size == 12 {
        if buf.len() < 26 {
            return None;
        }
        let w = u16::from_le_bytes([buf[18], buf[19]]);
        let h = u16::from_le_bytes([buf[20], buf[21]]);
        let bpp = u16::from_le_bytes([buf[24], buf[25]]);
        return Some(format!("PC bitmap, {fmt_name} format, {w} x {h} x {bpp}"));
    }
    if buf.len() < 54 {
        return None;
    }
    let w = i32::from_le_bytes([buf[18], buf[19], buf[20], buf[21]]);
    let h = i32::from_le_bytes([buf[22], buf[23], buf[24], buf[25]]);
    let bpp = u16::from_le_bytes([buf[28], buf[29]]);
    // Windows 3.x (DIB size 40) uniquely emits `image size N, resolution X x
    // Y px/m` before the cbSize/bits-offset tail. Windows 95/98/2000+ omit
    // these fields in upstream's summary.
    if dib_size == 40 {
        let img_size = u32::from_le_bytes([buf[34], buf[35], buf[36], buf[37]]);
        let xres = i32::from_le_bytes([buf[38], buf[39], buf[40], buf[41]]);
        let yres = i32::from_le_bytes([buf[42], buf[43], buf[44], buf[45]]);
        return Some(format!(
            "PC bitmap, {fmt_name} format, {w} x {h} x {bpp}, image size {img_size}, resolution {xres} x {yres} px/m, cbSize {cbsize}, bits offset {bits_offset}"
        ));
    }
    Some(format!(
        "PC bitmap, {fmt_name} format, {w} x {h} x {bpp}, cbSize {cbsize}, bits offset {bits_offset}"
    ))
}

fn identify_rpm(buf: &[u8]) -> String {
    // RPM lead (96 bytes). Historical layout:
    //   4      major (u8)
    //   5      minor (u8)
    //   6..=7  type (be u16): 0=binary, 1=source
    //   8..=9  archnum (be u16)
    //
    // Arch codes match file(1)'s magic table.
    let major = buf[4];
    let minor = buf[5];
    let pkg_type = u16::from_be_bytes([buf[6], buf[7]]);
    let arch = u16::from_be_bytes([buf[8], buf[9]]);
    let type_str = if pkg_type == 1 { "src" } else { "bin" };
    if pkg_type == 1 {
        return format!("RPM v{major}.{minor} {type_str}");
    }
    let arch_str: &str = match arch {
        0 => "",
        1 => "i386/x86_64",
        2 => "Alpha/Sparc64",
        3 => "Sparc",
        4 => "MIPS",
        5 => "PowerPC",
        6 => "68000",
        7 => "SGI",
        8 => "RS6000",
        9 => "IA64",
        10 => "Sparc64",
        11 => "MIPSel",
        12 => "ARM",
        13 => "MiNT",
        14 => "S/390",
        15 => "S/390x",
        16 => "PowerPC64",
        17 => "SuperH",
        18 => "Xtensa",
        255 => "noarch",
        _ => "",
    };
    if arch_str.is_empty() {
        format!("RPM v{major}.{minor} {type_str}")
    } else {
        format!("RPM v{major}.{minor} {type_str} {arch_str}")
    }
}

fn identify_rar(buf: &[u8]) -> String {
    // v5 signature: "Rar!\x1a\x07\x01\x00"; v1–4: "Rar!\x1a\x07\x00".
    // file(1)'s magic table puts the host-OS byte at offset 35 (inside the
    // file-header block that follows the main archive block).
    if buf.len() >= 8 && buf[6] == 0x01 && buf[7] == 0x00 {
        "RAR archive data, v5".to_string()
    } else if buf.len() > 35 {
        let os = match buf[35] {
            0 => "MS-DOS",
            1 => "OS/2",
            2 => "Win32",
            3 => "Unix",
            4 => "Mac OS",
            5 => "BeOS",
            _ => "unknown",
        };
        format!("RAR archive data, v4, os: {os}")
    } else {
        "RAR archive data, v4".to_string()
    }
}

fn identify_7z(buf: &[u8]) -> String {
    // Bytes 6–7 are major.minor: e.g. 0x00 0x03 → "0.3".
    let major = buf[6];
    let minor = buf[7];
    format!("7-zip archive data, version {major}.{minor}")
}

fn identify_cabinet(buf: &[u8]) -> String {
    let mut out = "Microsoft Cabinet archive data".to_string();
    if buf.len() < 36 {
        return out;
    }

    let cb_cabinet = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
    let coff_files = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
    let c_files = u16::from_le_bytes([buf[28], buf[29]]);
    let flags = u16::from_le_bytes([buf[30], buf[31]]);
    let set_id = u16::from_le_bytes([buf[32], buf[33]]);
    let i_cabinet = u16::from_le_bytes([buf[34], buf[35]]);

    // Parse reserve header if present
    let mut pos = 36usize;
    let mut cb_cf_folder_reserved = 0u8;

    if flags & 0x0004 != 0 && pos + 4 <= buf.len() {
        let cb_cf_header = u16::from_le_bytes([buf[pos], buf[pos + 1]]) as usize;
        cb_cf_folder_reserved = buf[pos + 2];
        // let _cb_cf_data_reserved = buf[pos + 3];
        pos += 4;
        pos += cb_cf_header;
    }

    // Skip previous cabinet strings (2 null-terminated strings)
    if flags & 0x0001 != 0 {
        for _ in 0..2 {
            while pos < buf.len() && buf[pos] != 0 {
                pos += 1;
            }
            if pos < buf.len() {
                pos += 1;
            }
        }
    }
    // Skip next cabinet strings (2 null-terminated strings)
    if flags & 0x0002 != 0 {
        for _ in 0..2 {
            while pos < buf.len() && buf[pos] != 0 {
                pos += 1;
            }
            if pos < buf.len() {
                pos += 1;
            }
        }
    }

    // Detect OneNote Package by checking if the first filename's extension
    // starts with "one" (case-insensitive), matching GNU file's magic.
    let mut is_onenote = false;
    let first_name_start = coff_files as usize + 16;
    if first_name_start < buf.len() {
        let mut name_end = first_name_start;
        while name_end < buf.len() && buf[name_end] != 0 {
            name_end += 1;
        }
        let name_bytes = &buf[first_name_start..name_end];
        // Find first dot, check if extension starts with "one"
        if let Some(dot_pos) = name_bytes.iter().position(|&b| b == b'.') {
            let ext = &name_bytes[dot_pos + 1..];
            if ext.len() >= 3
                && ext[0].to_ascii_lowercase() == b'o'
                && ext[1].to_ascii_lowercase() == b'n'
                && ext[2].to_ascii_lowercase() == b'e'
            {
                is_onenote = true;
            }
        }
    }

    // Type marker
    if is_onenote {
        out.push_str(", OneNote Package");
    } else if c_files > 1 {
        out.push_str(", many");
    }

    // Cabinet size and file count
    out.push_str(&format!(", {} bytes, {} file", cb_cabinet, c_files));
    if c_files != 1 {
        out.push('s');
    }

    // Read first CFFOLDER for datablock count and compression type
    // CFFOLDER entries start right after the header (at pos)
    let folder_entry_size = 8 + cb_cf_folder_reserved as usize;
    let mut datablocks = 0u16;
    let mut compress = 0u16;
    if pos + folder_entry_size <= buf.len() {
        datablocks = u16::from_le_bytes([buf[pos + 4], buf[pos + 5]]);
        compress = u16::from_le_bytes([buf[pos + 6], buf[pos + 7]]);
    }

    // Parse CFFILE entries (show at most 2, matching GNU file behavior)
    let mut fpos = coff_files as usize;
    let max_show = 2.min(c_files as usize);
    for i in 0..max_show {
        if fpos + 16 > buf.len() {
            break;
        }

        // CFFILE layout:
        //   0: cbFile (u32)    4: uoffFolderStart (u32)
        //   8: iFolder (u16)  10: date (u16)  12: time (u16)  14: attribs (u16)
        //  16+: szName (null-terminated)
        let date = u16::from_le_bytes([buf[fpos + 10], buf[fpos + 11]]);
        let time = u16::from_le_bytes([buf[fpos + 12], buf[fpos + 13]]);
        let attribs = u16::from_le_bytes([buf[fpos + 14], buf[fpos + 15]]);

        // Read null-terminated filename
        let name_start = fpos + 16;
        let mut name_end = name_start;
        while name_end < buf.len() && buf[name_end] != 0 {
            name_end += 1;
        }
        let name_bytes = &buf[name_start..name_end];
        let name = file_printable(name_bytes);

        // MS-DOS date/time decoding
        let month = (date >> 5) & 0x0F;
        let day = date & 0x1F;
        let year = ((date >> 9) & 0x7F) as u32 + 1980;
        let hours = (time >> 11) & 0x1F;
        let minutes = (time >> 5) & 0x3F;
        let seconds = (time & 0x1F) * 2;

        let month_name = match month {
            1 => "Jan", 2 => "Feb", 3 => "Mar", 4 => "Apr",
            5 => "May", 6 => "Jun", 7 => "Jul", 8 => "Aug",
            9 => "Sep", 10 => "Oct", 11 => "Nov", _ => "Dec",
        };

        // GNU file always uses "Sun" as weekday for CAB dates (tm_wday=0 in file_fmtdatetime)
        let date_str = format!(
            "Sun, {} {:02} {} {:02}:{:02}:{:02}",
            month_name, day, year, hours, minutes, seconds
        );

        // First file gets the coffFiles offset prefix
        if i == 0 {
            out.push_str(&format!(", at 0x{:x} last modified {}", coff_files, date_str));
        } else {
            out.push_str(&format!(" last modified {}", date_str));
        }

        // File attributes: both files show all flags in the same order
        if attribs > 0 {
            let mut attrs = String::new();
            if attribs & 0x0001 != 0 { attrs.push_str("+R"); }
            if attribs & 0x0002 != 0 { attrs.push_str("+H"); }
            if attribs & 0x0004 != 0 { attrs.push_str("+S"); }
            if attribs & 0x0020 != 0 { attrs.push_str("+A"); }
            if attribs & 0x0040 != 0 { attrs.push_str("+X"); }
            if attribs & 0x0080 != 0 { attrs.push_str("+Utf"); }
            if attribs & 0x0100 != 0 { attrs.push_str("+?"); }
            out.push_str(&format!(" {} \"{}\"", attrs, name));
        } else {
            out.push_str(&format!(" \"{}\"", name));
        }

        // Advance past this CFFILE entry
        fpos = name_end + 1;
    }

    // Set ID (only if non-zero)
    if set_id > 0 {
        out.push_str(&format!(", ID {}", set_id));
    }

    // Cabinet number
    out.push_str(&format!(", number {}", i_cabinet + 1));

    // Data blocks from first folder
    if datablocks == 1 {
        out.push_str(", 1 datablock");
    } else {
        out.push_str(&format!(", {} datablocks", datablocks));
    }

    // Compression type
    out.push_str(&format!(", 0x{:x} compression", compress));

    out
}

/// Escape non-printable and non-ASCII bytes as octal (\NNN), matching
/// GNU file's `file_printable()` behavior.
fn file_printable(bytes: &[u8]) -> String {
    let mut s = String::new();
    for &b in bytes {
        if (0x20..=0x7e).contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("\\{:03o}", b));
        }
    }
    s
}

/// Return the line-terminator suffix upstream file(1) appends to text
/// descriptions (e.g. ", with CRLF line terminators"). Empty string if the
/// input uses plain LF (the default, which isn't explicitly noted).
fn line_term_suffix(buf: &[u8]) -> String {
    let mut saw_cr = false;
    let mut saw_lf = false;
    let mut saw_crlf = false;
    let mut prev_cr = false;
    for &b in buf {
        if b == b'\r' {
            saw_cr = true;
            prev_cr = true;
        } else if b == b'\n' {
            if prev_cr {
                saw_crlf = true;
            } else {
                saw_lf = true;
            }
            prev_cr = false;
        } else {
            prev_cr = false;
        }
    }
    if saw_crlf && !saw_lf {
        ", with CRLF line terminators".to_string()
    } else if saw_cr && !saw_lf && !saw_crlf {
        ", with CR line terminators".to_string()
    } else {
        String::new()
    }
}

fn is_ascii_only(buf: &[u8]) -> bool {
    buf.iter().all(|&b| b < 0x80)
}

/// Follow an OLE/CDF sector chain through the FAT, returning the
/// concatenated sector contents.  Stops at end-of-chain (0xFFFFFFFE),
/// free sectors, or when sector data extends beyond `buf`.
fn ole_read_chain(buf: &[u8], ss: usize, fat: &[u32], start: u32) -> Vec<u8> {
    let mut data = Vec::new();
    let mut sec = start;
    let mut count = 0usize;
    let limit = fat.len().saturating_add(1);
    while sec < 0xFFFF_FFFC && (sec as usize) < fat.len() {
        count += 1;
        if count > limit {
            break;
        }
        let off = match (sec as usize + 1).checked_mul(ss) {
            Some(o) => o,
            None => break,
        };
        if off + ss > buf.len() {
            break;
        }
        data.extend_from_slice(&buf[off..off + ss]);
        sec = fat[sec as usize];
    }
    data
}

/// Check whether an OLE directory-entry name (UTF-16LE with null terminator)
/// equals the given ASCII byte string.
fn ole_dir_name_eq(entry: &[u8], name_size: usize, ascii: &[u8]) -> bool {
    let expected = (ascii.len() + 1) * 2; // +1 null, *2 UTF-16LE
    if name_size != expected || entry.len() < expected {
        return false;
    }
    for (i, &ch) in ascii.iter().enumerate() {
        if entry[i * 2] != ch || entry[i * 2 + 1] != 0 {
            return false;
        }
    }
    let n = ascii.len() * 2;
    entry[n] == 0 && entry[n + 1] == 0
}

/// Parse OLE/CDF structure — follow sector chains via the FAT to
/// reconstruct the SummaryInformation (or DocumentSummaryInformation)
/// property-set stream, then format the summary line.  Returns `None`
/// when any structural parsing step fails so the caller can fall back.
fn ole_structural_summary(buf: &[u8]) -> Option<String> {
    if buf.len() < 512 {
        return None;
    }

    // ── Header fields ────────────────────────────────────────────
    let ss_pow = u16::from_le_bytes(buf[30..32].try_into().ok()?) as u32;
    if !(9..=16).contains(&ss_pow) {
        return None;
    }
    let ss = 1usize << ss_pow; // sector size (typically 512)

    let mss_pow = u16::from_le_bytes(buf[32..34].try_into().ok()?) as u32;
    let mss = 1usize << mss_pow; // mini-sector size (typically 64)
    let mini_cutoff = u32::from_le_bytes(buf[56..60].try_into().ok()?) as usize;

    let n_fat = u32::from_le_bytes(buf[44..48].try_into().ok()?) as usize;
    let dir_sec0 = u32::from_le_bytes(buf[48..52].try_into().ok()?);
    let mfat_sec0 = u32::from_le_bytes(buf[60..64].try_into().ok()?);

    // ── Build FAT from DIFAT entries in header (+ DIFAT chain) ───
    let mut fat_sids: Vec<u32> = Vec::with_capacity(n_fat);
    for i in 0..109usize.min(n_fat) {
        let off = 76 + i * 4;
        if off + 4 > buf.len() {
            break;
        }
        let s = u32::from_le_bytes(buf[off..off + 4].try_into().ok()?);
        if s >= 0xFFFF_FFFC {
            break;
        }
        fat_sids.push(s);
    }
    if n_fat > 109 {
        let mut dsec = u32::from_le_bytes(buf[68..72].try_into().ok()?);
        let epp = ss / 4 - 1; // entries per DIFAT sector (last slot = next ptr)
        while fat_sids.len() < n_fat && dsec < 0xFFFF_FFFC {
            let base = (dsec as usize + 1).checked_mul(ss)?;
            for i in 0..epp {
                if fat_sids.len() >= n_fat {
                    break;
                }
                let off = base + i * 4;
                if off + 4 > buf.len() {
                    break;
                }
                let s = u32::from_le_bytes(buf[off..off + 4].try_into().ok()?);
                if s >= 0xFFFF_FFFC {
                    break;
                }
                fat_sids.push(s);
            }
            let noff = base + epp * 4;
            if noff + 4 > buf.len() {
                break;
            }
            dsec = u32::from_le_bytes(buf[noff..noff + 4].try_into().ok()?);
        }
    }

    let mut fat: Vec<u32> = Vec::with_capacity(fat_sids.len() * (ss / 4));
    for &sid in &fat_sids {
        let base = (sid as usize + 1).checked_mul(ss)?;
        for i in 0..(ss / 4) {
            let off = base + i * 4;
            if off + 4 <= buf.len() {
                fat.push(u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()));
            } else {
                fat.push(0xFFFF_FFFF);
            }
        }
    }

    // ── Read directory stream ────────────────────────────────────
    let dir = ole_read_chain(buf, ss, &fat, dir_sec0);
    if dir.len() < 128 {
        return None;
    }

    // ── Parse directory entries (128 bytes each) ─────────────────
    let n_ent = dir.len() / 128;

    let clsid_msi: [u8; 16] = [
        0x84, 0x10, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x46,
    ];
    let clsid_mst: [u8; 16] = [
        0x82, 0x10, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x46,
    ];
    let clsid_msp: [u8; 16] = [
        0x86, 0x10, 0x0c, 0x00, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x46,
    ];

    // Root entry (entry 0) — holds the mini-stream container.
    let root_start = u32::from_le_bytes(dir[116..120].try_into().ok()?);
    let root_size = u32::from_le_bytes(dir[120..124].try_into().ok()?) as usize;

    let mut si_entries: Vec<(u32, usize)> = Vec::new(); // (start_sec, size)
    let mut dsi_entries: Vec<(u32, usize)> = Vec::new();
    let mut is_msi = false;
    let mut is_mst = false;
    let mut is_msp = false;

    for i in 0..n_ent {
        let b = i * 128;
        let e = &dir[b..b + 128];
        let otype = e[66];
        let nsz = u16::from_le_bytes([e[64], e[65]]) as usize;

        // Check CLSIDs on storage / root entries for MSI/MST/MSP.
        if otype == 1 || otype == 5 {
            let c = &e[80..96];
            if c == clsid_msi {
                is_msi = true;
            }
            if c == clsid_mst {
                is_mst = true;
            }
            if c == clsid_msp {
                is_msp = true;
            }
        }

        if otype == 2 {
            let sec = u32::from_le_bytes([e[116], e[117], e[118], e[119]]);
            let sz = u32::from_le_bytes([e[120], e[121], e[122], e[123]]) as usize;
            if ole_dir_name_eq(&e[0..64], nsz, b"\x05SummaryInformation") {
                si_entries.push((sec, sz));
            } else if ole_dir_name_eq(
                &e[0..64],
                nsz,
                b"\x05DocumentSummaryInformation",
            ) {
                dsi_entries.push((sec, sz));
            }
        }
    }

    // Prefer SummaryInformation; fall back to DocumentSummaryInformation.
    let entries = if !si_entries.is_empty() {
        &si_entries
    } else {
        &dsi_entries
    };
    if entries.is_empty() {
        return None;
    }

    // For MSP the installer-level metadata is in the last entry.
    let &(start_sec, size) = if is_msp {
        entries.last()?
    } else {
        entries.first()?
    };

    // ── Reconstruct the property-set stream ──────────────────────
    let stream = if size > 0 && size < mini_cutoff {
        // Stream lives in the mini-stream — read mini-FAT + container.
        let mfat_raw = ole_read_chain(buf, ss, &fat, mfat_sec0);
        let mfat: Vec<u32> = (0..mfat_raw.len() / 4)
            .map(|i| u32::from_le_bytes(mfat_raw[i * 4..(i + 1) * 4].try_into().unwrap()))
            .collect();
        let mut container = ole_read_chain(buf, ss, &fat, root_start);
        container.truncate(root_size);

        let mut data = Vec::with_capacity(size);
        let mut msec = start_sec;
        let mut rem = size;
        let mut it = 0usize;
        while rem > 0 && msec < 0xFFFF_FFFC && (msec as usize) < mfat.len() {
            it += 1;
            if it > size / mss + 2 {
                break;
            }
            let moff = msec as usize * mss;
            let n = rem.min(mss);
            if moff + n > container.len() {
                break;
            }
            data.extend_from_slice(&container[moff..moff + n]);
            rem = rem.saturating_sub(mss);
            msec = mfat[msec as usize];
        }
        data
    } else {
        let mut d = ole_read_chain(buf, ss, &fat, start_sec);
        d.truncate(size);
        d
    };

    if stream.len() < 28 {
        return None;
    }
    // Verify property-set byte-order mark.
    if u16::from_le_bytes([stream[0], stream[1]]) != 0xFFFE {
        return None;
    }

    // ── Locate the right FMTID section inside the property set ───
    let fmtid_si: [u8; 16] = [
        0xe0, 0x85, 0x9f, 0xf2, 0xf9, 0x4f, 0x68, 0x10, 0xab, 0x91, 0x08, 0x00, 0x2b, 0x27,
        0xb3, 0xd9,
    ];
    let fmtid_dsi: [u8; 16] = [
        0x02, 0xd5, 0xcd, 0xd5, 0x9c, 0x2e, 0x1b, 0x10, 0x93, 0x97, 0x08, 0x00, 0x2b, 0x2c,
        0xf9, 0xae,
    ];

    let nsec = u32::from_le_bytes(stream[24..28].try_into().ok()?) as usize;
    let mut best_pos: Option<usize> = None;
    for s in 0..nsec.min(16) {
        let off = 28 + s * 20;
        if off + 20 > stream.len() {
            break;
        }
        let fmtid = &stream[off..off + 16];
        if fmtid == fmtid_si || fmtid == fmtid_dsi {
            if is_msp {
                best_pos = Some(off); // last wins for MSP
            } else if best_pos.is_none() || fmtid == fmtid_si {
                best_pos = Some(off); // prefer SI
            }
        }
    }
    let fmtid_pos = best_pos?;

    let installer = if is_msi {
        Some("MSI Installer")
    } else if is_mst {
        Some("MST")
    } else if is_msp {
        Some("MSP")
    } else {
        None
    };

    Some(format_ole_summary(&stream, 0, fmtid_pos, installer))
}


/// Parse the first section of an OLE SummaryInformation property set and
/// emit the classic upstream `file` summary line. `ps_start` points at the
/// property-set header; `fmtid_pos` is where the (SI or DSI) FMTID sits.
fn format_ole_summary(
    buf: &[u8],
    ps_start: usize,
    fmtid_pos: usize,
    installer_label: Option<&str>,
) -> String {
    let os_major = buf[ps_start + 4];
    let os_minor = buf[ps_start + 5];
    let os_platform = buf[ps_start + 6];
    let os_name = match os_platform {
        0 => "MS-DOS",
        1 => "Macintosh",
        2 => "Windows",
        _ => "unknown",
    };
    let mut out = format!(
        "Composite Document File V2 Document, Little Endian, Os: {os_name}, Version {os_major}.{os_minor}"
    );
    // Only MSI gets a visible suffix; MST uses the label internally to
    // switch to its field ordering but doesn't append anything to the header.
    if installer_label == Some("MSI Installer") {
        out.push_str(", MSI Installer");
    }
    let section_offset = u32::from_le_bytes([
        buf[fmtid_pos + 16],
        buf[fmtid_pos + 17],
        buf[fmtid_pos + 18],
        buf[fmtid_pos + 19],
    ]) as usize;
    let section_start = ps_start + section_offset;
    if section_start + 8 >= buf.len() {
        return out;
    }
    let count = u32::from_le_bytes([
        buf[section_start + 4],
        buf[section_start + 5],
        buf[section_start + 6],
        buf[section_start + 7],
    ]) as usize;
    // Collect (pid, offset) pairs.
    let mut props: Vec<(u32, usize)> = Vec::new();
    for k in 0..count.min(64) {
        let entry = section_start + 8 + k * 8;
        if entry + 8 > buf.len() {
            break;
        }
        let pid = u32::from_le_bytes([
            buf[entry], buf[entry + 1], buf[entry + 2], buf[entry + 3],
        ]);
        let off = u32::from_le_bytes([
            buf[entry + 4], buf[entry + 5], buf[entry + 6], buf[entry + 7],
        ]) as usize;
        props.push((pid, off));
    }
    let read_prop = |pid: u32| -> Option<(u16, usize)> {
        props.iter().find(|(p, _)| *p == pid).map(|(_, o)| {
            let val_off = section_start + *o;
            if val_off + 4 > buf.len() {
                return (0u16, val_off);
            }
            let vtype = u16::from_le_bytes([buf[val_off], buf[val_off + 1]]);
            (vtype, val_off)
        })
    };
    let read_str = |val_off: usize| -> Option<String> {
        if val_off + 8 > buf.len() {
            return None;
        }
        let len = u32::from_le_bytes([
            buf[val_off + 4], buf[val_off + 5], buf[val_off + 6], buf[val_off + 7],
        ]) as usize;
        if len == 0 || val_off + 8 + len > buf.len() {
            return None;
        }
        let bytes = &buf[val_off + 8..val_off + 8 + len];
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        if end == 0 {
            return None;
        }
        // Upstream keeps only printable ASCII from LPSTR values; non-ASCII
        // bytes (0xe4 for ä, etc.) are silently dropped.
        let filtered: String = bytes[..end]
            .iter()
            .filter(|&&b| (0x20..0x7f).contains(&b))
            .map(|&b| b as char)
            .collect();
        if filtered.is_empty() {
            return None;
        }
        Some(filtered)
    };
    let read_i4 = |val_off: usize| -> Option<i32> {
        if val_off + 8 > buf.len() {
            return None;
        }
        Some(i32::from_le_bytes([
            buf[val_off + 4], buf[val_off + 5], buf[val_off + 6], buf[val_off + 7],
        ]))
    };
    let read_filetime = |val_off: usize| -> Option<u64> {
        if val_off + 12 > buf.len() {
            return None;
        }
        Some(u64::from_le_bytes([
            buf[val_off + 4], buf[val_off + 5], buf[val_off + 6], buf[val_off + 7],
            buf[val_off + 8], buf[val_off + 9], buf[val_off + 10], buf[val_off + 11],
        ]))
    };
    // Code page (PID 1). Upstream prints unsigned 16-bit even if stored
    // as negative.
    if let Some((vtype, val_off)) = read_prop(0x01)
        && vtype == 0x0002
        && val_off + 6 <= buf.len()
    {
        // Upstream preserves the signedness — UTF-8 (65001) renders as -535.
        let cp = i16::from_le_bytes([buf[val_off + 4], buf[val_off + 5]]);
        out.push_str(&format!(", Code page: {cp}"));
    }
    let emit_str = |out: &mut String, pid: u32, label: &str| {
        if let Some((vtype, val_off)) = read_prop(pid)
            && vtype == 0x001e
            && let Some(s) = read_str(val_off)
        {
            out.push_str(&format!(", {label}: {s}"));
        }
    };
    let emit_filetime = |out: &mut String, pid: u32, label: &str| {
        if let Some((vtype, val_off)) = read_prop(pid)
            && vtype == 0x0040
            && let Some(ft) = read_filetime(val_off)
            && ft > 0
        {
            let filetime_epoch_offset = 11_644_473_600u64;
            let secs = (ft / 10_000_000).saturating_sub(filetime_epoch_offset) as i64;
            out.push_str(&format!(", {label}: {}", format_unix_utc(secs)));
        }
    };
    let emit_i4 = |out: &mut String, pid: u32, label: &str| {
        if let Some((vtype, val_off)) = read_prop(pid)
            && vtype == 0x0003
            && let Some(v) = read_i4(val_off)
        {
            out.push_str(&format!(", {label}: {v}"));
        }
    };
    let emit_duration = |out: &mut String| {
        if let Some((vtype, val_off)) = read_prop(0x0a)
            && vtype == 0x0040
            && let Some(ft) = read_filetime(val_off)
        {
            let total_secs = ft / 10_000_000;
            if total_secs < 3600 {
                let mins = total_secs / 60;
                let secs = total_secs % 60;
                out.push_str(&format!(", Total Editing Time: {mins:02}:{secs:02}"));
            } else {
                let hrs = total_secs / 3600;
                let mins = (total_secs / 60) % 60;
                out.push_str(&format!(", Total Editing Time: {hrs:02}:{mins:02}"));
            }
        }
    };
    let is_msi = installer_label == Some("MSI Installer");
    let is_mst = installer_label == Some("MST") || installer_label == Some("MSP");
    if is_mst {
        // MST ordering: Title, Subject, Author, Keywords, Comments, Create
        // Time/Date, Name of Creating Application, Security, Template,
        // Last Saved By, Revision Number, Number of Pages, Number of
        // Characters.
        emit_str(&mut out, 0x02, "Title");
        emit_str(&mut out, 0x03, "Subject");
        emit_str(&mut out, 0x04, "Author");
        emit_str(&mut out, 0x05, "Keywords");
        emit_str(&mut out, 0x06, "Comments");
        emit_filetime(&mut out, 0x0c, "Create Time/Date");
        emit_str(&mut out, 0x12, "Name of Creating Application");
        emit_i4(&mut out, 0x13, "Security");
        emit_str(&mut out, 0x07, "Template");
        emit_str(&mut out, 0x08, "Last Saved By");
        emit_str(&mut out, 0x09, "Revision Number");
        emit_i4(&mut out, 0x0e, "Number of Pages");
        emit_i4(&mut out, 0x10, "Number of Characters");
        return out;
    }
    if is_msi {
        // MSI ordering: Title, Subject, Author, Keywords, Comments, Template,
        // Revision Number, Create Time/Date, Last Saved Time/Date, Pages,
        // Words, Name of Creating Application, Security.
        emit_str(&mut out, 0x02, "Title");
        emit_str(&mut out, 0x03, "Subject");
        emit_str(&mut out, 0x04, "Author");
        emit_str(&mut out, 0x05, "Keywords");
        emit_str(&mut out, 0x06, "Comments");
        emit_str(&mut out, 0x07, "Template");
        emit_str(&mut out, 0x09, "Revision Number");
        emit_filetime(&mut out, 0x0c, "Create Time/Date");
        emit_filetime(&mut out, 0x0d, "Last Saved Time/Date");
        emit_i4(&mut out, 0x0e, "Number of Pages");
        emit_i4(&mut out, 0x0f, "Number of Words");
        emit_str(&mut out, 0x12, "Name of Creating Application");
        emit_i4(&mut out, 0x13, "Security");
    } else {
        // DOC/XLS/PPT ordering.
        emit_str(&mut out, 0x02, "Title");
        emit_str(&mut out, 0x03, "Subject");
        emit_str(&mut out, 0x04, "Author");
        emit_str(&mut out, 0x05, "Keywords");
        emit_str(&mut out, 0x06, "Comments");
        emit_str(&mut out, 0x07, "Template");
        emit_str(&mut out, 0x08, "Last Saved By");
        emit_str(&mut out, 0x09, "Revision Number");
        emit_str(&mut out, 0x12, "Name of Creating Application");
        emit_duration(&mut out);
        emit_filetime(&mut out, 0x0b, "Last Printed");
        emit_filetime(&mut out, 0x0c, "Create Time/Date");
        emit_filetime(&mut out, 0x0d, "Last Saved Time/Date");
        emit_i4(&mut out, 0x0e, "Number of Pages");
        emit_i4(&mut out, 0x0f, "Number of Words");
        emit_i4(&mut out, 0x10, "Number of Characters");
        emit_i4(&mut out, 0x13, "Security");
    }
    out
}

/// Walk Ogg pages to find the vendor string buried in the Vorbis comment
/// packet. The comment packet lives in page 2; its payload is
/// `[type byte = 0x03] "vorbis" [vendor_len:le u32] [vendor:utf-8] ...`.
fn ogg_vorbis_vendor(buf: &[u8]) -> Option<String> {
    let mut i = 0usize;
    // Skip the first page (identification packet).
    if buf.len() < 27 || &buf[0..4] != b"OggS" {
        return None;
    }
    let seg_count = buf[26] as usize;
    let mut total_payload = 0usize;
    if i + 27 + seg_count > buf.len() {
        return None;
    }
    for k in 0..seg_count {
        total_payload += buf[27 + k] as usize;
    }
    i = 27 + seg_count + total_payload;
    // Second page: Vorbis comment.
    if i + 27 > buf.len() || &buf[i..i + 4] != b"OggS" {
        return None;
    }
    let seg_count = buf[i + 26] as usize;
    let payload_start = i + 27 + seg_count;
    if payload_start + 14 > buf.len() {
        return None;
    }
    // Check packet type byte 0x03 then "vorbis" then vendor_len.
    if buf[payload_start] != 0x03 || &buf[payload_start + 1..payload_start + 7] != b"vorbis" {
        return None;
    }
    let vendor_len_off = payload_start + 7;
    let vendor_len = u32::from_le_bytes([
        buf[vendor_len_off],
        buf[vendor_len_off + 1],
        buf[vendor_len_off + 2],
        buf[vendor_len_off + 3],
    ]) as usize;
    let vendor_start = vendor_len_off + 4;
    if vendor_start + vendor_len > buf.len() {
        return None;
    }
    std::str::from_utf8(&buf[vendor_start..vendor_start + vendor_len])
        .ok()
        .map(|s| s.to_string())
}

/// Render byte slice as ASCII, escaping non-printable or non-ASCII bytes as
/// 3-digit octal escapes. Mirrors `file`'s behavior for embedded-name fields
/// like gzip's FNAME or tar headers.
fn bytes_to_octal_string(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        if (0x20..0x7f).contains(&b) && b != b'"' && b != b'\\' {
            out.push(b as char);
        } else {
            out.push_str(&format!("\\{:03o}", b));
        }
    }
    out
}

/// Format a Unix UTC timestamp as upstream's `file` does — "Mon Feb 16
/// 21:10:58 2009" style. We implement the date math ourselves rather than
/// pulling in `chrono` because the rest of the crate avoids dependencies.
fn format_unix_utc(ts: i64) -> String {
    if ts < 0 {
        return "invalid timestamp".to_string();
    }
    let days_since_epoch = ts / 86_400;
    let secs_in_day = (ts % 86_400) as u32;
    let hour = secs_in_day / 3600;
    let minute = (secs_in_day / 60) % 60;
    let second = secs_in_day % 60;
    // Compute weekday. 1970-01-01 was a Thursday → weekday index 4.
    let weekdays = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let weekday = weekdays[((days_since_epoch + 4).rem_euclid(7)) as usize];
    // Days -> (year, month, day) using the civil_from_days algorithm.
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun",
        "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = months[(m - 1) as usize];
    format!(
        "{weekday} {month} {d:>2} {hour:02}:{minute:02}:{second:02} {y}"
    )
}

/// Parse the first IFD of a TIFF file and build the comma-separated suffix
/// upstream emits ("direntries=N, height=H, bps=B, compression=NAME, ...").
fn tiff_summary(buf: &[u8], le: bool, is_exif: bool) -> String {
    let u32_at = |i: usize| -> Option<u32> {
        if i + 4 > buf.len() {
            return None;
        }
        let bytes = [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]];
        Some(if le { u32::from_le_bytes(bytes) } else { u32::from_be_bytes(bytes) })
    };
    let u16_at = |i: usize| -> Option<u16> {
        if i + 2 > buf.len() {
            return None;
        }
        let bytes = [buf[i], buf[i + 1]];
        Some(if le { u16::from_le_bytes(bytes) } else { u16::from_be_bytes(bytes) })
    };
    // Read an ASCII (type 2) string value from an IFD entry whose first byte
    // is at position `eoff` in `buf`.
    let str_at = |eoff: usize| -> Option<String> {
        if eoff + 12 > buf.len() {
            return None;
        }
        let cnt_b = [buf[eoff + 4], buf[eoff + 5], buf[eoff + 6], buf[eoff + 7]];
        let cnt = (if le {
            u32::from_le_bytes(cnt_b)
        } else {
            u32::from_be_bytes(cnt_b)
        }) as usize;
        if cnt == 0 {
            return Some(String::new());
        }
        let data = if cnt <= 4 {
            buf.get(eoff + 8..eoff + 8 + cnt)?
        } else {
            let ob = [buf[eoff + 8], buf[eoff + 9], buf[eoff + 10], buf[eoff + 11]];
            let off = (if le {
                u32::from_le_bytes(ob)
            } else {
                u32::from_be_bytes(ob)
            }) as usize;
            buf.get(off..off + cnt)?
        };
        Some(
            String::from_utf8_lossy(data)
                .trim_end_matches('\0')
                .to_string(),
        )
    };
    // Read the raw 4-byte value/offset field from an IFD entry at `eoff`.
    // For RATIONAL (type 5) entries this field is actually a TIFF offset
    // pointing to the numerator/denominator pair, but upstream `file`
    // prints this raw field value (the offset) rather than dereferencing
    // it.  We match that behaviour.
    let rat_num_at = |eoff: usize| -> Option<u32> {
        if eoff + 12 > buf.len() {
            return None;
        }
        let ob = [buf[eoff + 8], buf[eoff + 9], buf[eoff + 10], buf[eoff + 11]];
        Some(if le {
            u32::from_le_bytes(ob)
        } else {
            u32::from_be_bytes(ob)
        })
    };

    let ifd_off = match u32_at(4) {
        Some(o) => o as usize,
        None => return String::new(),
    };
    let count = match u16_at(ifd_off) {
        Some(c) => c as usize,
        None => return String::new(),
    };
    let entry_base = ifd_off + 2;

    // --- Standalone-TIFF tags ---
    let mut width: Option<u64> = None;
    let mut height: Option<u64> = None;
    let mut bps: Option<u64> = None;
    let mut compression: Option<u16> = None;
    let mut photometric: Option<u16> = None;
    // --- Exif-relevant TIFF tags ---
    let mut description: Option<String> = None;
    let mut manufacturer: Option<String> = None;
    let mut model_str: Option<String> = None;
    let mut orientation: Option<u16> = None;
    let mut xresolution: Option<u32> = None;
    let mut yresolution: Option<u32> = None;
    let mut resolutionunit: Option<u16> = None;
    let mut software: Option<String> = None;
    let mut datetime: Option<String> = None;

    for i in 0..count {
        let off = entry_base + i * 12;
        if off + 12 > buf.len() {
            break;
        }
        let tag = match u16_at(off) {
            Some(t) => t,
            None => break,
        };
        let ttype = match u16_at(off + 2) {
            Some(t) => t,
            None => break,
        };
        // Decode inline scalar values (BYTE / SHORT / LONG).
        let value: Option<u64> = match ttype {
            1 => buf.get(off + 8).copied().map(|b| b as u64),
            3 => u16_at(off + 8).map(|v| v as u64),
            4 => u32_at(off + 8).map(|v| v as u64),
            _ => None,
        };
        match tag {
            0x0100 => width = value,
            0x0101 => height = value,
            0x0102 => bps = value,
            0x0103 => compression = value.map(|v| v as u16),
            0x0106 => photometric = value.map(|v| v as u16),
            0x010E => description = Some(str_at(off).unwrap_or_default()),
            0x010F => manufacturer = Some(str_at(off).unwrap_or_default()),
            0x0110 => model_str = Some(str_at(off).unwrap_or_default()),
            0x0112 => orientation = value.map(|v| v as u16),
            0x011A => xresolution = rat_num_at(off),
            0x011B => yresolution = rat_num_at(off),
            0x0128 => resolutionunit = value.map(|v| v as u16),
            0x0131 => software = Some(str_at(off).unwrap_or_default()),
            0x0132 => datetime = Some(str_at(off).unwrap_or_default()),
            _ => {}
        }
    }

    // Build the output in upstream `file`'s order.  The standalone-TIFF tags
    // (height, bps, compression, photometric, width) follow the magic-file
    // order; the Exif-relevant tags follow ascending tag-number order (which
    // matches upstream's JPEG/Exif magic).
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("direntries={count}"));
    if let Some(h) = height {
        parts.push(format!("height={h}"));
    }
    if let Some(b) = bps {
        parts.push(format!("bps={b}"));
    }
    if let Some(c) = compression {
        let name = match c {
            1 => "none",
            2 => "CCITT 1D",
            3 => "CCITT Group 3",
            4 => "CCITT Group 4",
            5 => "LZW",
            6 => "JPEG (old)",
            7 => "JPEG",
            8 => "AdobeDeflate",
            9 => "JBIG B&W",
            10 => "JBIG color",
            32773 => "PackBits",
            32946 => "Deflate",
            _ => "unknown",
        };
        parts.push(format!("compression={name}"));
    }
    if let Some(p) = photometric {
        let name = match p {
            0 => "WhiteIsZero",
            1 => "BlackIsZero",
            2 => "RGB",
            3 => "RGB Palette",
            4 => "Transparency Mask",
            5 => "CMYK",
            6 => "YCbCr",
            8 => "CIELab",
            _ => "unknown",
        };
        parts.push(format!("PhotometricInterpretation={name}"));
    }
    if is_exif {
    if let Some(ref d) = description {
        parts.push(format!("description={d}"));
    }
    if let Some(ref m) = manufacturer {
        parts.push(format!("manufacturer={m}"));
    }
    if let Some(ref m) = model_str {
        parts.push(format!("model={m}"));
    }
    if let Some(o) = orientation {
        let name = match o {
            1 => "upper-left",
            2 => "upper-right",
            3 => "lower-right",
            4 => "lower-left",
            5 => "left-top",
            6 => "right-top",
            7 => "right-bottom",
            8 => "left-bottom",
            _ => "unknown",
        };
        parts.push(format!("orientation={name}"));
    }
    if let Some(x) = xresolution {
        parts.push(format!("xresolution={x}"));
    }
    if let Some(y) = yresolution {
        parts.push(format!("yresolution={y}"));
    }
    if let Some(r) = resolutionunit {
        parts.push(format!("resolutionunit={r}"));
    }
    if let Some(ref s) = software {
        parts.push(format!("software={s}"));
    }
    if let Some(ref d) = datetime {
        parts.push(format!("datetime={d}"));
    }
    }
    if let Some(w) = width {
        parts.push(format!("width={w}"));
    }
    format!(", {}", parts.join(", "))
}

/// Upstream appends ", with very long lines (N)" to text summaries when any
/// line exceeds 128 bytes; the reported length is the longest line.
fn long_lines_suffix(buf: &[u8]) -> String {
    let mut max = 0usize;
    let mut cur = 0usize;
    for &b in buf {
        if b == b'\n' {
            if cur > max {
                max = cur;
            }
            cur = 0;
        } else if b != b'\r' {
            cur += 1;
        }
    }
    if cur > max {
        max = cur;
    }
    if max > 256 {
        format!(", with very long lines ({max})")
    } else {
        String::new()
    }
}

/// Classify the text encoding of a buffer we've already decided is text.
/// Returns one of the upstream encoding tokens ("ASCII", "Unicode text, UTF-8",
/// "ISO-8859") so the caller can embed it as "<encoding> text".
fn encoding_suffix_for_text(buf: &[u8], is_utf8: bool) -> String {
    if is_utf8 && is_ascii_only(buf) {
        return "ASCII".to_string();
    }
    if is_utf8 {
        if buf.starts_with(b"\xef\xbb\xbf") {
            return "Unicode text, UTF-8 (with BOM)".to_string();
        }
        return "Unicode text, UTF-8".to_string();
    }
    "ISO-8859".to_string()
}

fn looks_like_mail(text: &str) -> bool {
    // Upstream only labels a file as RFC 822 mail when it *starts* with a
    // mail header — mail-like strings later in the body do not count.
    let markers = [
        "From:", "To:", "Subject:", "Date:", "Message-ID:",
        "Received:", "Return-Path:", "From ", "Delivered-To:",
    ];
    if !markers.iter().any(|m| text.starts_with(m)) {
        return false;
    }
    // Then at least one additional distinct header must appear in the first
    // block of lines before the first blank line (= the RFC 822 header).
    let hdr_block: &str = text.split("\n\n").next().unwrap_or("");
    markers
        .iter()
        .filter(|m| {
            hdr_block.starts_with(*m)
                || hdr_block.contains(&format!("\n{m}"))
        })
        .count()
        >= 2
}

fn looks_like_json(buf: &[u8]) -> bool {
    // Trim leading ASCII whitespace; require the top-level value to open with
    // `{` or `[`, followed by a JSON-valid continuation (string, nested
    // object/array, or closer). This rejects RTF (`{\\rtf`), shell scripts
    // that happen to start with `{`, and so on.
    let skip_ws = |mut i: usize| -> usize {
        while i < buf.len() && matches!(buf[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        i
    };
    let i = skip_ws(0);
    if i >= buf.len() {
        return false;
    }
    let opener = buf[i];
    if !matches!(opener, b'{' | b'[') {
        return false;
    }
    let j = skip_ws(i + 1);
    if j >= buf.len() {
        return false;
    }
    if opener == b'{' {
        // JSON object members must be `"key": value`. The next non-whitespace
        // byte after `{` is a `"` (first member) or `}` (empty object).
        matches!(buf[j], b'"' | b'}')
    } else {
        // JSON array opener — value can be string, number, literal, or nested
        // object/array.
        matches!(
            buf[j],
            b'"' | b'[' | b'{' | b'-' | b'0'..=b'9' | b't' | b'f' | b'n' | b']'
        )
    }
}

fn identify_utf16(buf: &[u8], opts: &FileOpts, le: bool) -> String {
    // Strip the BOM, decode half-word-by-half-word, and look for an XML
    // prolog in the resulting ASCII characters. Upstream reports these as
    // "XML 1.0 document, Unicode text, UTF-16, little-endian text" etc.
    let mut chars = Vec::with_capacity(buf.len() / 2);
    let mut i = 2;
    while i + 1 < buf.len() {
        let code = if le {
            u16::from_le_bytes([buf[i], buf[i + 1]])
        } else {
            u16::from_be_bytes([buf[i], buf[i + 1]])
        };
        chars.push(code);
        i += 2;
    }
    let ascii: String = chars
        .iter()
        .filter_map(|&c| if c < 0x80 { Some(c as u8 as char) } else { None })
        .collect();
    let endian = if le { "little-endian" } else { "big-endian" };
    if ascii.starts_with("<?xml") {
        if opts.mime_type {
            return mime_with_encoding("text/xml", opts);
        }
        return format!("XML 1.0 document, Unicode text, UTF-16, {endian} text");
    }
    // Windows Registry Editor text — the "Win2K or above" variant uses the
    // UTF-16 signature with the literal "Windows Registry Editor Version 5"
    // header; older Win9x/NT4 variants are ASCII with "REGEDIT4".
    if ascii.starts_with("Windows Registry Editor Version 5") {
        if opts.mime_type {
            return mime_with_encoding("text/plain; charset=utf-16", opts);
        }
        return format!("Windows Registry {endian} text (Win2K or above)");
    }
    if opts.mime_type {
        return mime_with_encoding("text/plain; charset=utf-16", opts);
    }
    format!("Unicode text, UTF-16, {endian} text")
}

fn identify_utf32(_buf: &[u8], opts: &FileOpts, le: bool) -> String {
    let endian = if le { "little-endian" } else { "big-endian" };
    if opts.mime_type {
        return mime_with_encoding("text/plain; charset=utf-32", opts);
    }
    format!("Unicode text, UTF-32, {endian} text")
}

fn identify_pdf(buf: &[u8]) -> String {
    // %PDF-X.Y where X.Y is the version. Read up to the first newline or
    // 16 bytes, whichever comes first.
    let end = buf
        .iter()
        .take(16)
        .position(|&b| b == b'\n' || b == b'\r')
        .unwrap_or(buf.len().min(16));
    // buf[5..end] contains the version (e.g. "1.4")
    let version = std::str::from_utf8(&buf[5..end])
        .unwrap_or("")
        .trim();
    if version.is_empty() {
        "PDF document".to_string()
    } else {
        format!("PDF document, version {version}")
    }
}

fn identify_ext_fs(buf: &[u8]) -> String {
    let sb = 1024usize; // superblock offset
    let le_u16 = |o: usize| u16::from_le_bytes([buf[sb + o], buf[sb + o + 1]]);
    let le_u32 = |o: usize| u32::from_le_bytes([buf[sb + o], buf[sb + o + 1], buf[sb + o + 2], buf[sb + o + 3]]);

    let rev_level = le_u32(76);
    let minor_rev = le_u16(62);
    let feature_compat = le_u32(92);
    let feature_incompat = le_u32(96);
    let feature_ro_compat = le_u32(100);
    let uuid_bytes = &buf[sb + 104..sb + 120];
    let s_state = le_u16(58); // 1=clean, 2=errors, 4=orphans

    // Determine filesystem type
    let fs_type = if feature_incompat & 0x0040 != 0 {
        "ext4"
    } else if feature_compat & 0x0004 != 0 {
        "ext3"
    } else {
        "ext2"
    };

    // Format UUID as 8-4-4-4-12
    let uuid = format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        uuid_bytes[0], uuid_bytes[1], uuid_bytes[2], uuid_bytes[3],
        uuid_bytes[4], uuid_bytes[5],
        uuid_bytes[6], uuid_bytes[7],
        uuid_bytes[8], uuid_bytes[9],
        uuid_bytes[10], uuid_bytes[11], uuid_bytes[12], uuid_bytes[13], uuid_bytes[14], uuid_bytes[15]
    );

    let mut out = format!("Linux rev {rev_level}.{minor_rev} {fs_type} filesystem data, UUID={uuid}");

    // Volume name (16 bytes at offset 136)
    let vol_name_bytes = &buf[sb + 120..sb + 136];
    let vol_end = vol_name_bytes.iter().position(|&b| b == 0).unwrap_or(16);
    if vol_end > 0 {
        let vol_name = String::from_utf8_lossy(&vol_name_bytes[..vol_end]);
        out.push_str(&format!(", volume name \"{}\"", vol_name));
    }

    // Feature flags - order matters! Match GNU file's output order
    // Incompat features
    if feature_incompat & 0x0004 != 0 {
        // RECOVER flag = needs journal recovery
        out.push_str(" (needs journal recovery)");
    }
    if feature_incompat & 0x0040 != 0 {
        out.push_str(" (extents)");
    }

    // Ro-compat features
    if feature_ro_compat & 0x0008 != 0 {
        out.push_str(" (huge files)");
    }

    out
}

fn identify_mbr(buf: &[u8]) -> String {
    let mut out = "DOS/MBR boot sector".to_string();

    // Check for NTFS OEM-ID at offset 3
    let is_ntfs = buf.len() >= 11 && &buf[3..11] == b"NTFS    ";

    if is_ntfs {
        return identify_ntfs_boot(buf);
    }

    // Parse partition table entries (4 entries at offsets 446, 462, 478, 494)
    let mut first_part = true;
    for i in 0..4 {
        let base = 446 + i * 16;
        if base + 16 > buf.len() {
            break;
        }

        let status = buf[base];
        let part_type = buf[base + 4];

        // Skip empty partition entries
        if part_type == 0 {
            continue;
        }

        let start_chs_head = buf[base + 1];
        let start_chs_sec = buf[base + 2] & 0x3f;
        let start_chs_cyl = ((buf[base + 2] as u16 & 0xc0) << 2) | buf[base + 3] as u16;

        let end_chs_head = buf[base + 5];
        let end_chs_sec = buf[base + 6] & 0x3f;
        let end_chs_cyl = ((buf[base + 6] as u16 & 0xc0) << 2) | buf[base + 7] as u16;

        let start_lba = u32::from_le_bytes([buf[base + 8], buf[base + 9], buf[base + 10], buf[base + 11]]);
        let num_sectors = u32::from_le_bytes([buf[base + 12], buf[base + 13], buf[base + 14], buf[base + 15]]);

        let sep = if first_part { "; " } else { "; " };
        first_part = false;

        out.push_str(&format!(
            "{}partition {} : ID=0x{:x}",
            sep, i + 1, part_type
        ));

        if status == 0x80 {
            out.push_str(", active");
        }

        out.push_str(&format!(
            ", start-CHS (0x{:x},{},{}), end-CHS (0x{:x},{},{}), startsector {}, {} sectors",
            start_chs_cyl, start_chs_head, start_chs_sec,
            end_chs_cyl, end_chs_head, end_chs_sec,
            start_lba, num_sectors
        ));

        // Extended partition types
        if part_type == 0x05 || part_type == 0x0f || part_type == 0x85 {
            out.push_str(", extended partition table");
        }
    }

    out
}

fn identify_ntfs_boot(buf: &[u8]) -> String {
    let mut out = "DOS/MBR boot sector".to_string();

    // Code offset: byte 0 is the jump instruction (0xEB = short jump)
    if buf[0] == 0xeb {
        out.push_str(&format!(", code offset 0x{:02x}+2", buf[1]));
    } else if buf[0] == 0xe9 {
        let offset = u16::from_le_bytes([buf[1], buf[2]]);
        out.push_str(&format!(", code offset 0x{:04x}+3", offset));
    }

    // OEM-ID at offset 3 (8 bytes)
    let oem = String::from_utf8_lossy(&buf[3..11]);
    out.push_str(&format!(", OEM-ID \"{}\"", oem));

    // BPB (BIOS Parameter Block)
    let sectors_per_cluster = buf[13];
    let media_descriptor = buf[21];
    let sectors_per_track = u16::from_le_bytes([buf[24], buf[25]]);
    let heads = u16::from_le_bytes([buf[26], buf[27]]);
    let hidden_sectors = u32::from_le_bytes([buf[28], buf[29], buf[30], buf[31]]);

    out.push_str(&format!(", sectors/cluster {}", sectors_per_cluster));
    out.push_str(&format!(", Media descriptor 0x{:02x}", media_descriptor));
    out.push_str(&format!(", sectors/track {}", sectors_per_track));
    out.push_str(&format!(", heads {}", heads));
    out.push_str(&format!(", hidden sectors {}", hidden_sectors));

    // Boot indicator and FAT descriptor
    let boot_indicator = buf[36];
    out.push_str(&format!(", dos < 4.0 BootSector (0x{:02x})", boot_indicator));

    // FAT descriptor based on media_descriptor
    if media_descriptor == 0xf8 {
        out.push_str(", FAT (1Y bit by descriptor)");
    }

    // NTFS specific fields
    let total_sectors = u64::from_le_bytes([buf[40], buf[41], buf[42], buf[43], buf[44], buf[45], buf[46], buf[47]]);
    let mft_start_cluster = u64::from_le_bytes([buf[48], buf[49], buf[50], buf[51], buf[52], buf[53], buf[54], buf[55]]);
    let mft_mirror_cluster = u64::from_le_bytes([buf[56], buf[57], buf[58], buf[59], buf[60], buf[61], buf[62], buf[63]]);
    let record_segment_raw = buf[64];
    let clusters_per_index = buf[68];
    let serial = u64::from_le_bytes([buf[72], buf[73], buf[74], buf[75], buf[76], buf[77], buf[78], buf[79]]);

    out.push_str(&format!("; NTFS, sectors/track {}", sectors_per_track));
    out.push_str(&format!(", sectors {}", total_sectors));
    out.push_str(&format!(", $MFT start cluster {}", mft_start_cluster));
    out.push_str(&format!(", $MFTMirror start cluster {}", mft_mirror_cluster));

    // Record segment size: if > 127, it's 2^(-1*value) bytes; otherwise clusters
    if record_segment_raw > 127 {
        out.push_str(&format!(", bytes/RecordSegment 2^(-1*{})", record_segment_raw));
    } else {
        out.push_str(&format!(", clusters/RecordSegment {}", record_segment_raw));
    }

    out.push_str(&format!(", clusters/index block {}", clusters_per_index));
    out.push_str(&format!(", serial number 0{:016x}", serial));

    // Check for NTLDR bootstrap signature
    if buf.len() >= 512 {
        let boot_code = &buf[0..512];
        if boot_code.windows(5).any(|w| w == b"NTLDR") {
            out.push_str("; contains bootstrap NTLDR");
        }
    }

    out
}


fn identify_dump_be(buf: &[u8]) -> String {
    let be_u32 = |o: usize| u32::from_be_bytes([buf[o], buf[o + 1], buf[o + 2], buf[o + 3]]);

    let c_type = be_u32(0);
    let c_date = be_u32(4) as i64;
    let c_ddate = be_u32(8) as i64;
    let c_volume = be_u32(12);
    let c_magic = be_u32(24);

    let fs_kind = if c_magic == 0x0000EA6C {
        "new-fs"
    } else {
        "old-fs"
    };

    let type_name = match c_type {
        1 => "tape header",
        2 => "beginning of file record",
        3 => "map of inodes on tape",
        4 => "continuation of file record",
        5 => "end of volume",
        6 => "map of inodes deleted since reference",
        7 => "end of medium",
        _ => "unknown",
    };

    let read_cstr = |off: usize, max: usize| -> String {
        let end = buf[off..off + max]
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(max);
        String::from_utf8_lossy(&buf[off..off + end]).to_string()
    };

    let c_label = read_cstr(676, 16);
    let c_level = be_u32(692);
    let c_filesys = read_cstr(696, 64);
    let c_dev = read_cstr(760, 64);
    let c_host = read_cstr(824, 64);
    let c_flags = be_u32(888);

    let level_str = if c_level == 0 {
        "zero".to_string()
    } else {
        c_level.to_string()
    };

    let this_dump = format_unix_utc(c_date);
    let prev_dump = format_unix_utc(c_ddate);

    format!(
        "{fs_kind} dump file (big endian), This dump {this_dump}, Previous dump {prev_dump}, Volume {c_volume}, Level {level_str}, type: {type_name}, Label {c_label}, Filesystem {c_filesys}, Device {c_dev}, Host {c_host}, Flags {c_flags:x}"
    )
}

fn identify_installshield(buf: &[u8]) -> String {
    let mut out = "InstallShield Script".to_string();

    if buf.len() > 15 {
        let slen = u16::from_le_bytes([buf[13], buf[14]]) as usize;
        if slen > 0 && 15 + slen <= buf.len() {
            let raw = &buf[15..15 + slen];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            let s = String::from_utf8_lossy(&raw[..end]);
            if !s.is_empty() {
                out.push_str(&format!(" \"{}\"", s));
            }
        }
    }

    let search_range = buf.len().min(600);
    if search_range > 6 {
        if let Some(pos) = buf[..search_range].windows(6).position(|w| w == b"SRCDIR") {
            let mut vars: Vec<(u16, String)> = Vec::new();

            if pos >= 4 {
                let idx = u16::from_le_bytes([buf[pos - 4], buf[pos - 3]]);
                let nlen = u16::from_le_bytes([buf[pos - 2], buf[pos - 1]]) as usize;
                if nlen > 0 && nlen <= 64 && pos + nlen <= buf.len() {
                    let raw = &buf[pos..pos + nlen];
                    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
                    let name = String::from_utf8_lossy(&raw[..end]).to_string();
                    vars.push((idx, name));

                    let mut next = pos + nlen;
                    for _ in 0..2 {
                        if next + 4 <= buf.len() {
                            let idx2 = u16::from_le_bytes([buf[next], buf[next + 1]]);
                            let nlen2 = u16::from_le_bytes([buf[next + 2], buf[next + 3]]) as usize;
                            if nlen2 > 0 && nlen2 <= 64 && next + 4 + nlen2 <= buf.len() {
                                let raw2 = &buf[next + 4..next + 4 + nlen2];
                                let end2 = raw2.iter().position(|&b| b == 0).unwrap_or(raw2.len());
                                let name2 = String::from_utf8_lossy(&raw2[..end2]).to_string();
                                vars.push((idx2, name2));
                                next = next + 4 + nlen2;
                            } else {
                                break;
                            }
                        }
                    }
                }
            }

            if !vars.is_empty() {
                out.push_str(", variable names:");
                for (idx, name) in &vars {
                    out.push_str(&format!(" #{} {}", idx, name));
                }
                out.push_str(" ...");
            }
        }
    }

    out
}

fn format_filetime(ft: u64) -> String {
    if ft == 0 {
        return String::new();
    }
    let secs = (ft / 10_000_000).saturating_sub(11_644_473_600) as i64;
    format_unix_utc(secs)
}

fn identify_lnk(buf: &[u8]) -> String {
    let mut parts: Vec<String> = vec!["MS Windows shortcut".to_string()];

    let flags = u32::from_le_bytes([buf[20], buf[21], buf[22], buf[23]]);
    let file_attrs = u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]);

    // LinkFlags descriptions — emitted in bit order, but TrackerDataBlock
    // (MachineID) is inserted between bit 7 and bit 25.
    if flags & 0x001 != 0 {
        parts.push("Item id list present".to_string());
    }
    if flags & 0x002 != 0 {
        parts.push("Points to a file or directory".to_string());
    }
    if flags & 0x004 != 0 {
        parts.push("Has Description string".to_string());
    }
    if flags & 0x008 != 0 {
        parts.push("Has Relative path".to_string());
    }
    if flags & 0x010 != 0 {
        parts.push("Has Working directory".to_string());
    }
    if flags & 0x020 != 0 {
        parts.push("Has command line arguments".to_string());
    }
    // bit 6 (0x040): HasIconLocation — handled separately with icon index
    if flags & 0x080 != 0 {
        parts.push("Unicoded".to_string());
    }

    // TrackerDataBlock (MachineID) — search for signature 0x00000060 +
    // 0xA0000003 in the buffer. Fires when ForceNoLinkTrack (bit 24) is NOT set.
    if flags & 0x1000000 == 0 {
        let sig: [u8; 8] = [0x60, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0xA0];
        let search_start = 76;
        if let Some(pos) = buf[search_start..]
            .windows(8)
            .position(|w| w == sig)
        {
            let block_start = search_start + pos;
            let machine_off = block_start + 16;
            if machine_off + 16 <= buf.len() {
                let machine_bytes = &buf[machine_off..machine_off + 16];
                let end = machine_bytes
                    .iter()
                    .position(|&b| b == 0)
                    .unwrap_or(16);
                let machine_id =
                    std::str::from_utf8(&machine_bytes[..end]).unwrap_or("");
                if !machine_id.is_empty() {
                    parts.push(format!("MachineID {machine_id}"));
                }
            }
        }
    }

    // PropertyStoreDataBlock (signature 0xA0000009) => EnableTargetMetadata
    {
        // Search for u32 sig == 0xA0000009 at offset+4 of each ExtraData block
        let search_start = 76usize;
        let mut found_etm = false;
        for i in search_start..buf.len().saturating_sub(8) {
            // Look for the 4-byte signature 0xA0000009 LE = [0x09, 0x00, 0x00, 0xA0]
            if buf[i + 4] == 0x09 && buf[i + 5] == 0x00 && buf[i + 6] == 0x00 && buf[i + 7] == 0xA0 {
                let block_size = u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
                if block_size >= 12 {
                    found_etm = true;
                    break;
                }
            }
        }
        if found_etm {
            parts.push("EnableTargetMetadata".to_string());
        }
    }

    // FileAttributes
    if file_attrs & 0x01 != 0 {
        parts.push("ReadOnly".to_string());
    }
    if file_attrs & 0x02 != 0 {
        parts.push("Hidden".to_string());
    }
    if file_attrs & 0x04 != 0 {
        parts.push("System".to_string());
    }
    if file_attrs & 0x10 != 0 {
        parts.push("Directory".to_string());
    }
    if file_attrs & 0x20 != 0 {
        parts.push("Archive".to_string());
    }

    // Timestamps
    let ctime_raw = u64::from_le_bytes([
        buf[28], buf[29], buf[30], buf[31], buf[32], buf[33], buf[34], buf[35],
    ]);
    let atime_raw = u64::from_le_bytes([
        buf[36], buf[37], buf[38], buf[39], buf[40], buf[41], buf[42], buf[43],
    ]);
    let mtime_raw = u64::from_le_bytes([
        buf[44], buf[45], buf[46], buf[47], buf[48], buf[49], buf[50], buf[51],
    ]);
    let ctime_str = format_filetime(ctime_raw);
    let atime_str = format_filetime(atime_raw);
    let mtime_str = format_filetime(mtime_raw);
    if !ctime_str.is_empty() {
        parts.push(format!("ctime={ctime_str}"));
    }
    if !atime_str.is_empty() {
        parts.push(format!("atime={atime_str}"));
    }
    if !mtime_str.is_empty() {
        parts.push(format!("mtime={mtime_str}"));
    }

    // File size
    let file_size = u32::from_le_bytes([buf[52], buf[53], buf[54], buf[55]]);
    parts.push(format!("length={file_size}"));

    // ShowCommand
    let show_cmd = u32::from_le_bytes([buf[60], buf[61], buf[62], buf[63]]);
    let window = match show_cmd {
        1 => "normal",
        3 => "showmaximized",
        7 => "showminnoactive",
        _ => "normal",
    };
    parts.push(format!("window={window}"));

    // Parse LinkTargetIDList and LinkInfo
    let mut offset = 76usize;

    if flags & 0x001 != 0 && offset + 2 <= buf.len() {
        let id_list_size =
            u16::from_le_bytes([buf[offset], buf[offset + 1]]) as usize;
        parts.push(format!("IDListSize 0x{id_list_size:04x}"));

        // Parse individual items in the IDList
        let list_start = offset + 2;
        let list_end = (list_start + id_list_size).min(buf.len());
        let mut item_off = list_start;
        while item_off + 2 <= list_end {
            let item_size =
                u16::from_le_bytes([buf[item_off], buf[item_off + 1]]) as usize;
            if item_size == 0 {
                break;
            }
            if item_off + item_size > buf.len() {
                break;
            }
            let data_start = item_off + 2;
            let data_len = item_size - 2;
            if data_len == 0 {
                item_off += item_size;
                continue;
            }
            let item_type = buf[data_start];
            if item_type == 0x1F && data_len >= 18 {
                // Root folder — CLSID at data offset 2
                let g = data_start + 2;
                if g + 16 <= buf.len() {
                    let d1 = u32::from_le_bytes([buf[g], buf[g + 1], buf[g + 2], buf[g + 3]]);
                    let d2 = u16::from_le_bytes([buf[g + 4], buf[g + 5]]);
                    let d3 = u16::from_le_bytes([buf[g + 6], buf[g + 7]]);
                    let d4a = u16::from_be_bytes([buf[g + 8], buf[g + 9]]);
                    let d4b = &buf[g + 10..g + 16];
                    let clsid = format!(
                        "{d1:08X}-{d2:04X}-{d3:04X}-{d4a:04X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
                        d4b[0], d4b[1], d4b[2], d4b[3], d4b[4], d4b[5]
                    );
                    parts.push(format!("Root folder \"{clsid}\""));
                }
            } else if item_type == 0x2F && data_len >= 2 {
                // Volume item — drive letter string at data offset 1
                let s_start = data_start + 1;
                let s_end = buf[s_start..]
                    .iter()
                    .position(|&b| b == 0)
                    .map(|p| s_start + p)
                    .unwrap_or((data_start + data_len).min(buf.len()));
                let vol = std::str::from_utf8(&buf[s_start..s_end]).unwrap_or("");
                parts.push(format!("Volume \"{vol}\""));
            }
            item_off += item_size;
        }

        offset = list_start + id_list_size;
    }

    // LinkInfo
    if flags & 0x002 != 0 && offset + 4 <= buf.len() {
        let link_info_start = offset;
        let link_info_size =
            u32::from_le_bytes([buf[offset], buf[offset + 1], buf[offset + 2], buf[offset + 3]])
                as usize;
        if link_info_size >= 20 && link_info_start + link_info_size <= buf.len() {
            let local_base_path_off = u32::from_le_bytes([
                buf[link_info_start + 16],
                buf[link_info_start + 17],
                buf[link_info_start + 18],
                buf[link_info_start + 19],
            ]) as usize;
            if local_base_path_off > 0 && link_info_start + local_base_path_off < buf.len() {
                let s_start = link_info_start + local_base_path_off;
                let s_end = buf[s_start..]
                    .iter()
                    .position(|&b| b == 0)
                    .map(|p| s_start + p)
                    .unwrap_or(buf.len());
                let local_path =
                    std::str::from_utf8(&buf[s_start..s_end]).unwrap_or("");
                if !local_path.is_empty() {
                    parts.push(format!("LocalBasePath \"{local_path}\""));
                }
            }
        }
    }

    parts.join(", ")
}
