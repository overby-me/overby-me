use anyhow::{bail, Result};
use git2::Repository;

pub fn clone_repo(_url: &str, _path: &std::path::Path) -> Result<Repository> {
    // TODO: support ssh/https and depth
    bail!("clone_repo not implemented")
}
