use crate::cli::Config;
use crate::compress::compress;
use crate::decompress::decompress;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::PathBuf;

pub fn process_file(config: &Config, path: &str) -> io::Result<()> {
    let input_path = PathBuf::from(path);

    if !input_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("bzip2: {path}: No such file or directory"),
        ));
    }

    if !input_path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("bzip2: {path}: Not a regular file"),
        ));
    }

    if config.decompress {
        let ext = input_path.extension().unwrap_or_default();
        if ext != "bz2" && ext != "bz" && ext != "tbz2" && ext != "tbz" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("bzip2: {path}: unknown suffix -- ignored"),
            ));
        }

        let output_path = match ext.to_str().unwrap_or("") {
            "bz2" => {
                let s = input_path.to_string_lossy();
                PathBuf::from(s.strip_suffix(".bz2").unwrap())
            }
            "bz" => {
                let s = input_path.to_string_lossy();
                PathBuf::from(s.strip_suffix(".bz").unwrap())
            }
            "tbz2" => input_path.with_extension("tar"),
            "tbz" => input_path.with_extension("tar"),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("bzip2: {path}: unknown suffix -- ignored"),
                ));
            }
        };

        if config.test {
            let mut input = File::open(&input_path)?;
            decompress(&mut input, &mut io::sink())?;
            if config.verbose {
                eprintln!("{path}: ok");
            }
        } else if config.stdout {
            let mut input = File::open(&input_path)?;
            let mut stdout = io::stdout().lock();
            decompress(&mut input, &mut stdout)?;
        } else {
            if output_path.exists() && !config.force {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "bzip2: output file {} already exists",
                        output_path.display()
                    ),
                ));
            }
            let mut input = File::open(&input_path)?;
            let mut output = File::create(&output_path)?;
            decompress(&mut input, &mut output)?;
            if !config.keep {
                fs::remove_file(&input_path)?;
            }
            if config.verbose {
                eprintln!("  {path}: done", path = path);
            }
        }
    } else {
        let output_path = PathBuf::from(format!("{path}.bz2"));

        if config.stdout {
            let mut input = File::open(&input_path)?;
            let mut stdout = io::stdout().lock();
            compress(&mut input, &mut stdout, config.level)?;
        } else {
            if output_path.exists() && !config.force {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!(
                        "bzip2: output file {} already exists",
                        output_path.display()
                    ),
                ));
            }

            // Preserve permissions
            let metadata = fs::metadata(&input_path)?;
            let mut input = File::open(&input_path)?;
            let mut output = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&output_path)?;
            compress(&mut input, &mut output, config.level)?;

            // Copy permissions
            fs::set_permissions(&output_path, metadata.permissions())?;

            if !config.keep {
                fs::remove_file(&input_path)?;
            }
            if config.verbose {
                eprintln!("  {path}: done");
            }
        }
    }

    Ok(())
}

pub fn process_stdin(config: &Config) -> io::Result<()> {
    // Read from stdin, write to stdout
    let mut stdin = io::stdin().lock();
    let mut stdout = io::stdout().lock();
    if config.decompress {
        if config.test {
            decompress(&mut stdin, &mut io::sink())
        } else {
            decompress(&mut stdin, &mut stdout)
        }
    } else {
        compress(&mut stdin, &mut stdout, config.level)
    }
}
