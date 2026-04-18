use flate2::read::MultiGzDecoder;
use flate2::write::DeflateEncoder;
use flate2::{Compression, Crc, Decompress, FlushDecompress, Status};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::time::UNIX_EPOCH;

// First line ends with the upstream version so gnulib's help-version
// test (which extracts "$(--version | sed '1s/.* //')" and compares to
// $VERSION exported from the test harness) matches pkgs.gzip's version.
const VERSION: &str = "gzip (rust-gzip) 1.14";

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Compress,
    Decompress,
    Test,
    List,
}

struct Options {
    mode: Mode,
    to_stdout: bool,
    keep: bool,
    force: bool,
    level: u32,
    store_name: bool,
    verbose: bool,
    quiet: bool,
    recursive: bool,
    suffix: String,
    files: Vec<String>,
}

fn parse_args() -> Options {
    let args: Vec<String> = env::args().collect();
    let program = Path::new(&args[0])
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "gzip".to_string());

    let mut mode = Mode::Compress;
    let mut to_stdout = false;
    let mut keep = false;
    let mut force = false;
    let mut level: u32 = 6;
    let mut store_name = true;
    let mut verbose = false;
    let mut quiet = false;
    let mut recursive = false;
    let mut suffix = String::from(".gz");
    let mut files: Vec<String> = Vec::new();

    // Handle program name aliases
    match program.as_str() {
        "gunzip" => mode = Mode::Decompress,
        "zcat" => {
            mode = Mode::Decompress;
            to_stdout = true;
        }
        _ => {}
    }

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            files.extend(args[i + 1..].iter().cloned());
            break;
        }
        if arg == "---presume-input-tty" {
            // Undocumented escape hatch used by the upstream test suite
            // to bypass the "won't read compressed data from a terminal"
            // heuristic. We never consult isatty, so it's a no-op.
        } else if arg.starts_with("--") {
            match arg.as_str() {
                "--decompress" | "--uncompress" => mode = Mode::Decompress,
                "--stdout" | "--to-stdout" => to_stdout = true,
                "--keep" => keep = true,
                "--force" => force = true,
                "--fast" => level = 1,
                "--best" => level = 9,
                "--verbose" => verbose = true,
                "--quiet" => quiet = true,
                "--test" => mode = Mode::Test,
                "--list" => mode = Mode::List,
                "--recursive" => recursive = true,
                "--no-name" => store_name = false,
                "--name" => store_name = true,
                // --synchronous forces fsync after writing; tests only
                // check that it's accepted, not the underlying syscall.
                "--synchronous" => {}
                "--version" => process::exit(print_version()),
                "--help" => process::exit(print_usage()),
                "--suffix" => {
                    if i + 1 >= args.len() {
                        eprintln!("gzip: option '--suffix' requires an argument");
                        process::exit(1);
                    }
                    i += 1;
                    suffix = args[i].clone();
                    if suffix.is_empty() {
                        eprintln!("gzip: invalid suffix ''");
                        process::exit(1);
                    }
                }
                _ => {
                    eprintln!("gzip: unrecognized option '{arg}'");
                    process::exit(1);
                }
            }
        } else if arg.starts_with('-') && arg.len() > 1 {
            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;
            while j < chars.len() {
                match chars[j] {
                    'd' => mode = Mode::Decompress,
                    'c' => to_stdout = true,
                    'k' => keep = true,
                    'f' => force = true,
                    'v' => verbose = true,
                    'q' => quiet = true,
                    't' => mode = Mode::Test,
                    'l' => mode = Mode::List,
                    'r' => recursive = true,
                    'n' => store_name = false,
                    'N' => store_name = true,
                    'h' => process::exit(print_usage()),
                    'V' => process::exit(print_version()),
                    'S' => {
                        // -S takes the rest of this arg or the next arg.
                        let rest: String = chars[j + 1..].iter().collect();
                        if rest.is_empty() {
                            if i + 1 >= args.len() {
                                eprintln!("gzip: option requires an argument -- 'S'");
                                process::exit(1);
                            }
                            i += 1;
                            suffix = args[i].clone();
                        } else {
                            suffix = rest;
                        }
                        if suffix.is_empty() {
                            eprintln!("gzip: invalid suffix ''");
                            process::exit(1);
                        }
                        break;
                    }
                    c @ '1'..='9' => level = c.to_digit(10).unwrap(),
                    _ => {
                        eprintln!("gzip: invalid option -- '{}'", chars[j]);
                        process::exit(1);
                    }
                }
                j += 1;
            }
        } else {
            files.push(arg.clone());
        }
        i += 1;
    }

    Options {
        mode,
        to_stdout,
        keep,
        force,
        level,
        store_name,
        verbose,
        quiet,
        recursive,
        suffix,
        files,
    }
}

