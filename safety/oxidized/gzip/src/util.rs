use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::Path;

/// Read adapter that tallies bytes as they flow through. Used by the
/// stdin branch of `-l`, which can't stat the input to get the compressed
/// size and must not buffer 4 GiB of data in memory just to count it.
pub struct CountingReader<R: Read> {
    inner: R,
    counter: std::rc::Rc<std::cell::Cell<u64>>,
}

impl<R: Read> CountingReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            counter: std::rc::Rc::new(std::cell::Cell::new(0)),
        }
    }
    pub fn counter(&self) -> std::rc::Rc<std::cell::Cell<u64>> {
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

pub fn strip_gz_suffix(path: &Path, user_suffix: &str) -> Option<String> {
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

pub fn create_output_file(path: &Path, force: bool) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true);
    if force {
        opts.create(true).truncate(true);
    } else {
        opts.create_new(true);
    }
    opts.open(path)
}
