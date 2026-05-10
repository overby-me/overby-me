// pw-mon: PipeWire registry monitor.

use crate::tools::common::{expand_short_clusters, print_version};

pub fn main(raw_args: &[String]) -> i32 {
    let argv0 = raw_args.first().map(String::as_str).unwrap_or("pw-mon");
    // Expand `-hV` / `-Vh` etc. into separate flags so getopt-cluster
    // semantics match the C tool.
    let args = expand_short_clusters(raw_args, &['h', 'V', 'N', 'C', 'o', 'a', 'p']);
    let mut remote: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--" => break, // End of options; rest are positional and we ignore them.
            "-h" | "--help" => {
                print_help(argv0);
                return 0;
            }
            "--version" | "-V" => {
                print_version(argv0);
                return 0;
            }
            s if s.starts_with("--help=") => {
                eprintln!("{argv0}: option '--help' doesn't allow an argument");
                print_help(argv0);
                return u8::MAX as i32;
            }
            s if s.starts_with("--version=") => {
                eprintln!("{argv0}: option '--version' doesn't allow an argument");
                print_help(argv0);
                return u8::MAX as i32;
            }
            // No-arg long flags reject `--FOO=value`.
            s if s.starts_with("--no-colors=")
                || s.starts_with("--hide-props=")
                || s.starts_with("--hide-params=")
                || s.starts_with("--print-separator=") =>
            {
                let name = s.split_once('=').map(|(n, _)| n).unwrap_or(s);
                eprintln!("{argv0}: option '{name}' doesn't allow an argument");
                print_help(argv0);
                return u8::MAX as i32;
            }
            // -r and -C take a required arg.
            opt @ ("-r" | "--remote") => {
                if i + 1 >= args.len() {
                    if opt == "--remote" {
                        eprintln!("{argv0}: option '--remote' requires an argument");
                    } else {
                        eprintln!("{argv0}: option requires an argument -- 'r'");
                    }
                    print_help(argv0);
                    return u8::MAX as i32;
                }
                remote = Some(args[i + 1].clone());
                i += 2;
                continue;
            }
            s if s.starts_with("--remote=") => {
                remote = Some(s["--remote=".len()..].to_string());
            }
            s if s.starts_with("-r") && s.len() > 2 => {
                remote = Some(s[2..].to_string());
            }
            // -C and --color have OPTIONAL argument; bare `-C` is fine.
            "-C" | "--color" => {}
            "-N" | "--no-colors" | "-o" | "--hide-props" | "-a"
            | "--hide-params" | "-p" | "--print-separator" => {}
            s if s.starts_with("--color=") => {
                let val = &s["--color=".len()..];
                if !matches!(val, "" | "never" | "always" | "auto") {
                    eprintln!("Invalid color: {val}");
                    print_help(argv0);
                    return 255;
                }
            }
            "-" => {
                // Lone `-` is a positional, not an option.
            }
            s if s.starts_with("--") => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            s if s.starts_with('-') && s.len() == 2 => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            s if s.starts_with('-') && !s.starts_with("--") => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            s if s.starts_with('-') => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return u8::MAX as i32;
            }
            _ => {}
        }
        i += 1;
    }
    // C connects to the daemon and dumps the registry. Use the remote
    // path if --remote was given (so a bad remote yields the connect-fail
    // error). Otherwise PIPEWIRE_REMOTE picks up the env-supplied name,
    // and falls back to the default socket.
    let env_remote = std::env::var("PIPEWIRE_REMOTE").ok();
    let chosen = remote.clone().or(env_remote);
    let connect = if let Some(name) = &chosen {
        let runtime = std::env::var("PIPEWIRE_RUNTIME_DIR")
            .or_else(|_| std::env::var("XDG_RUNTIME_DIR"))
            .unwrap_or_else(|_| "/tmp".to_string());
        let path = std::path::PathBuf::from(runtime).join(name);
        crate::pipewire_lib::client::Client::connect_path(&path)
    } else {
        crate::pipewire_lib::client::Client::connect_default()
    };
    match connect {
        Ok(_) => 0,
        Err(_) => {
            eprintln!("can't connect: {}", crate::tools::common::connect_failure_msg());
            255
        }
    }
}

fn print_help(argv0: &str) {
    println!("{argv0} [options]");
    println!("  -h, --help                            Show this help");
    println!("      --version                         Show version");
    println!("  -r, --remote                          Remote daemon name");
    println!("  -N, --no-colors                       disable color output");
    println!(
        "  -C, --color[=WHEN]                    whether to enable color support. WHEN is `never`, `always`, or `auto`"
    );
    println!("  -o, --hide-props                      hide node properties");
    println!("  -a, --hide-params                     hide node parameters");
    println!(
        "  -p, --print-separator                 print empty line after every event to help streaming parser"
    );
}
