// spa-acp-tool: ALSA Card Profile interactive tool. The real
// implementation drives ALSA via the SPA bluez/alsa monitor and is
// Phase 10+ work. We mirror the C tool's --help / -h output exactly.

use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("spa-acp-tool");

    for a in args.iter().skip(1) {
        match a.as_str() {
            "-h" | "--help" => {
                print_help(argv0);
                return 0;
            }
            // spa-acp-tool's getopt has no --version or -V — both fall
            // through to the unrecognized-option branch.
            // Long unknown:
            s if s.starts_with("--")
                && !matches!(s, "--verbose" | "--card" | "--properties") =>
            {
                eprintln!("{argv0}: unrecognized option '{s}'");
                eprintln!("error: unknown option '?'");
                print_options(argv0);
                println!("Available commands:");
                print_commands();
                return 0;
            }
            // Short unknown: getopt prints `invalid option -- 'X'`
            // (single-quoted single char, like pw-cat).
            s if s.starts_with('-')
                && s.len() == 2
                && !matches!(s, "-v" | "-c" | "-p") =>
            {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                eprintln!("error: unknown option '?'");
                print_options(argv0);
                println!("Available commands:");
                print_commands();
                return 0;
            }
            _ => {}
        }
    }

    eprintln!("{argv0}: not yet implemented in rust-pipewire");
    1
}

fn print_help(argv0: &str) {
    // C's --help path: cmd_help (stderr "Available commands:" + stdout
    // commands) is called from inside show_usage(false) (stdout). With
    // line-buffered stderr and block-buffered stdout, the user sees:
    //   1. "Available commands:"  (stderr, flushed first)
    //   2. tool name + options   (stdout)
    //   3. blank line             (stdout, from trailing \n)
    //   4. command list           (stdout)
    println!("Available commands:");
    print_options(argv0);
    print_commands();
}

fn print_options(argv0: &str) {
    println!("{argv0} [options] [COMMAND]");
    println!("  -h, --help                            Show this help");
    println!("  -v  --verbose                         Be verbose");
    println!("  -c  --card                            Card number");
    println!("  -p  --properties                      Extra properties:");
    println!("                                         'key=value ... '");
    println!();
}

fn print_commands() {
    let cmds = [
        ("help                      ", "Show available commands (h)"),
        ("quit                      ", "Quit (q)"),
        ("card            <id>      ", "Probe card (c)"),
        ("info                      ", "List card info (i)"),
        ("list                      ", "List all objects (l)"),
        ("list-verbose              ", "List all data (lv)"),
        ("list-profiles   [id]      ", "List profiles (lpr)"),
        ("set-profile     <id>      ", "Activate a profile (spr)"),
        ("list-ports      [id]      ", "List ports (lp)"),
        ("set-port        <id>      ", "Activate a port (sp)"),
        ("list-devices    [id]      ", "List available devices (ld)"),
        ("get-volume      <id>      ", "Get volume from device (gv)"),
        ("set-volume      <id> <vol>", "Set volume on device (v)"),
        ("inc-volume      <id>      ", "Increase volume on device (v+)"),
        ("dec-volume      <id>      ", "Decrease volume on device (v-)"),
        ("get-mute        <id>      ", "Get mute state from device (gm)"),
        ("set-mute        <id> <val>", "Set mute on device (sm)"),
        ("toggle-mute     <id>      ", "Toggle mute on device (m)"),
    ];
    for (head, desc) in cmds {
        println!("\t{head}\t{desc}");
    }
}
