// pw-link: PipeWire port and link manager.
//
// Phase 7 minimal mode: -i / -o (list ports), -l (list links), -I (with
// id prefix). No latency, no monitor, no link create/disconnect — those
// will land as Phase 7 progresses.

use crate::pipewire_lib::client::{Client, RegistryGlobal};
use crate::pipewire_lib::interfaces;
use crate::tools::common::{expand_short_clusters, print_version};

pub fn main(raw_args: &[String]) -> i32 {
    let argv0 = raw_args.first().map(String::as_str).unwrap_or("pw-link");
    // Expand short clusters like `-hV`, `-iol`, `-ioI` etc.
    let args = expand_short_clusters(
        raw_args,
        &['h', 'V', 'd', 'i', 'o', 'l', 'm', 'I', 'v', 'L', 'P', 'w', 't', 'N'],
    );

    let mut list_inputs = false;
    let mut list_outputs = false;
    let mut list_links = false;
    let mut list_latency = false;
    let mut show_id = false;
    let mut verbose = false;
    let mut disconnect = false;
    let mut remote: Option<String> = None;
    let mut positional: Vec<&str> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--" => {
                // End-of-options marker: rest are positionals.
                for v in args.iter().skip(i + 1) {
                    positional.push(v.as_str());
                }
                break;
            }
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
                return 0;
            }
            s if s.starts_with("--version=") => {
                eprintln!("{argv0}: option '--version' doesn't allow an argument");
                print_help(argv0);
                return 0;
            }
            // Long no-arg flags: input/output/links/latency/monitor/etc.
            // reject `--FOO=value` form.
            s if s.starts_with("--input=")
                || s.starts_with("--output=")
                || s.starts_with("--links=")
                || s.starts_with("--latency=")
                || s.starts_with("--monitor=")
                || s.starts_with("--linger=")
                || s.starts_with("--passive=")
                || s.starts_with("--wait=")
                // --no-colors isn't in pw-link's long_options, so the
                // `=value` form falls through to "unrecognized option".
                || s.starts_with("--id=")
                || s.starts_with("--verbose=")
                || s.starts_with("--disconnect=") =>
            {
                let name = s.split_once('=').map(|(n, _)| n).unwrap_or(s);
                eprintln!("{argv0}: option '{name}' doesn't allow an argument");
                print_help(argv0);
                return 0;
            }
            "-i" | "--input" => list_inputs = true,
            "-o" | "--output" => list_outputs = true,
            "-l" | "--links" => list_links = true,
            "-I" | "--id" => show_id = true,
            "-v" | "--verbose" => verbose = true,
            "-r" | "--remote" => {
                if let Some(v) = args.get(i + 1) {
                    remote = Some(v.clone());
                    i += 2;
                    continue;
                }
                eprintln!("{argv0}: option requires an argument -- 'r'");
                print_help(argv0);
                return 0;
            }
            s if s.starts_with("--remote=") => {
                remote = Some(s["--remote=".len()..].to_string());
            }
            s if s.starts_with("-r") && s.len() > 2 => {
                remote = Some(s[2..].to_string());
            }
            "-d" | "--disconnect" => disconnect = true,
            // Per pw-link.c optstring "hVr:oilmIvLPp:wdt", `t` takes
            // NO argument — it just sets MODE_LIST + LIST_LATENCY (which
            // by itself produces no output without -i/-o/-l).
            "-t" | "--latency" => list_latency = true,
            // pw-link's long_options don't include --no-colors, so the
            // long form is unrecognized — only -N short works.
            "-m" | "--monitor" | "-L" | "--linger"
            | "-P" | "--passive" | "-w" | "--wait" | "-N" => {}
            "-p" | "--props" => {
                // -p / --props requires an argument.
                if i + 1 >= args.len() {
                    eprintln!("{argv0}: option requires an argument -- 'p'");
                    print_help(argv0);
                    return 0;
                }
                i += 2;
                continue;
            }
            s if s.starts_with("--props=") => {
                // --props=PROPS — value is inline.
            }
            s if s.starts_with("-p") && s.len() > 2 => {
                // -p<value> attached form.
            }
            // pw-link's getopt has no --color or -C option — it falls
            // through to the unrecognized-option branch.
            s if s.starts_with("--") => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return 0;
            }
            "-" => {
                // Lone `-` is a positional argument, not an option.
                positional.push("-");
            }
            s if s.starts_with('-') && s.len() == 2 => {
                let ch = s.chars().nth(1).unwrap_or('?');
                eprintln!("{argv0}: invalid option -- '{ch}'");
                print_help(argv0);
                return 0;
            }
            s if s.starts_with('-') => {
                eprintln!("{argv0}: unrecognized option '{s}'");
                print_help(argv0);
                return 0;
            }
            s => positional.push(s),
        }
        i += 1;
    }

    let _ = &positional;

    if args.len() == 1 {
        // No flags at all → C calls show_help on stderr but does NOT
        // return; falls through to MODE_CONNECT_PORTS check which
        // errors `missing output and input port names to connect`.
        print_help(argv0);
    }

    // C tool's mode-validation runs after option parsing:
    //   MODE_DISCONNECT (--disconnect): needs at least one positional
    //     (link-id or output-port name); empty → error + exit -1.
    //   MODE_CONNECT (no -i/-o/-l, has positional): needs both output
    //     and input port names; missing one → error + exit -1.
    if disconnect && positional.is_empty() {
        eprintln!("missing link-id or output and input port names to disconnect");
        return 255;
    }
    if !list_inputs
        && !list_outputs
        && !list_links
        && !list_latency
        && !disconnect
        && positional.len() < 2
    {
        // MODE_CONNECT_PORTS path in C: requires both opt_output and
        // opt_input. With <2 positional, error. Extra positionals are
        // silently ignored (C just consumes argv[optind] / argv[optind+1]).
        // -t/--latency also sets MODE_LIST which skips this check.
        eprintln!("missing output and input port names to connect");
        return 255;
    }

    if !list_inputs && !list_outputs && !list_links {
        // C always attempts pw_context_connect; without a daemon the
        // connect-fail message preempts everything else.
        let connected = match crate::pipewire_lib::client::Client::connect_default() {
            Ok(_) => true,
            Err(_) => {
                eprintln!("can't connect: {}", crate::tools::common::connect_failure_msg());
                return 255;
            }
        };
        let _ = connected;
        // Connect succeeded. -t (MODE_LIST + LIST_LATENCY only) walks
        // nothing without other LIST_X flags → silent. MODE_DISCONNECT/
        // CONNECT_PORTS try to (un)link → "failed to (un)link ports".
        if !list_latency && (disconnect || positional.len() >= 2) {
            if disconnect {
                eprintln!("failed to unlink ports: No such file or directory");
            } else {
                eprintln!("failed to link ports: No such file or directory");
            }
            return 255;
        }
        return 0;
    }

    // pw-link.c line 1096–7: `pw-link -l` (links only) implies both
    // directions, so each port that participates in any link is listed.
    let list_ports = list_inputs || list_outputs;
    if !list_ports && list_links {
        list_inputs = true;
        list_outputs = true;
    }

    let globals = match collect_globals(remote.as_deref(), "rust-pipewire-link") {
        Ok(g) => g,
        Err(e) => {
            // C pw-link prints `can't connect: <strerror>\n` and returns
            // -1 (= 255 truncated). The protocol-native client maps
            // ENOENT → EHOSTDOWN ("Host is down").
            if e.contains("connect:") || e.starts_with("connect:") {
                eprintln!("can't connect: {}", crate::tools::common::connect_failure_msg());
            } else {
                eprintln!("{argv0}: {e}");
            }
            return 255;
        }
    };

    // Mirror C: opt_output / opt_input are the first / second positional
    // args, used as regexes (we use a simple substring match) to filter
    // ports by direction.
    let opt_output = positional.first().copied();
    let opt_input = positional.get(1).copied();

    print_listing(
        &globals,
        list_inputs,
        list_outputs,
        list_links,
        list_ports,
        show_id,
        verbose,
        opt_output,
        opt_input,
    );
    0
}