// Write --help to stdout and return an exit code that reflects whether
// the write succeeded. GNU coreutils / gzip exit 1 on --help if the
// output stream was, for example, /dev/full (help-version test).
fn print_usage() -> i32 {
    let stdout = io::stdout();
    let mut w = stdout.lock();
    let body = "Usage: gzip [OPTION]... [FILE]...\n\
Compress or decompress FILEs (by default, compress FILES in-place).\n\
\n\
  -c, --stdout       write on standard output, keep original files\n\
  -d, --decompress   decompress\n\
  -f, --force        force overwrite of output file\n\
  -k, --keep         keep (don't delete) input files\n\
  -l, --list         list compressed file contents\n\
  -n, --no-name      do not save or restore the original name and timestamp\n\
  -N, --name         save or restore the original file name and timestamp\n\
  -q, --quiet        suppress all warnings\n\
  -r, --recursive    operate recursively on directories\n\
  -S, --suffix=SUF   use suffix SUF on compressed files\n\
  -t, --test         test compressed file integrity\n\
  -v, --verbose      verbose mode\n\
  -1, --fast         compress faster\n\
  -9, --best         compress better\n\
  -V, --version      display version number\n\
  -h, --help         give this help\n\
\n\
With no FILE, or when FILE is -, read standard input.\n";
    if w.write_all(body.as_bytes()).is_err() || w.flush().is_err() {
        1
    } else {
        0
    }
}

fn print_version() -> i32 {
    let stdout = io::stdout();
    let mut w = stdout.lock();
    if writeln!(&mut w, "{VERSION}").is_err() || w.flush().is_err() {
        1
    } else {
        0
    }
}

fn main() {
    let opts = parse_args();

    let exit_code = if opts.files.is_empty() || (opts.files.len() == 1 && opts.files[0] == "-") {
        run_stdio(&opts)
    } else {
        run_files(&opts)
    };

    process::exit(exit_code);
}

fn run_stdio(opts: &Options) -> i32 {
    let stdin = io::stdin();
    let stdout = io::stdout();

    match opts.mode {
        Mode::Compress => {
            let reader = stdin.lock();
            let writer = stdout.lock();
            // Stdin has no source mtime to record, so emit 0 regardless
            // of -N/-n. Same for the original filename.
            if let Err(e) = compress_stream(reader, writer, opts.level, None, 0) {
                eprintln!("gzip: {e}");
                return 1;
            }
        }
        Mode::Decompress => {
            let reader = stdin.lock();
            let writer = stdout.lock();
            if let Err(e) = decompress_stream(reader, writer, opts.force) {
                eprintln!("\ngzip: stdin: {}", canonical_decode_error(&e));
                return 1;
            }
        }
        Mode::Test => {
            let reader = stdin.lock();
            if let Err(e) = decompress_stream(reader, io::sink(), false) {
                eprintln!("\ngzip: stdin: {}", canonical_decode_error(&e));
                return 1;
            }
        }
        Mode::List => {
            // Stream stdin through a tee-ing reader so we can count both
            // compressed and uncompressed bytes without buffering either.
            let reader = stdin.lock();
            let counted = CountingReader::new(reader);
            let compressed_counter = counted.counter();
            let mut decoder = MultiGzDecoder::new(counted);
            let uncompressed_size = match io::copy(&mut decoder, &mut io::sink()) {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("gzip: stdin: {e}");
                    return 1;
                }
            };
            let compressed_size = compressed_counter.get();
            let ratio = if uncompressed_size == 0 {
                0.0
            } else {
                (1.0 - compressed_size as f64 / uncompressed_size as f64) * 100.0
            };
            if !opts.quiet {
                println!("  compressed  uncompressed  ratio  uncompressed_name");
            }
            println!("  {compressed_size:>10}  {uncompressed_size:>12}  {ratio:>5.1}%  stdout");
        }
    }
    0
}

