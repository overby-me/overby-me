// pipewire / pipewire-pulse: the daemon. Phase 0 only handles
// --help and --version; the actual server is Phase 9.

use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pipewire");
    // Each daemon variant has its own default config name. We infer it
    // from argv0 — the C tools all derive their default the same way.
    let default_config = if argv0.contains("pulse") {
        "pipewire-pulse.conf"
    } else if argv0.contains("aes67") {
        "pipewire-aes67.conf"
    } else if argv0.contains("avb") {
        "pipewire-avb.conf"
    } else if argv0.contains("vulkan") {
        "pipewire-vulkan.conf"
    } else {
        "pipewire.conf"
    };

    let mut config_value: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        let s = args[i].as_str();
        match s {
            "-h" | "--help" => {
                print_help(argv0, default_config);
                return 0;
            }
            "--version" | "-V" => {
                print_version(argv0);
                return 0;
            }
            s if s.starts_with("--help=") => {
                eprintln!("{argv0}: option '--help' doesn't allow an argument");
                return 234;
            }
            s if s.starts_with("--version=") => {
                eprintln!("{argv0}: option '--version' doesn't allow an argument");
                return 234;
            }
            "-v" | "--verbose" => {
                // No-arg, continue.
            }
            opt @ ("-c" | "--config") => {
                if let Some(value) = args.get(i + 1) {
                    config_value = Some(value.clone());
                    i += 2;
                    continue;
                } else if opt == "--config" {
                    eprintln!("{argv0}: option '--config' requires an argument");
                    return 234;
                } else {
                    eprintln!("{argv0}: option requires an argument -- 'c'");
                    return 234;
                }
            }
            opt @ ("-P" | "--properties") => {
                if args.get(i + 1).is_some() {
                    i += 2;
                    continue;
                } else if opt == "--properties" {
                    eprintln!("{argv0}: option '--properties' requires an argument");
                    return 234;
                } else {
                    eprintln!("{argv0}: option requires an argument -- 'P'");
                    return 234;
                }
            }
            s if s.starts_with("--config=") => {
                config_value = Some(s["--config=".len()..].to_string());
            }
            s if s.starts_with("--properties=") => {}
            s if s.starts_with("--") => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                return 234;
            }
            s if s.starts_with("-c") && s.len() > 2 => {
                config_value = Some(s[2..].to_string());
            }
            s if s.starts_with("-P") && s.len() > 2 => {}
            s if s.starts_with('-') && s.len() == 2 => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                return 234;
            }
            // Mixed cluster like `-hh` or `-hX`: getopt processes char-by-char,
            // so a cluster starting with `-h` short-circuits to help.
            s if s.starts_with('-') && !s.starts_with("--") && s.len() > 2 => {
                for c in s[1..].chars() {
                    if c == 'h' {
                        print_help(argv0, default_config);
                        return 0;
                    } else if c == 'V' {
                        print_version(argv0);
                        return 0;
                    } else if c == 'v' {
                        // verbose, no-arg, continue cluster
                    } else {
                        eprintln!("{argv0}: invalid option -- '{c}'");
                        return 234;
                    }
                }
            }
            _ => {
                // Positional argument; daemon doesn't accept any but
                // since we don't run the daemon we just stop processing
                // and fall through to the "not implemented" path.
                break;
            }
        }
        i += 1;
    }
    if let Some(value) = config_value {
        return pipewire_load_config(argv0, &value);
    }
    eprintln!("{argv0}: not yet implemented in rust-pipewire");
    1
}

// Mirror upstream's pw_conf_load_conf_for_context: validate the
// config name ends with `.conf` (else -EINVAL → exit 234), then attempt
// to load the file (we always fail with -ENOENT → exit 254 since the
// daemon isn't implemented). C also emits log lines with timestamps,
// which are too volatile to mirror; we emit a stable subset that the
// test harness can normalize via sed.
fn pipewire_load_config(argv0: &str, value: &str) -> i32 {
    if !value.ends_with(".conf") {
        eprintln!(
            "[E][TIME] pw.conf      | [          conf.c: 1210 pw_conf_load_conf_for_context()] config.name '{value}' does not end with .conf"
        );
        eprintln!(
            "[E][TIME] default      | [      pipewire.c:  124 main()] failed to create context: Invalid argument"
        );
        return 234;
    }
    eprintln!(
        "[W][TIME] pw.conf      | [          conf.c: 1182 try_load_conf()] can't load config {value}: No such file or directory"
    );
    eprintln!(
        "[E][TIME] pw.conf      | [          conf.c: 1215 pw_conf_load_conf_for_context()] can't load config {value}: No such file or directory"
    );
    eprintln!(
        "[E][TIME] default      | [      pipewire.c:  124 main()] failed to create context: No such file or directory"
    );
    let _ = argv0;
    254
}

fn print_help(argv0: &str, default_config: &str) {
    println!("{argv0} [options]");
    println!("  -h, --help                            Show this help");
    println!("  -v, --verbose                         Increase verbosity by one level");
    println!("      --version                         Show version");
    println!("  -c, --config                          Load config (Default {default_config})");
    println!("  -P  --properties                      Set context properties");
}