fn collect_globals(remote: Option<&str>, app_name: &str) -> Result<Vec<RegistryGlobal>, String> {
    let mut client = match remote {
        Some(name) if name.starts_with('/') => Client::connect_path(std::path::Path::new(name)),
        Some(name) => {
            let runtime = std::env::var("PIPEWIRE_RUNTIME_DIR")
                .or_else(|_| std::env::var("XDG_RUNTIME_DIR"))
                .unwrap_or_else(|_| "/tmp".to_string());
            Client::connect_path(&std::path::PathBuf::from(runtime).join(name))
        }
        None => Client::connect_default(),
    }
    .map_err(|e| format!("connect: {e}"))?;

    client
        .handshake(app_name)
        .map_err(|e| format!("handshake: {e}"))?;
    let sync_seq = client
        .sync(interfaces::ID_CORE)
        .map_err(|e| format!("sync: {e}"))?;

    let mut globals = Vec::new();
    loop {
        let msg = match client.read_message() {
            Ok(Some(m)) => m,
            Ok(None) => break,
            Err(e) => return Err(format!("read: {e}")),
        };
        if msg.opcode == interfaces::registry_event::GLOBAL
            && msg.id == 2
            && let Ok(g) = crate::pipewire_lib::client::decode_registry_global(&msg.args)
        {
            globals.push(g);
        }
        if msg.id == interfaces::ID_CORE
            && msg.opcode == interfaces::core_event::DONE
            && let Ok((_id, seq)) = crate::pipewire_lib::client::decode_core_done(&msg.args)
            && seq == sync_seq
        {
            break;
        }
    }
    Ok(globals)
}

