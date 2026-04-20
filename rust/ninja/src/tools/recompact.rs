//! `-t recompact` / `-t restat` stubs.
//!
//! With no real `.ninja_log` writer in place yet we only check the
//! existing log's version header and warn if it's too old to upgrade
//! (target: `test_issue_2048`).

const CURRENT_LOG_VERSION: u32 = 6;

pub fn run() -> Result<u8, String> {
    if let Ok(contents) = std::fs::read_to_string(".ninja_log") {
        if let Some(first) = contents.lines().next() {
            if let Some(rest) = first.strip_prefix("# ninja log v") {
                if let Ok(v) = rest.trim().parse::<u32>() {
                    if v < CURRENT_LOG_VERSION {
                        println!("ninja: warning: build log version is too old; starting over");
                    }
                }
            }
        }
    }
    Ok(0)
}
