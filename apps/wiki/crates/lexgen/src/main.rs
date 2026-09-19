//! `cargo run -p lexgen`: write `appview-client/src/generated.rs` from the
//! lexicons. Run from anywhere: the paths are relative to this crate.

use std::path::Path;

fn main() -> std::io::Result<()> {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lexicons = lexgen::read_lexicons(&here.join("../../lexicons/com/example/wiki"))?;
    let target = here.join("../appview-client/src/generated.rs");
    std::fs::write(&target, lexgen::generate(&lexicons))?;
    eprintln!(
        "wrote {} from {} lexicons",
        target.display(),
        lexicons.len()
    );
    Ok(())
}