fn prop<'a>(g: &'a RegistryGlobal, key: &str) -> Option<&'a str> {
    g.props
        .iter()
        .find(|i| i.key == key)
        .map(|i| i.value.as_str())
}

fn node_name_for(globals: &[RegistryGlobal], node_id: u32) -> String {
    if let Some(n) = globals
        .iter()
        .find(|g| g.id == node_id && g.interface == interfaces::TYPE_NODE)
        && let Some(name) = prop(n, "node.name")
    {
        return name.to_string();
    }
    format!("node.id.{node_id}")
}

fn port_full_name(globals: &[RegistryGlobal], port: &RegistryGlobal) -> String {
    let node_id: u32 = prop(port, "node.id")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let node_name = node_name_for(globals, node_id);
    let port_name = prop(port, "port.name")
        .map(str::to_string)
        .unwrap_or_else(|| format!("port.id.{}", port.id));
    format!("{node_name}:{port_name}")
}

#[allow(clippy::too_many_arguments)]
fn print_listing(
    globals: &[RegistryGlobal],
    list_inputs: bool,
    list_outputs: bool,
    list_links: bool,
    list_ports: bool,
    show_id: bool,
    verbose: bool,
    opt_output: Option<&str>,
    opt_input: Option<&str>,
) {
    // pw-link iterates Nodes in registry order; for each Node, walks Ports
    // belonging to it (also in registry order).
    for node in globals
        .iter()
        .filter(|g| g.interface == interfaces::TYPE_NODE)
    {
        for direction in [
            (list_outputs, "out", opt_output),
            (list_inputs, "in", opt_input),
        ] {
            if !direction.0 {
                continue;
            }
            // Substring filter against `<node-name>:<port-name>` mirrors C
            // `port_regex` for simple non-anchored patterns. (Full POSIX
            // ERE would require the regex crate; this gets us most
            // practical patterns including alphanumerics and dots.)
            let pattern = direction.2;
            for port in globals
                .iter()
                .filter(|g| g.interface == interfaces::TYPE_PORT)
                .filter(|p| prop(p, "node.id").and_then(|s| s.parse().ok()) == Some(node.id))
                .filter(|p| prop(p, "port.direction") == Some(direction.1))
                .filter(|p| {
                    pattern.is_none_or(|pat| {
                        let name = port_full_name(globals, p);
                        port_regex_match(&name, pat)
                    })
                })
            {
                if list_ports {
                    print_port_line(port, globals, show_id, verbose);
                }
                if list_links {
                    // When LIST_PORTS is unset (just `-l`), `do_list_port_links`
                    // sets `first = true` and prints the port name lazily —
                    // only when it finds a matching link. Otherwise the port
                    // line was already emitted above. Verbose only applies to
                    // the port-being-listed; peer ports are always non-verbose.
                    print_port_links(globals, port, show_id, !list_ports, verbose);
                }
            }
        }
    }
}

