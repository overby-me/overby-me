//! Shell abstraction layer - hook generation, export, and dump for all supported shells.

mod bash;
mod elvish;
mod fish;
mod gha;
mod gzenv_shell;
mod json_shell;
mod murex;
mod pwsh;
mod systemd;
mod tcsh;
mod vim;
mod zsh;

use crate::env::Env;
use std::collections::BTreeMap;

/// ShellExport represents environment variables to add and remove.
/// A None value means the variable should be removed.
pub struct ShellExport {
    pub vars: BTreeMap<String, Option<String>>,
}

impl ShellExport {
    pub fn new() -> Self {
        Self {
            vars: BTreeMap::new(),
        }
    }

    pub fn add(&mut self, key: &str, value: &str) {
        self.vars.insert(key.to_string(), Some(value.to_string()));
    }

    pub fn remove(&mut self, key: &str) {
        self.vars.insert(key.to_string(), None);
    }
}

/// The Shell trait represents the interaction with the host shell.
pub trait Shell {
    /// Returns the shell hook script that gets evaluated into the host shell config.
    fn hook(&self, self_path: &str) -> Result<String, String>;

    /// Outputs the ShellExport as an evaluatable string in the host shell.
    fn export(&self, e: &ShellExport) -> Result<String, String>;

    /// Outputs an evaluatable string that sets the entire env in the host shell.
    #[allow(dead_code)]
    fn dump(&self, env: &Env) -> Result<String, String>;
}

/// Detect a shell from a target string (typically $0).
pub fn detect_shell(target: &str) -> Option<Box<dyn Shell>> {
    let target = target.strip_prefix('-').unwrap_or(target);

    // Strip path prefix
    let target = target.rsplit('/').next().unwrap_or(target);

    match target {
        "bash" => Some(Box::new(bash::Bash)),
        "zsh" => Some(Box::new(zsh::Zsh)),
        "fish" => Some(Box::new(fish::Fish)),
        "tcsh" => Some(Box::new(tcsh::Tcsh)),
        "elvish" => Some(Box::new(elvish::Elvish)),
        "json" => Some(Box::new(json_shell::JsonShell)),
        "gzenv" => Some(Box::new(gzenv_shell::GzEnvShell)),
        "gha" => Some(Box::new(gha::Gha)),
        "vim" => Some(Box::new(vim::Vim)),
        "pwsh" => Some(Box::new(pwsh::Pwsh)),
        "murex" => Some(Box::new(murex::Murex)),
        "systemd" => Some(Box::new(systemd::Systemd)),
        _ => None,
    }
}

/// List of supported shell names.
pub fn supported_shells() -> Vec<&'static str> {
    vec![
        "bash", "elvish", "fish", "gha", "gzenv", "json", "murex", "pwsh", "systemd", "tcsh",
        "vim", "zsh",
    ]
}