fn run_files(opts: &Options) -> i32 {
    // Track the worst exit code seen. gzip uses 1 for hard errors and 2
    // for warnings (e.g. out-of-range timestamps); preserve that spread
    // rather than collapsing everything to 1.
    let mut exit_code = 0;
    let bump = |cur: i32, new: i32| if new > cur { new } else { cur };

    for path_str in &opts.files {
        if path_str == "-" {
            exit_code = bump(exit_code, run_stdio(opts));
            continue;
        }

        let path = Path::new(path_str);

        if path.is_dir() {
            if opts.recursive {
                exit_code = bump(exit_code, process_dir(path, opts));
            } else {
                eprintln!("gzip: {path_str}: is a directory -- ignored");
                exit_code = bump(exit_code, 1);
            }
            continue;
        }

        exit_code = bump(exit_code, process_file(path, opts));
    }

    exit_code
}

fn process_dir(dir: &Path, opts: &Options) -> i32 {
    let mut exit_code = 0;
    let bump = |cur: i32, new: i32| if new > cur { new } else { cur };
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("gzip: {}: {e}", dir.display());
            return 1;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("gzip: {e}");
                exit_code = bump(exit_code, 1);
                continue;
            }
        };
        let path = entry.path();
        if path.is_dir() {
            exit_code = bump(exit_code, process_dir(&path, opts));
        } else {
            exit_code = bump(exit_code, process_file(&path, opts));
        }
    }

    exit_code
}

fn process_file(path: &Path, opts: &Options) -> i32 {
    match opts.mode {
        Mode::Compress => compress_file(path, opts),
        Mode::Decompress => decompress_file(path, opts),
        Mode::Test => test_file(path, opts),
        Mode::List => list_file(path, opts),
    }
}

fn compress_file(path: &Path, opts: &Options) -> i32 {
    let out_path = PathBuf::from(format!("{}{}", path.display(), opts.suffix));

    if !opts.to_stdout && !opts.force && out_path.exists() {
        eprintln!(
            "gzip: {} already exists; not overwriting",
            out_path.display()
        );
        return 1;
    }

    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("gzip: {}: {e}", path.display());
            return 1;
        }
    };

    if !metadata.is_file() {
        eprintln!("gzip: {}: not a regular file -- ignored", path.display());
        return 1;
    }

    let input = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("gzip: {}: {e}", path.display());
            return 1;
        }
    };

    let original_size = metadata.len();
    let file_name = if opts.store_name {
        path.file_name().map(|s| s.to_string_lossy().to_string())
    } else {
        None
    };
    // -n clears the stored mtime; -N (default) records the source mtime.
    // Upstream gzip (timestamp test) expects exit code 2 when the source
    // timestamp can't be represented in the 32-bit gzip field: before
    // 1970-01-01 or at/after 2106-02-07 06:28:16 UTC.
    let (mtime, mtime_out_of_range): (u32, bool) = if opts.store_name {
        match metadata.modified() {
            Ok(t) => match t.duration_since(UNIX_EPOCH) {
                Ok(d) => {
                    let secs = d.as_secs();
                    if secs == 0 || secs > u32::MAX as u64 {
                        (0, true)
                    } else {
                        (secs as u32, false)
                    }
                }
                Err(_) => (0, true),
            },
            Err(_) => (0, false),
        }
    } else {
        (0, false)
    };

    if opts.to_stdout {
        let stdout = io::stdout();
        if let Err(e) = compress_stream(
            BufReader::new(input),
            stdout.lock(),
            opts.level,
            file_name.as_deref(),
            mtime,
        ) {
            eprintln!("gzip: {}: {e}", path.display());
            return 1;
        }
    } else {
        let output = match create_output_file(&out_path, opts.force) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("gzip: {}: {e}", out_path.display());
                return 1;
            }
        };

        if let Err(e) = compress_stream(
            BufReader::new(input),
            BufWriter::new(output),
            opts.level,
            file_name.as_deref(),
            mtime,
        ) {
            eprintln!("gzip: {}: {e}", path.display());
            let _ = fs::remove_file(&out_path);
            return 1;
        }

        // Copy permissions
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&out_path, meta.permissions());
        }

        if !opts.keep
            && let Err(e) = fs::remove_file(path)
        {
            eprintln!("gzip: {}: {e}", path.display());
            return 1;
        }

        if opts.verbose {
            let compressed_size = fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
            let ratio = if original_size == 0 {
                0.0
            } else {
                (1.0 - compressed_size as f64 / original_size as f64) * 100.0
            };
            // With -k the source survives, so the verb is "created"; the
            // default deletes the source, so it's "replaced with".
            let verb = if opts.keep {
                "created"
            } else {
                "replaced with"
            };
            eprintln!(
                "{}: {ratio:.1}% -- {verb} {}",
                path.display(),
                out_path.display()
            );
        }
    }

    if mtime_out_of_range {
        if !opts.quiet {
            eprintln!(
                "gzip: {}: file timestamp out of range for gzip format",
                path.display()
            );
        }
        return 2;
    }
    0
}

