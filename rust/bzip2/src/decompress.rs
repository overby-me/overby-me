use bzip2::read::BzDecoder;
use std::io::{self, Read, Write};

pub fn decompress(input: &mut dyn Read, output: &mut dyn Write) -> io::Result<()> {
    let mut decoder = BzDecoder::new(input);
    io::copy(&mut decoder, output)?;
    Ok(())
}
