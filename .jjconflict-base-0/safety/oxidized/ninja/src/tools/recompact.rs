//! `-t recompact` / `-t restat`: rewrite `.ninja_log` keeping only the
//! latest entry per output, and warn if the on-disk log is too old to
//! upgrade in place (target: `test_issue_2048`).

use crate::build::log::{self, BuildLog};

pub fn run() -> Result<u8, String> {
    let log_path = ".ninja_log";
    let log: BuildLog = log::load(log_path);
    if log.too_old {
        // Reference ninja deletes the unreadable log so the next build
        // re-populates a fresh v6 file. We do the same; failure to
        // remove the file is non-fatal.
        let _ = std::fs::remove_file(log_path);
        println!("ninja: warning: build log version is too old; starting over");
        return Ok(0);
    }
    if let Err(e) = log::recompact(log_path) {
        // A non-existent log is fine — there's nothing to recompact and
        // ninja exits 0. Anything else gets reported but not fatally.
        if e.kind() != std::io::ErrorKind::NotFound {
            eprintln!("ninja: warning: failed to recompact build log: {e}");
        }
    }
    Ok(0)
}