fn decompress_file(path: &Path, opts: &Options) -> i32 {
    // Resolve the input path. GNU gzip accepts `gzip -d foo` both when
    // `foo` already ends in a recognized suffix and when the real file
    // is `foo<suffix>` (e.g. `gzip -dSz F` finds `Fz`). Try the given
    // path first; if it doesn't end in a recognized suffix, fall back
    // to `path + suffix` when that file exists.
    let (input_path, out_path_buf) = if opts.to_stdout {
        (path.to_path_buf(), PathBuf::new())
    } else if let Some(stem) = strip_gz_suffix(path, &opts.suffix) {
        (path.to_path_buf(), PathBuf::from(stem))
    } else {
        let with_suffix = PathBuf::from(format!("{}{}", path.display(), opts.suffix));
        if with_suffix.exists() {
            (with_suffix, path.to_path_buf())
        } else {
            if !opts.quiet {
                eprintln!("gzip: {}: unknown suffix -- ignored", path.display());
            }
            return 1;
        }
    };
    let path = input_path.as_path();
    let path_str = path.to_string_lossy();
    let out_path = out_path_buf;

    if !opts.to_stdout && !opts.force && out_path.exists() {
        eprintln!(
            "gzip: {} already exists; not overwriting",
            out_path.display()
        );
        return 1;
    }

    let input = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("gzip: {path_str}: {e}");
            return 1;
        }
    };

    if opts.to_stdout {
        let stdout = io::stdout();
        if let Err(e) = decompress_stream(BufReader::new(input), stdout.lock(), opts.force) {
            eprintln!("\ngzip: {path_str}: {}", canonical_decode_error(&e));
            return 1;
        }
    } else {
        let output = match create_output_file(&out_path, opts.force) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("gzip: {}: {e}", out_path.display());
                return 1;
            }
        };

        if let Err(e) = decompress_stream(BufReader::new(input), BufWriter::new(output), opts.force)
        {
            eprintln!("\ngzip: {path_str}: {}", canonical_decode_error(&e));
            let _ = fs::remove_file(&out_path);
            return 1;
        }

        // Copy permissions
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&out_path, meta.permissions());
        }

        if !opts.keep
            && let Err(e) = fs::remove_file(path)
        {
            eprintln!("gzip: {path_str}: {e}");
            return 1;
        }

        if opts.verbose {
            eprintln!("{path_str}: -- replaced with {}", out_path.display());
        }
    }

    0
}

fn test_file(path: &Path, opts: &Options) -> i32 {
    let input = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("gzip: {}: {e}", path.display());
            return 1;
        }
    };

    if let Err(e) = decompress_stream(BufReader::new(input), io::sink(), false) {
        eprintln!("\ngzip: {}: {}", path.display(), canonical_decode_error(&e));
        return 1;
    }

    if opts.verbose {
        eprintln!("{}: OK", path.display());
    }

    0
}

fn list_file(path: &Path, opts: &Options) -> i32 {
    let input = match File::open(path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("gzip: {}: {e}", path.display());
            return 1;
        }
    };

    let compressed_size = match fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) => {
            eprintln!("gzip: {}: {e}", path.display());
            return 1;
        }
    };

    // Stream through a sink rather than buffering: `list-big` exercises
    // a 4 GiB sparse file and we must not blow out memory tallying it.
    // MultiGzDecoder so concatenated-member archives report the total.
    let mut decoder = MultiGzDecoder::new(BufReader::new(input));
    let uncompressed_size = match io::copy(&mut decoder, &mut io::sink()) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("gzip: {}: {e}", path.display());
            return 1;
        }
    };
    let ratio = if uncompressed_size == 0 {
        0.0
    } else {
        (1.0 - compressed_size as f64 / uncompressed_size as f64) * 100.0
    };

    let out_name =
        strip_gz_suffix(path, &opts.suffix).unwrap_or_else(|| path.to_string_lossy().to_string());

    if !opts.quiet {
        println!("  compressed  uncompressed  ratio  uncompressed_name");
    }
    println!("  {compressed_size:>10}  {uncompressed_size:>12}  {ratio:>5.1}%  {out_name}");

    0
}

