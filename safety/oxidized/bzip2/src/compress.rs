use bzip2::Compression;
use bzip2::read::BzEncoder;
use std::io::{self, Read, Write};

pub fn compress(input: &mut dyn Read, output: &mut dyn Write, level: u32) -> io::Result<()> {
    let mut encoder = BzEncoder::new(input, Compression::new(level));
    io::copy(&mut encoder, output)?;
    Ok(())
}
