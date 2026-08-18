use super::{Shell, ShellExport};
use crate::env::Env;

pub struct Systemd;

/// Check if value is already encapsulated by a given quote char on both ends.
/// If so, return the inner content and true; otherwise return the original and false.
fn cut_encapsulated<'a>(value: &'a str, encap: &str) -> (&'a str, bool) {
    if let Some(rest) = value.strip_prefix(encap)
        && let Some(inner) = rest.strip_suffix(encap)
    {
        return (inner, true);
    }
    (value, false)
}

/// Sanitize a value for systemd EnvironmentFile format.
fn sanitize_value(value: &str) -> String {
    let special_chars = ['\n', '\\', '"', '\''];
    let contains_special = value.chars().any(|c| special_chars.contains(&c));

    if !contains_special {
        return value.to_string();
    }

    // Check if single-quote encapsulated
    let (inner, was_single) = cut_encapsulated(value, "'");
    if was_single {
        return format!("'{}'", inner.replace('\'', "\\'"));
    }

    // Otherwise strip double-quote encapsulation if present
    let (inner, _) = cut_encapsulated(value, "\"");
    format!("\"{}\"", inner.replace('"', "\\\""))
}

impl Shell for Systemd {
    fn hook(&self, _self_path: &str) -> Result<String, String> {
        Err("this feature is not supported".to_string())
    }

    fn export(&self, e: &ShellExport) -> Result<String, String> {
        let mut out = String::new();
        for (key, value) in &e.vars {
            if let Some(v) = value {
                out.push_str(&format!("{}={}\n", key, sanitize_value(v)));
            }
            // systemd doesn't support unsetting
        }
        Ok(out)
    }

    fn dump(&self, env: &Env) -> Result<String, String> {
        let mut out = String::new();
        for (key, value) in env {
            out.push_str(&format!("{}={}\n", key, sanitize_value(value)));
        }
        Ok(out)
    }
}
