//! systemd-escape — Escape strings for use in systemd unit names.
//!
//! A drop-in replacement for `systemd-escape(1)` supporting:
//!
//! - Escaping arbitrary strings for use in unit names
//! - Unescaping unit name components back to the original string
//! - Path escaping/unescaping (for .mount / .device style names)
//! - Mangling strings into complete unit names
//! - Template instantiation (`--template`)
//! - Instance extraction (`--instance`)
//! - Appending a unit type suffix (`--suffix`)

use clap::Parser;
use libsystemd::unit_name;
use std::process;

#[derive(Parser, Debug)]
#[command(
    name = "systemd-escape",
    about = "Escape strings for use as systemd unit names",
    version
)]
struct Cli {
    /// Unescape the given strings instead of escaping them.
    #[arg(short, long)]
    unescape: bool,

    /// Mangle the given strings into unit names (appending .service if needed).
    #[arg(short, long)]
    mangle: bool,

    /// Treat the input/output as a filesystem path (strip leading/trailing
    /// slashes, collapse consecutive slashes).
    #[arg(short, long)]
    path: bool,

    /// Append the specified unit type suffix (e.g. "mount", "service") to
    /// the escaped string. The leading dot is added automatically if not
    /// present.
    #[arg(long, value_name = "SUFFIX")]
    suffix: Option<String>,

    /// Use the specified unit template and insert the escaped string as
    /// the instance. TEMPLATE should be a template unit name like
    /// "foo@.service".
    #[arg(long, value_name = "TEMPLATE")]
    template: Option<String>,

    /// When used with --unescape, extract and print only the instance part
    /// of a template unit name.
    #[arg(long)]
    instance: bool,

    /// The strings to escape or unescape. If none are given, reads from
    /// stdin (one per line).
    strings: Vec<String>,
}


/// Known valid unit type suffixes.
const VALID_SUFFIXES: &[&str] = &[
    "service",
    "socket",
    "target",
    "device",
    "mount",
    "automount",
    "swap",
    "timer",
    "path",
    "slice",
    "scope",
];

fn normalize_suffix(suffix: &str) -> Result<String, String> {
    let bare = suffix.strip_prefix('.').unwrap_or(suffix);
    if bare.is_empty() || !VALID_SUFFIXES.contains(&bare) {
        return Err(format!("Invalid unit type suffix: {suffix}"));
    }
    Ok(format!(".{bare}"))
}

fn do_escape(input: &str, cli: &Cli) -> Result<String, String> {
    if cli.unescape {
        // Unescape mode
        if cli.instance || cli.template.is_some() {
            // Extract instance from a template instance name
            let name = input;
            match unit_name::unit_name_template_split(name) {
                Some((_prefix, instance, _suffix)) => {
                    // Instance must be non-empty (otherwise it's a template, not an instance)
                    if instance.is_empty() {
                        return Err(format!("Not a template instance unit name: {name}"));
                    }
                    if cli.path {
                        unit_name::unit_name_path_unescape(instance)
                            .ok_or_else(|| format!("Failed to path-unescape instance: {instance}"))
                    } else {
                        unit_name::unit_name_unescape(instance)
                            .ok_or_else(|| format!("Failed to unescape instance: {instance}"))
                    }
                }
                None => Err(format!("Not a template instance unit name: {name}")),
            }
        } else {
            // Strip known suffix before unescaping if present
            let name = strip_unit_suffix(input);

            if cli.path {
                unit_name::unit_name_path_unescape(name)
                    .ok_or_else(|| format!("Failed to path-unescape: {name}"))
            } else {
                unit_name::unit_name_unescape(name)
                    .ok_or_else(|| format!("Failed to unescape: {name}"))
            }
        }
    } else if cli.mangle {
        // Mangle mode — empty input is invalid
        if input.is_empty() {
            return Err("Cannot mangle empty string".to_string());
        }
        Ok(unit_name::unit_name_mangle(input))
    } else {
        // Escape mode (default)
        if cli.path {
            // Validate path input
            let escaped = unit_name::unit_name_path_escape_checked(input)
                .ok_or_else(|| format!("Invalid path: {input}"))?;

            // Apply --template if given
            if let Some(template) = &cli.template {
                if !unit_name::is_template(template) {
                    return Err(format!("Not a valid template unit name: {template}"));
                }
                unit_name::template_instantiate(template, &escaped)
                    .ok_or_else(|| {
                        format!("Failed to instantiate template {template} with {escaped}")
                    })
            } else if let Some(suffix) = &cli.suffix {
                let norm = normalize_suffix(suffix)?;
                Ok(format!("{escaped}{norm}"))
            } else {
                Ok(escaped)
            }
        } else {
            let escaped = unit_name::unit_name_escape(input);

            // For --template, the escaped instance must be non-empty
            if let Some(template) = &cli.template {
                if escaped.is_empty() {
                    return Err(format!(
                        "Cannot instantiate template {template} with empty instance"
                    ));
                }
                if !unit_name::is_template(template) {
                    return Err(format!("Not a valid template unit name: {template}"));
                }
                unit_name::template_instantiate(template, &escaped)
                    .ok_or_else(|| {
                        format!("Failed to instantiate template {template} with {escaped}")
                    })
            } else if let Some(suffix) = &cli.suffix {
                let norm = normalize_suffix(suffix)?;
                Ok(format!("{escaped}{norm}"))
            } else {
                Ok(escaped)
            }
        }
    }
}