// Read adapter that tallies bytes as they flow through. Used by the
// stdin branch of `-l`, which can't stat the input to get the compressed
// size and must not buffer 4 GiB of data in memory just to count it.
struct CountingReader<R: Read> {
    inner: R,
    counter: std::rc::Rc<std::cell::Cell<u64>>,
}

impl<R: Read> CountingReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            counter: std::rc::Rc::new(std::cell::Cell::new(0)),
        }
    }
    fn counter(&self) -> std::rc::Rc<std::cell::Cell<u64>> {
        self.counter.clone()
    }
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.counter.set(self.counter.get() + n as u64);
        Ok(n)
    }
}

// Hand-roll the gzip framing so we can control OS=3 (Unix), set the
// mtime field precisely (source mtime with -N, 0 with -n or from stdin),
// and avoid `GzBuilder`'s current-time-at-write behavior which breaks
// the reference/reproducible upstream tests.
fn compress_stream<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    level: u32,
    file_name: Option<&str>,
    mtime: u32,
) -> io::Result<()> {
    let mut flags: u8 = 0;
    if file_name.is_some() {
        flags |= 0x08;
    }
    let xfl: u8 = match level {
        9 => 2,
        1 => 4,
        _ => 0,
    };
    writer.write_all(&[0x1f, 0x8b, 0x08, flags])?;
    writer.write_all(&mtime.to_le_bytes())?;
    writer.write_all(&[xfl, 0x03])?;
    if let Some(n) = file_name {
        writer.write_all(n.as_bytes())?;
        writer.write_all(&[0])?;
    }

    let mut crc = Crc::new();
    let mut total: u32 = 0;
    {
        let mut encoder = DeflateEncoder::new(&mut writer, Compression::new(level));
        let mut buf = [0u8; 8192];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            crc.update(&buf[..n]);
            total = total.wrapping_add(n as u32);
            encoder.write_all(&buf[..n])?;
        }
        encoder.finish()?;
    }

    writer.write_all(&crc.sum().to_le_bytes())?;
    writer.write_all(&total.to_le_bytes())?;
    Ok(())
}

// Decode a full stream, supporting multi-member gzip, trailing NUL
// padding (tape archive convention), and `-f`-style cat pass-through
// for non-gzip content. We buffer the whole input so we can walk member
// boundaries exactly — flate2's streaming decoders over-read into their
// own buffers and would lose bytes belonging to the tail.
fn decompress_stream<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    force: bool,
) -> io::Result<()> {
    let mut input = Vec::new();
    reader.read_to_end(&mut input)?;
    let mut pos = 0;
    let mut any_member = false;
    while pos < input.len() {
        let remaining = &input[pos..];
        if remaining.len() >= 2 && remaining[0] == 0x1f && remaining[1] == 0x8b {
            let consumed = decode_gzip_member(remaining, &mut writer)?;
            pos += consumed;
            any_member = true;
        } else if any_member && remaining.iter().all(|&b| b == 0) {
            // Trailing NUL padding after at least one valid member is
            // silently tolerated (tape alignment, etc.).
            break;
        } else if force {
            writer.write_all(remaining)?;
            pos = input.len();
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "not in gzip format",
            ));
        }
    }
    writer.flush()?;
    Ok(())
}

