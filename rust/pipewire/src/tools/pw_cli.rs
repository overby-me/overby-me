// pw-cli: PipeWire command-line client.
// Phase 0: only --version / --help / -h. Real subcommand REPL is Phase 7.

use crate::pipewire_lib::version::PIPEWIRE_API_VERSION;

pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map(String::as_str).unwrap_or("pw-cli");

    for a in args.iter().skip(1) {
        match a.as_str() {
            "--version" => {
                println!("{argv0}");
                println!("Compiled with libpipewire {PIPEWIRE_API_VERSION}");
                println!("Linked with libpipewire {PIPEWIRE_API_VERSION}");
                return 0;
            }
            "-h" | "--help" => {
                print_help(argv0);
                return 0;
            }
            // Stop at the first non-flag — that's where commands start.
            s if !s.starts_with('-') => break,
            // Other flags are ignored for now; they'll be parsed in Phase 7.
            _ => continue,
        }
    }

    // No --version/--help and no implemented subcommand: behave like the C
    // tool by printing help and exiting with code 0 when no command is given.
    print_help(argv0);
    0
}

fn print_help(argv0: &str) {
    // Mirror the C tool's --help layout exactly. The tests normalize the
    // leading store-path/binary-name; everything after must be byte-identical.
    println!("{argv0} [options] [command]");
    println!("  -h, --help                            Show this help");
    println!("      --version                         Show version");
    println!("  -d, --daemon                          Start as daemon (Default false)");
    println!("  -r, --remote                          Remote daemon name");
    println!("  -m, --monitor                         Monitor activity");
    println!();
    println!("Available commands:");
    let cmds = [
        ("help | h            ", "Show this help"),
        ("list-vars | lv      ", "List all variables"),
        (
            "load-module | lm    ",
            "Load a module. <module-name> [<module-arguments>]",
        ),
        ("unload-module | um  ", "Unload a module. <module-var>"),
        (
            "connect | con       ",
            "Connect to a remote. [<remote-name>]",
        ),
        (
            "disconnect | dis    ",
            "Disconnect from a remote. [<remote-var>]",
        ),
        ("list-remotes | lr   ", "List connected remotes."),
        (
            "switch-remote | sr  ",
            "Switch between current remotes. [<remote-var>]",
        ),
        (
            "list-objects | ls   ",
            "List objects or current remote. [<interface>]",
        ),
        (
            "info | i            ",
            "Get info about an object. <object-id>|all",
        ),
        (
            "create-device | cd  ",
            "Create a device from a factory. <factory-name> [<properties>]",
        ),
        (
            "create-node | cn    ",
            "Create a node from a factory. <factory-name> [<properties>]",
        ),
        (
            "destroy | d         ",
            "Destroy a global object. <object-id>",
        ),
        (
            "create-link | cl    ",
            "Create a link between nodes. <node-id> <port-id> <node-id> <port-id> [<properties>]",
        ),
        (
            "export-node | en    ",
            "Export a local node to the current remote. <node-id> [remote-var]",
        ),
        (
            "enum-params | e     ",
            "Enumerate params of an object <object-id> <param-id>",
        ),
        (
            "set-param | s       ",
            "Set param of an object <object-id> <param-id> <param-json>",
        ),
        (
            "permissions | sp    ",
            "Set permissions for a client <client-id> <object> <permission>",
        ),
        (
            "get-permissions | gp",
            "Get permissions of a client <client-id>",
        ),
        ("send-command | c    ", "Send a command <object-id>"),
        ("quit | q            ", "Quit"),
    ];
    for (head, desc) in cmds {
        println!("\t{head}\t{desc}");
    }
}
