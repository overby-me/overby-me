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
            "--version" | "-V" => {
                print_version(argv0);
                return 0;
            }
            _ => {}
        }
    }

    eprintln!("{argv0}: not yet implemented in rust-pipewire");
    1
}

fn print_help(argv0: &str) {
    // C's `show_help` prefixes "Available commands:\n" before the usual
    // tool-name + options block (yes, it really emits "Available
    // commands:" as the very first line — even before the usage).
    println!("Available commands:");
    println!("{argv0} [options] [COMMAND]");
    println!("  -h, --help                            Show this help");
    println!("  -v  --verbose                         Be verbose");
    println!("  -c  --card                            Card number");
    println!("  -p  --properties                      Extra properties:");
    println!("                                         'key=value ... '");
    println!();
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