/// Strip a recognized unit type suffix from a name, returning the name
/// part without the suffix. If no recognized suffix is found, returns
/// the whole input.
fn strip_unit_suffix(name: &str) -> &str {
    const SUFFIXES: &[&str] = &[
        ".service",
        ".socket",
        ".target",
        ".device",
        ".mount",
        ".automount",
        ".swap",
        ".timer",
        ".path",
        ".slice",
        ".scope",
    ];

    for &suffix in SUFFIXES {
        if let Some(stripped) = name.strip_suffix(suffix) {
            return stripped;
        }
    }
    name
}

fn main() {
    let cli = Cli::parse();

    // Validate conflicting options
    if cli.unescape && cli.mangle {
        eprintln!("Error: --unescape and --mangle cannot be used together.");
        process::exit(1);
    }
    if cli.mangle && cli.template.is_some() {
        eprintln!("Error: --mangle and --template cannot be used together.");
        process::exit(1);
    }
    if cli.mangle && cli.suffix.is_some() {
        eprintln!("Error: --mangle and --suffix cannot be used together.");
        process::exit(1);
    }
    if cli.suffix.is_some() && cli.template.is_some() {
        eprintln!("Error: --suffix and --template cannot be used together.");
        process::exit(1);
    }
    if cli.instance && !cli.unescape {
        eprintln!("Error: --instance can only be used with --unescape.");
        process::exit(1);
    }
    if let Some(template) = &cli.template {
        if template.is_empty() || !unit_name::is_template(template) {
            eprintln!("Error: Not a valid template unit name: {template}");
            process::exit(1);
        }
    }

    let inputs = if cli.strings.is_empty() {
        eprintln!("Error: no input strings provided.");
        process::exit(1);
    } else {
        cli.strings.clone()
    };

    if inputs.is_empty() {
        eprintln!("Error: no input strings provided.");
        process::exit(1);
    }

    let mut exit_code = 0;
    let mut results = Vec::new();

    for input in &inputs {
        match do_escape(input, &cli) {
            Ok(result) => results.push(result),
            Err(e) => {
                eprintln!("Error: {e}");
                exit_code = 1;
            }
        }
    }

    if !results.is_empty() {
        println!("{}", results.join(" "));
    }

    process::exit(exit_code);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cli(
        unescape: bool,
        mangle: bool,
        path: bool,
        suffix: Option<&str>,
        template: Option<&str>,
        instance: bool,
    ) -> Cli {
        Cli {
            unescape,
            mangle,
            path,
            suffix: suffix.map(String::from),
            template: template.map(String::from),
            instance,
            strings: vec![],
        }
    }

    #[test]
    fn test_basic_escape() {
        let cli = make_cli(false, false, false, None, None, false);
        assert_eq!(do_escape("foo bar", &cli).unwrap(), r"foo\x20bar");
    }

    #[test]
    fn test_basic_unescape() {
        let cli = make_cli(true, false, false, None, None, false);
        assert_eq!(do_escape(r"foo\x20bar", &cli).unwrap(), "foo bar");
    }

    #[test]
    fn test_path_escape() {
        let cli = make_cli(false, false, true, None, None, false);
        assert_eq!(do_escape("/foo/bar", &cli).unwrap(), "foo-bar");
    }

    #[test]
    fn test_path_unescape() {
        let cli = make_cli(true, false, true, None, None, false);
        assert_eq!(do_escape("foo-bar", &cli).unwrap(), "/foo/bar");
    }

    #[test]
    fn test_escape_with_suffix() {
        let cli = make_cli(false, false, true, Some("mount"), None, false);
        assert_eq!(do_escape("/foo/bar", &cli).unwrap(), "foo-bar.mount");
    }

    #[test]
    fn test_escape_with_template() {
        let cli = make_cli(false, false, false, None, Some("foo@.service"), false);
        assert_eq!(do_escape("bar", &cli).unwrap(), "foo@bar.service");
    }

    #[test]
    fn test_unescape_instance() {
        let cli = make_cli(true, false, false, None, None, true);
        assert_eq!(do_escape("foo@bar.service", &cli).unwrap(), "bar");
    }

    #[test]
    fn test_mangle() {
        let cli = make_cli(false, true, false, None, None, false);
        assert_eq!(do_escape("foo", &cli).unwrap(), "foo.service");
        assert_eq!(do_escape("foo.service", &cli).unwrap(), "foo.service");
    }

    #[test]
    fn test_unescape_strips_suffix() {
        let cli = make_cli(true, false, false, None, None, false);
        assert_eq!(do_escape("foo-bar.mount", &cli).unwrap(), "foo/bar");
    }

    #[test]
    fn test_escape_root_path() {
        let cli = make_cli(false, false, true, Some("mount"), None, false);
        assert_eq!(do_escape("/", &cli).unwrap(), "-.mount");
    }
}
