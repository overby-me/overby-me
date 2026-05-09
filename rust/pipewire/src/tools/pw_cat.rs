// pw-cat: PipeWire audio file player/recorder + the pw-play / pw-record /
// pw-midiplay / pw-midirecord / pw-midi2play / pw-midi2record / pw-sysex /
// pw-dsdplay / pw-encplay multicall variants.
//
// Phase 8 will implement actual capture/playback. For now we just emit
// the help text byte-identical to upstream so installers and probes work.

use crate::tools::common::print_version;

pub fn main(args: &[String]) -> i32 {
    // C's pw-cat basenames argv[0] (`prog = strrchr(argv[0], '/')`),
    // so the help/usage text shows just `pw-play` not the full
    // /nix/store/.../bin/pw-play path.
    let raw0 = args.first().map(String::as_str).unwrap_or("pw-cat");
    let argv0 = raw0.rsplit('/').next().unwrap_or("pw-cat");

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

/// Whether `argv0` is the master tool that shows the trailing mode flags.
/// Only `pw-cat` itself shows them; the auto-mode variants (pw-play,
/// pw-record, pw-midiplay, ...) suppress them.
fn is_cat_master(argv0: &str) -> bool {
    argv0 == "pw-cat"
}

fn print_help(argv0: &str) {
    println!("{argv0} [options] [<file>|-]");
    println!("  -h, --help                            Show this help");
    println!("      --version                         Show version");
    println!("  -v, --verbose                         Enable verbose operations");
    println!();
    println!("  -R, --remote                          Remote daemon name");
    println!("      --media-type                      Set media type (default Audio)");
    println!("      --media-category                  Set media category (default Playback)");
    println!("      --media-role                      Set media role (default Music)");
    println!(
        "      --target                          Set node target serial or name (default auto)"
    );
    println!("                                          0 means don't link");
    println!("      --latency                         Set node latency (default 100ms)");
    println!("                                          Xunit (unit = s, ms, us, ns)");
    println!("                                          or direct samples (256)");
    println!("                                          the rate is the one of the source file");
    println!("  -P  --properties                      Set node properties");
    println!();
    println!("      --rate                            Sample rate (default 48000)");
    println!("      --channels                        Number of channels (default 2)");
    println!("      --channel-map                     Channel map");
    println!(
        "                                            a channel layout: \"Stereo\", \"5.1\",... or"
    );
    println!(
        "                                            comma separated list of channel names: eg. \"FL,FR\""
    );
    println!("      --list-layouts                    List supported channel layouts");
    println!("      --list-channel-names              List supported channel maps");
    println!("      --format                          Sample format (default s16)");
    println!("      --list-formats                    List supported sample formats");
    println!("      --container                       Container format");
    println!("      --list-containers                 List supported containers and extensions");
    println!("      --volume                          Stream volume 0-1.0 (default 1.000)");
    println!("  -q  --quality                         Resampler quality (0 - 15) (default 4)");
    println!("  -a, --raw                             RAW mode");
    println!(
        "  -M, --force-midi                      Force midi format, one of \"midi\" or \"ump\", (default ump)"
    );
    println!("  -n, --sample-count COUNT              Stop after COUNT samples");
    if is_cat_master(argv0) {
        println!();
        println!("  -p, --playback                        Playback mode");
        println!("  -r, --record                          Recording mode");
        println!("  -m, --midi                            Midi mode");
        println!("  -d, --dsd                             DSD mode");
        println!("  -o, --encoded                         Encoded mode");
        println!("  -s, --sysex                           SysEx mode");
        println!("  -c, --midi-clip                       MIDI clip mode");
    }
    println!();
}
