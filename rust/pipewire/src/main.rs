// rust-pipewire: a multicall binary providing every PipeWire tool/daemon.
//
// Dispatched by argv[0] (or argv[1] when invoked via `rust-pipewire <tool>`).
// See PLAN.md for the architecture and the list of tools we provide.

use std::env;
use std::path::Path;
use std::process::ExitCode;

mod pipewire_lib;
mod spa;
mod spa_tools;
mod tools;

fn tool_name(arg: &str) -> &str {
    Path::new(arg)
        .file_name()
        .map(|s| s.to_str().unwrap_or(""))
        .unwrap_or("")
}

fn dispatch(name: &str, args: &[String]) -> i32 {
    match name {
        // SPA tools
        "spa-json-dump" => spa_tools::spa_json_dump::main(args),

        // PipeWire tools — full help-output parity, real behavior follows.
        "pw-cli" => tools::pw_cli::main(args),
        "pw-config" => tools::pw_config::main(args),
        "pw-dot" => tools::pw_dot::main(args),
        "pw-dump" => tools::pw_dump::main(args),
        "pw-link" => tools::pw_link::main(args),
        "pw-metadata" => tools::pw_metadata::main(args),
        "pw-mididump" => tools::pw_mididump::main(args),
        "pw-mon" => tools::pw_mon::main(args),
        "pw-profiler" => tools::pw_profiler::main(args),
        "pw-top" => tools::pw_top::main(args),

        // Daemons
        "pipewire" | "pipewire-pulse" => tools::pipewire_daemon::main(args),

        // Internal helpers — used by the test harness, not user-facing.
        "pod-encode" => tools::pod_test_helper::main(args),
        "proto-probe" => tools::proto_test_helper::main(args),

        // Placeholder dispatchers for the rest. Each prints a stable
        // "not implemented" message and exits 0 on --version / --help.
        // The pw-cat family (cat / play / record + the midi/dsd/encoded
        // variants) all share the same help text apart from the tool name
        // and the trailing mode-flag block.
        "pw-cat" | "pw-play" | "pw-record" | "pw-midiplay" | "pw-midirecord"
        | "pw-midi2play" | "pw-midi2record" | "pw-sysex" | "pw-dsdplay"
        | "pw-encplay" => tools::pw_cat::main(args),

        "pipewire-aes67" | "pipewire-avb" | "pipewire-vulkan" | "pw-loopback"
        | "pw-reserve" | "pw-container" | "pw-v4l2" | "spa-inspect"
        | "spa-monitor" | "spa-acp-tool" | "spa-resample" => stub_main(name, args),

        "rust-pipewire" => {
            eprintln!(
                "rust-pipewire: multicall binary, invoke via a tool symlink or `rust-pipewire <tool> [args]`."
            );
            2
        }
        _ => {
            eprintln!("rust-pipewire: unknown tool {name:?}");
            2
        }
    }
}

fn stub_main(name: &str, args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--version") {
        let v = pipewire_lib::version::PIPEWIRE_API_VERSION;
        println!("{name}");
        println!("Compiled with libpipewire {v}");
        println!("Linked with libpipewire {v}");
        return 0;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{name}: not yet implemented in rust-pipewire");
        return 0;
    }
    eprintln!("{name}: not yet implemented in rust-pipewire");
    1
}

fn main() -> ExitCode {
    let argv: Vec<String> = env::args().collect();
    let argv0_name = tool_name(argv.first().map(String::as_str).unwrap_or(""));

    // If invoked as `rust-pipewire <tool> [args]`, dispatch on argv[1] and
    // shift it out so the called tool sees its own name in argv[0].
    let (name, args): (String, Vec<String>) = if argv0_name == "rust-pipewire" && argv.len() >= 2 {
        let mut v = vec![argv[1].clone()];
        v.extend(argv.iter().skip(2).cloned());
        (argv[1].clone(), v)
    } else {
        (argv0_name.to_string(), argv.clone())
    };

    let code = dispatch(&name, &args);
    // Exit codes only span 0..=255 in practice; clamp defensively so a
    // stray negative/large value still produces a valid ExitCode.
    ExitCode::from(code.clamp(0, 255).try_into().unwrap_or(2))
}
