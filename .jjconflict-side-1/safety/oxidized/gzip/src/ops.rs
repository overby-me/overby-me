use flate2::read::MultiGzDecoder;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::cli::{Mode, Options};
use crate::compress::compress_stream;
use crate::decompress::{canonical_decode_error, decompress_stream};
use crate::util::{CountingReader, create_output_file, strip_gz_suffix};

pub fn run_stdio(opts: &Options) -> i32 {
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

pub fn run_files(opts: &Options) -> i32 {
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
