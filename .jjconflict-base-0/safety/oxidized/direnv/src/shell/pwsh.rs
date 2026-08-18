use super::{Shell, ShellExport};
use crate::env::Env;
use std::fmt::Write;

pub struct Pwsh;

const PWSH_HOOK: &str = r#"using namespace System;
using namespace System.Management.Automation;

if ($PSVersionTable.PSVersion.Major -lt 7 -or ($PSVersionTable.PSVersion.Major -eq 7 -and $PSVersionTable.PSVersion.Minor -lt 2)) {
    throw "direnv: PowerShell version $($PSVersionTable.PSVersion) does not meet the minimum required version 7.2!"
}

$hook = [EventHandler[LocationChangedEventArgs]] {
  param([object] $source, [LocationChangedEventArgs] $eventArgs)
  end {
    $export = ("{{.SelfPath}}" export pwsh) -join [Environment]::NewLine;
    if ($export) {
      Invoke-Expression -Command $export;
    }
  }
};
$currentAction = $ExecutionContext.SessionState.InvokeCommand.LocationChangedAction;
if ($currentAction) {
  $ExecutionContext.SessionState.InvokeCommand.LocationChangedAction = [Delegate]::Combine($currentAction, $hook);
}
else {
  $ExecutionContext.SessionState.InvokeCommand.LocationChangedAction = $hook;
};
"#;

/// Escape environment variable keys for PowerShell.
/// Special chars *, :, =, ?, [, ] are hex-encoded; { and } are backtick-escaped.
fn escape_env_key(s: &str) -> String {
    if s.is_empty() {
        return "__DiReNv_UnReAcHaBlE__".to_string();
    }
    let mut out = String::new();
    for ch in s.bytes() {
        match ch {
            b'*' | b':' | b'=' | b'?' | b'[' | b']' => {
                write!(out, "\\x{ch:02x}").unwrap();
            }
            b'{' => out.push_str("`{"),
            b'}' => out.push_str("`}"),
            _ => out.push(ch as char),
        }
    }
    out
}

/// Escape environment variable keys using verbatim strings (for Remove-Item -LiteralPath).
fn escape_verbatim_env_key(s: &str) -> String {
    if s.is_empty() {
        return "__DiReNv_UnReAcHaBlE__".to_string();
    }
    s.replace('\'', "''")
}

/// Escape a value as a PowerShell verbatim string (single-quoted).
fn escape_verbatim_string(s: &str) -> String {
    s.replace('\'', "''")
}

impl Shell for Pwsh {
    fn hook(&self, self_path: &str) -> Result<String, String> {
        Ok(PWSH_HOOK.replace("{{.SelfPath}}", self_path))
    }

    fn export(&self, e: &ShellExport) -> Result<String, String> {
        let mut out = String::new();
        for (key, value) in &e.vars {
            if key.is_empty() {
                continue;
            }
            match value {
                Some(v) => {
                    write!(
                        out,
                        "${{env:{}}}='{}';",
                        escape_env_key(key),
                        escape_verbatim_string(v)
                    )
                    .unwrap();
                }
                None => {
                    write!(
                        out,
                        "Remove-Item -LiteralPath 'env:/{}';",
                        escape_verbatim_env_key(key)
                    )
                    .unwrap();
                }
            }
        }
        Ok(out)
    }

    fn dump(&self, env: &Env) -> Result<String, String> {
        let mut out = String::new();
        for (key, value) in env {
            write!(
                out,
                "${{env:{}}}='{}';",
                escape_env_key(key),
                escape_verbatim_string(value)
            )
            .unwrap();
        }
        Ok(out)
    }
}