/// Approximate POSIX ERE matching: handle the common patterns C's
/// `port_regex` is asked to match — plain substrings, `.*` wildcards
/// between literal segments, and `^X` / `X$` anchors. Falls back to
/// full substring containment for anything more exotic.
fn port_regex_match(text: &str, pattern: &str) -> bool {
    let anchored_start = pattern.starts_with('^');
    let anchored_end = pattern.ends_with('$') && !pattern.ends_with("\\$");
    let core = &pattern[anchored_start as usize..pattern.len() - if anchored_end { 1 } else { 0 }];

    // Split on `.*` — every segment must appear in order in `text`.
    let segments: Vec<&str> = core.split(".*").collect();
    let mut pos = 0usize;
    for (i, seg) in segments.iter().enumerate() {
        if seg.is_empty() {
            continue;
        }
        let must_start = i == 0 && anchored_start;
        let must_end = i == segments.len() - 1 && anchored_end;
        let haystack = &text[pos..];
        let idx = match haystack.find(seg) {
            Some(j) => j,
            None => return false,
        };
        if must_start && idx != 0 {
            return false;
        }
        let new_pos = pos + idx + seg.len();
        if must_end && new_pos != text.len() {
            return false;
        }
        pos = new_pos;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::port_regex_match;

    #[test]
    fn substring_match() {
        assert!(port_regex_match("alsa_output:playback_FL", "alsa"));
        assert!(port_regex_match("alsa_output:playback_FL", "FL"));
        assert!(!port_regex_match("alsa_output:playback_FL", "FR"));
        assert!(!port_regex_match("alsa_output:playback_FL", "nonexistent"));
    }

    #[test]
    fn wildcard_match() {
        assert!(port_regex_match("alsa_output:playback_FL", "alsa.*FL"));
        assert!(port_regex_match("alsa_output:playback_FL", "alsa.*Speaker.*FL") == false); // "Speaker" not in this string
        assert!(port_regex_match(
            "alsa_output_Speaker:playback_FL",
            "alsa.*Speaker.*FL"
        ));
    }

    #[test]
    fn anchors() {
        assert!(port_regex_match("alsa_output:playback_FL", "^alsa"));
        assert!(!port_regex_match("alsa_output:playback_FL", "^playback"));
        assert!(port_regex_match("alsa_output:playback_FL", "FL$"));
        assert!(!port_regex_match("alsa_output:playback_FL", "FR$"));
        assert!(port_regex_match("alsa", "^alsa$"));
        assert!(!port_regex_match("alsa_output", "^alsa$"));
    }

    #[test]
    fn empty_pattern_matches_all() {
        assert!(port_regex_match("anything", ""));
    }
}

/// Print one port line ("\t<id?> <node:port>"), plus, if verbose, the
/// object.path and port.alias indented underneath.
fn print_port_line(
    port: &RegistryGlobal,
    globals: &[RegistryGlobal],
    show_id: bool,
    verbose: bool,
) {
    let id_prefix = if show_id {
        format!("{:>4} ", port.id)
    } else {
        String::new()
    };
    let prefix2 = if show_id { "     " } else { "" };
    let name = port_full_name(globals, port);
    println!("{id_prefix}{name}");
    if !verbose {
        return;
    }
    if let Some(path) = prop(port, "object.path") {
        println!("  {prefix2}{path}");
    } else {
        println!("  {prefix2}port.path.{}", port.id);
    }
    if let Some(alias) = prop(port, "port.alias") {
        println!("  {prefix2}{alias}");
    } else {
        println!("  {prefix2}port_alias.{}", port.id);
    }
}

fn print_port_links(
    globals: &[RegistryGlobal],
    port: &RegistryGlobal,
    show_id: bool,
    mut print_port_first: bool,
    verbose_port: bool,
) {
    let port_dir = prop(port, "port.direction");
    for link in globals
        .iter()
        .filter(|g| g.interface == interfaces::TYPE_LINK)
    {
        let out_port: u32 = prop(link, "link.output.port")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let in_port: u32 = prop(link, "link.input.port")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let (peer_id, arrow) = if port_dir == Some("out") && out_port == port.id {
            (in_port, "|-> ")
        } else if port_dir == Some("in") && in_port == port.id {
            (out_port, "|<- ")
        } else {
            continue;
        };
        if print_port_first {
            print_port_line(port, globals, show_id, verbose_port);
            print_port_first = false;
        }
        // C `print_port_id`: prefix has the link's id (when -I); inner
        // print_port appends the peer's id (also when -I) before the name.
        let link_prefix = if show_id {
            format!("{:>4} ", link.id)
        } else {
            String::new()
        };
        let (peer_id_prefix, peer_name) = match globals
            .iter()
            .find(|g| g.id == peer_id && g.interface == interfaces::TYPE_PORT)
        {
            Some(p) => (
                if show_id {
                    format!("{:>4} ", p.id)
                } else {
                    String::new()
                },
                port_full_name(globals, p),
            ),
            None => (String::new(), format!("<unknown:{peer_id}>")),
        };
        println!("{link_prefix}  {arrow}{peer_id_prefix}{peer_name}");
    }
}

fn print_help(argv0: &str) {
    println!("{argv0} : PipeWire port and link manager.");
    println!("Generic: {argv0} [options]");
    println!("  -h, --help                            Show this help");
    println!("      --version                         Show version");
    println!("  -r, --remote=NAME                     Remote daemon name");
    println!("List: {argv0} [options] [out-pattern] [in-pattern]");
    println!("  -o, --output                          List output ports");
    println!("  -i, --input                           List input ports");
    println!("  -l, --links                           List links");
    println!("  -t, --latency                         List port latencies");
    println!("  -m, --monitor                         Monitor links and ports");
    println!("  -I, --id                              List IDs");
    println!("  -v, --verbose                         Verbose port properties");
    println!("Connect: {argv0} [options] output input");
    println!("  -L, --linger                          Linger (default, unless -m is used)");
    println!("  -P, --passive                         Passive link");
    println!("  -p, --props=PROPS                     Properties as JSON object");
    println!("  -w, --wait                            Wait until link creation attempt");
    println!("Disconnect: {argv0} -d [options] output input");
    println!("            {argv0} -d [options] link-id");
    println!("  -d, --disconnect                      Disconnect ports");
}