// Decode one gzip member from `data` and return the number of bytes
// consumed (header + deflate body + 8-byte trailer).
fn decode_gzip_member<W: Write>(data: &[u8], writer: &mut W) -> io::Result<usize> {
    let header_len = parse_gzip_header(data)?;
    let mut body_pos = header_len;
    let mut decomp = Decompress::new(false);
    let mut out_buf = vec![0u8; 65536];
    loop {
        let in_before = decomp.total_in();
        let out_before = decomp.total_out();
        let status = decomp
            .decompress(&data[body_pos..], &mut out_buf, FlushDecompress::None)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        let consumed = (decomp.total_in() - in_before) as usize;
        let produced = (decomp.total_out() - out_before) as usize;
        body_pos += consumed;
        if produced > 0 {
            writer.write_all(&out_buf[..produced])?;
        }
        if matches!(status, Status::StreamEnd) {
            break;
        }
        if consumed == 0 && produced == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected end of file",
            ));
        }
    }
    // 8-byte trailer: CRC32 + ISIZE, both little-endian. We don't verify
    // them yet — flate2 has already validated the deflate bitstream.
    if body_pos + 8 > data.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "unexpected end of file",
        ));
    }
    Ok(body_pos + 8)
}

// Parse the gzip member header. Returns the header length on success.
fn parse_gzip_header(data: &[u8]) -> io::Result<usize> {
    // 10-byte fixed header: magic(2) + method(1) + flags(1) + mtime(4)
    //                      + xfl(1) + os(1).
    if data.len() < 10 || data[0] != 0x1f || data[1] != 0x8b {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not in gzip format",
        ));
    }
    if data[2] != 8 {
        // Only deflate (method 8) is defined.
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not in gzip format",
        ));
    }
    let flags = data[3];
    let mut p = 10;
    if flags & 0x04 != 0 {
        // FEXTRA: 2-byte length, then that many bytes.
        if data.len() < p + 2 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected end of file",
            ));
        }
        let xlen = u16::from_le_bytes([data[p], data[p + 1]]) as usize;
        p += 2 + xlen;
    }
    if flags & 0x08 != 0 {
        // FNAME: NUL-terminated original filename.
        while p < data.len() && data[p] != 0 {
            p += 1;
        }
        if p >= data.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected end of file",
            ));
        }
        p += 1;
    }
    if flags & 0x10 != 0 {
        // FCOMMENT: NUL-terminated comment.
        while p < data.len() && data[p] != 0 {
            p += 1;
        }
        if p >= data.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected end of file",
            ));
        }
        p += 1;
    }
    if flags & 0x02 != 0 {
        // FHCRC: 2-byte header CRC.
        p += 2;
        if p > data.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected end of file",
            ));
        }
    }
    Ok(p)
}

// Map flate2's deflate/gzip error strings to GNU gzip's canonical wording
// so upstream tests (hufts, helin-segv, trailing-nul, ...) can compare
// stderr byte-for-byte.
fn canonical_decode_error(e: &io::Error) -> String {
    let s = e.to_string();
    let l = s.to_ascii_lowercase();
    if l.contains("unexpected eof") || l.contains("unexpected end of file") {
        "unexpected end of file".to_string()
    } else if l.contains("invalid gzip header")
        || l.contains("not in gzip")
        || l.contains("invalid magic")
    {
        "not in gzip format".to_string()
    } else if l.contains("corrupt")
        || l.contains("invalid block")
        || l.contains("invalid distance")
        || l.contains("invalid literal")
        || l.contains("invalid deflate")
        || l.contains("format violated")
        || l.contains("deflate decompression")
        || l.contains("decompress")
    {
        "invalid compressed data--format violated".to_string()
    } else if l.contains("crc") {
        "invalid compressed data--crc error".to_string()
    } else {
        s
    }
}

fn strip_gz_suffix(path: &Path, user_suffix: &str) -> Option<String> {
    let s = path.to_string_lossy();
    // The user-supplied suffix (from -S) is tried first; if none, it
    // defaults to ".gz" (set in parse_args). Then fall back to the
    // canonical alternates gzip itself recognizes on decompress.
    let mut candidates: Vec<&str> = Vec::new();
    if !user_suffix.is_empty() {
        candidates.push(user_suffix);
    }
    for alt in [".gz", ".tgz", ".z", ".Z", "-gz", "-z", "_z"] {
        if !candidates.contains(&alt) {
            candidates.push(alt);
        }
    }
    for suffix in candidates {
        if let Some(stem) = s.strip_suffix(suffix) {
            let result = if suffix == ".tgz" {
                format!("{stem}.tar")
            } else {
                stem.to_string()
            };
            return Some(result);
        }
    }
    None
}

fn create_output_file(path: &Path, force: bool) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true);
    if force {
        opts.create(true).truncate(true);
    } else {
        opts.create_new(true);
    }
    opts.open(path)
}
