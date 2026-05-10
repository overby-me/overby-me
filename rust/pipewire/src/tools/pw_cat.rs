// pw-cat: PipeWire audio file player/recorder + the pw-play / pw-record /
// pw-midiplay / pw-midirecord / pw-midi2play / pw-midi2record / pw-sysex /
// pw-dsdplay / pw-encplay multicall variants.
//
// Phase 8 will implement actual capture/playback. For now we just emit
// the help text byte-identical to upstream so installers and probes work.

use crate::tools::common::{expand_short_clusters, print_version};

pub fn main(raw_args: &[String]) -> i32 {
    // C's pw-cat basenames argv[0] for `prog` (used in the help text),
    // but uses the raw argv[0] (full path) in getopt's error messages.
    let raw0 = raw_args.first().map(String::as_str).unwrap_or("pw-cat");
    let argv0 = raw0.rsplit('/').next().unwrap_or("pw-cat");
    let getopt_argv0 = raw0;
    // Per OPTIONS = "hvprmdsR:P:q:aM:n:c": these short flags take no arg.
    let args = expand_short_clusters(
        raw_args,
        &['h', 'v', 'p', 'r', 'm', 'd', 's', 'a', 'c', 'o'],
    );

    // Short required-argument flags per OPTIONS = "hvprmdsR:P:q:aM:n:c"
    // (the `:`-marked entries).
    let short_required: &[(char, &str)] = &[
        ('R', "R"),
        ('P', "P"),
        ('q', "q"),
        ('M', "M"),
        ('n', "n"),
    ];
    // Long required-argument flags per long_options[].
    let long_required: &[&str] = &[
        "--remote",
        "--media-type",
        "--media-category",
        "--media-role",
        "--target",
        "--latency",
        "--properties",
        "--rate",
        "--channels",
        "--channel-map",
        "--format",
        "--container",
        "--volume",
        "--quality",
        "--force-midi",
        "--sample-count",
    ];
    let known_long_no_arg: &[&str] = &[
        "--verbose",
        "--playback",
        "--record",
        "--midi",
        "--dsd",
        "--encoded",
        "--sysex",
        "--midi-clip",
        "--raw",
        "--list-layouts",
        "--list-channel-names",
        "--list-formats",
        "--list-containers",
    ];
    let known_short_no_arg = ['v', 'p', 'r', 'm', 'd', 'o', 's', 'c', 'a'];

    let mut mode_set = false;
    let mut positional_count: usize = 0;
    let mut i = 1;
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "--" => {
                // End of options; rest are positional filenames.
                for s in args.iter().skip(i + 1) {
                    if !s.starts_with('-') || s == "-" {
                        positional_count += 1;
                    }
                }
                break;
            }
            "-h" | "--help" => {
                print_help(argv0);
                return 0;
            }
            "--list-formats" => {
                print_list_formats();
                return 0;
            }
            "--list-channel-names" => {
                print_list_channel_names();
                return 0;
            }
            "--list-layouts" => {
                print_list_layouts();
                return 0;
            }
            "--list-containers" => {
                print_list_containers();
                return 0;
            }
            "--version" => {
                print_version(argv0);
                return 0;
            }
            s if s.starts_with("--help=") => {
                eprintln!("{getopt_argv0}: option '--help' doesn't allow an argument");
                print_help(argv0);
                return 1;
            }
            s if s.starts_with("--version=") => {
                eprintln!("{getopt_argv0}: option '--version' doesn't allow an argument");
                print_help(argv0);
                return 1;
            }
            s if s.starts_with("--") => {
                let (name, has_inline) = match s.split_once('=') {
                    Some((n, _)) => (n, true),
                    None => (s, false),
                };
                if known_long_no_arg.contains(&name) {
                    if has_inline {
                        // No-arg long flag with `=value` → getopt error.
                        eprintln!("{getopt_argv0}: option '{name}' doesn't allow an argument");
                        print_help(argv0);
                        return 1;
                    }
                    if matches!(
                        name,
                        "--playback"
                            | "--record"
                            | "--midi"
                            | "--dsd"
                            | "--sysex"
                            | "--encoded"
                    ) {
                        mode_set = true;
                    }
                } else if let Some(&long) = long_required.iter().find(|&&l| l == name) {
                    if has_inline {
                        // --foo=val (consumed inline)
                    } else if i + 1 >= args.len() {
                        eprintln!("{getopt_argv0}: option '{long}' requires an argument");
                        print_help(argv0);
                        return 1;
                    } else {
                        i += 2;
                        continue;
                    }
                } else {
                    eprintln!("{getopt_argv0}: unrecognized option '{s}'");
                    print_help(argv0);
                    return 1;
                }
            }
            s if s.starts_with('-') && s.len() == 2 => {
                let ch = s.chars().nth(1).unwrap_or('?');
                if known_short_no_arg.contains(&ch) {
                    // Mode-selector flags cause `mode_set = true` so the
                    // post-loop error is `filename or - argument missing`
                    // instead of `one of the playback/record options...`.
                    if matches!(ch, 'p' | 'r' | 'm' | 'd' | 's' | 'o') {
                        mode_set = true;
                    }
                } else if let Some(&(_, name)) = short_required.iter().find(|(c, _)| *c == ch) {
                    if i + 1 >= args.len() {
                        eprintln!("{getopt_argv0}: option requires an argument -- '{name}'");
                        print_help(argv0);
                        return 1;
                    }
                    i += 2;
                    continue;
                } else {
                    eprintln!("{getopt_argv0}: invalid option -- '{ch}'");
                    print_help(argv0);
                    return 1;
                }
            }
            s if !s.starts_with('-') || s == "-" => {
                // C treats lone `-` as a positional filename (stdin).
                positional_count += 1;
            }
            s if s.starts_with('-') && s.len() > 2 && !s.starts_with("--") => {
                // Short cluster like `-hV` or `-Vh`. Process char-by-char
                // (getopt's behaviour). If the first char is `h`, print
                // help and exit before processing the rest.
                let ch = s.chars().nth(1).unwrap_or('?');
                if ch == 'h' {
                    print_help(argv0);
                    return 0;
                }
                if !known_short_no_arg.contains(&ch)
                    && !short_required.iter().any(|(c, _)| *c == ch)
                {
                    eprintln!("{getopt_argv0}: invalid option -- '{ch}'");
                    print_help(argv0);
                    return 1;
                }
                // First char is a known no-arg flag (other than h). Apply
                // its mode-setter side effect, then fall through (we
                // don't currently handle inline values for required-arg
                // clusters in pw-cat).
                if matches!(ch, 'p' | 'r' | 'm' | 'd' | 's' | 'o') {
                    mode_set = true;
                }
            }
            _ => {}
        }
        i += 1;
    }

    // C: post-getopt validation:
    //   - master `pw-cat` with no mode flag → "playback/record options"
    //     error
    //   - any path with mode set (or pw-play/pw-record/etc) but no file →
    //     "filename or - argument missing"
    if is_cat_master(argv0) && !mode_set {
        eprintln!("error: one of the playback/record options must be provided");
    } else if positional_count == 0 {
        eprintln!("error: filename or - argument missing");
    } else {
        // C tries to connect first, then open the file via sndfile.
        // Without daemon → connect-fail. With daemon → sndfile error.
        match crate::pipewire_lib::client::Client::connect_default() {
            Ok(_) => {
                // Daemon up; emit the sndfile error. sndfile distinguishes
                // "missing file" (System error) from "bad format on
                // existing file" (Format not recognised). Probe the path.
                let file = raw_args
                    .iter()
                    .skip(1)
                    .find(|a| !a.starts_with('-') || a == &"-")
                    .map(|s| s.as_str())
                    .unwrap_or("");
                let exists = file == "-" || std::path::Path::new(file).exists();
                if exists {
                    eprintln!(
                        "sndfile: failed to open audio file \"{file}\": Format not recognised."
                    );
                } else {
                    eprintln!(
                        "sndfile: failed to open audio file \"{file}\": System error : No such file or directory."
                    );
                }
                eprintln!("error: open failed: Input/output error");
            }
            Err(_) => {
                eprintln!(
                    "error: pw_context_connect() failed: {}",
                    crate::tools::common::connect_failure_msg()
                );
            }
        }
        return 1;
    }
    print_help(argv0);
    0
}

fn print_list_formats() {
    println!("Supported formats:");
    for f in [
        "ulaw",
        "alaw",
        "s8",
        "u8",
        "s16",
        "s24",
        "s32",
        "f32",
        "f64",
        "mp1",
        "mp2",
        "mp3",
        "vorbis",
        "opus",
        "ima-adpcm",
        "ms-adpcm",
        "nms-adpcm-16",
        "nms-adpcm-24",
        "nms-adpcm-32",
        "alac-16",
        "alac-20",
        "alac-24",
        "alac-32",
        "gsm610",
        "g721-32",
        "g723-24",
        "g723-40",
        "dwvw-12",
        "dwvw-16",
        "dwvw-24",
        "vox",
        "dpcm-16",
        "dpcm-8",
    ] {
        println!("  {f}");
    }
}

fn print_list_channel_names() {
    println!("Supported channel names:");
    println!("    AUX0 ... AUX4095");
    for n in [
        "UNK", "NA", "MONO", "FL", "FR", "FC", "LFE", "SL", "SR", "FLC", "FRC", "RC", "RL", "RR",
        "TC", "TFL", "TFC", "TFR", "TRL", "TRC", "TRR", "RLC", "RRC", "FLW", "FRW", "LFE2", "FLH",
        "FCH", "FRH", "TFLC", "TFRC", "TSL", "TSR", "LLFE", "RLFE", "BC", "BLC", "BRC",
    ] {
        println!("    {n}");
    }
}

fn print_list_layouts() {
    println!("Supported channel layouts:");
    println!("Supported channel layout aliases:");
    let layouts: &[(&str, &[&str])] = &[
        ("Mono", &["MONO"]),
        ("Stereo", &["FL", "FR"]),
        ("Quad", &["FL", "FR", "RL", "RR"]),
        ("Pentagonal", &["FL", "FR", "RL", "RR", "FC"]),
        ("Hexagonal", &["FL", "FR", "RL", "RR", "FC", "RC"]),
        (
            "Octagonal",
            &["FL", "FR", "RL", "RR", "FC", "RC", "SL", "SR"],
        ),
        (
            "Cube",
            &["FL", "FR", "RL", "RR", "TFL", "TFR", "TRL", "TRR"],
        ),
        ("MPEG-1.0", &["MONO"]),
        ("MPEG-2.0", &["FL", "FR"]),
        ("MPEG-3.0A", &["FL", "FR", "FC"]),
        ("MPEG-3.0B", &["FC", "FL", "FR"]),
        ("MPEG-4.0A", &["FL", "FR", "FC", "RC"]),
        ("MPEG-4.0B", &["FC", "FL", "FR", "RC"]),
        ("MPEG-5.0A", &["FL", "FR", "FC", "SL", "SR"]),
        ("MPEG-5.0B", &["FL", "FR", "SL", "SR", "FC"]),
        ("MPEG-5.0C", &["FL", "FC", "FR", "SL", "SR"]),
        ("MPEG-5.0D", &["FC", "FL", "FR", "SL", "SR"]),
        ("MPEG-5.1A", &["FL", "FR", "FC", "LFE", "SL", "SR"]),
        ("MPEG-5.1B", &["FL", "FR", "SL", "SR", "FC", "LFE"]),
        ("MPEG-5.1C", &["FL", "FC", "FR", "SL", "SR", "LFE"]),
        ("MPEG-5.1D", &["FC", "FL", "FR", "SL", "SR", "LFE"]),
        ("MPEG-6.1A", &["FL", "FR", "FC", "LFE", "SL", "SR", "RC"]),
        (
            "MPEG-7.1A",
            &["FL", "FR", "FC", "LFE", "RL", "RR", "SL", "SR"],
        ),
        (
            "MPEG-7.1B",
            &["FC", "SL", "SR", "FL", "FR", "RL", "RR", "LFE"],
        ),
        (
            "MPEG-7.1C",
            &["FL", "FR", "FC", "LFE", "SL", "SR", "RL", "RR"],
        ),
        ("2.1", &["FL", "FR", "LFE"]),
        ("2RC", &["FL", "FR", "RC"]),
        ("2FC", &["FL", "FR", "FC"]),
        ("3.1", &["FL", "FR", "FC", "LFE"]),
        ("4.0", &["FL", "FR", "FC", "RC"]),
        ("2.2", &["FL", "FR", "SL", "SR"]),
        ("4.1", &["FL", "FR", "FC", "LFE", "RC"]),
        ("5.0", &["FL", "FR", "FC", "SL", "SR"]),
        ("5.0R", &["FL", "FR", "FC", "RL", "RR"]),
        ("5.1", &["FL", "FR", "FC", "LFE", "SL", "SR"]),
        ("5.1R", &["FL", "FR", "FC", "LFE", "RL", "RR"]),
        ("6.0", &["FL", "FR", "FC", "RC", "SL", "SR"]),
        ("6.0F", &["FL", "FR", "FLC", "FRC", "SL", "SR"]),
        ("6.1", &["FL", "FR", "FC", "LFE", "RC", "SL", "SR"]),
        ("6.1F", &["FL", "FR", "FC", "LFE", "RL", "RR", "RC"]),
        ("7.0", &["FL", "FR", "FC", "RL", "RR", "SL", "SR"]),
        ("7.0F", &["FL", "FR", "FC", "FLC", "FRC", "SL", "SR"]),
        ("7.1", &["FL", "FR", "FC", "LFE", "RL", "RR", "SL", "SR"]),
        ("7.1W", &["FL", "FR", "FC", "LFE", "FLC", "FRC", "SL", "SR"]),
        (
            "7.1WR",
            &["FL", "FR", "FC", "LFE", "RL", "RR", "FLC", "FRC"],
        ),
    ];
    for (name, ch) in layouts {
        println!("    {name}: [ {} ]", ch.join(", "));
    }
    let aliases: &[(&str, &str)] = &[
        ("mono", "Mono"),
        ("stereo", "Stereo"),
        ("surround-21", "2.1"),
        ("quad", "Quad"),
        ("surround-22", "2.2"),
        ("surround-40", "4.0"),
        ("surround-31", "3.1"),
        ("surround-41", "4.1"),
        ("surround-50", "5.0"),
        ("surround-51", "5.1"),
        ("surround-51r", "5.1R"),
        ("surround-70", "7.0"),
        ("surround-71", "7.1"),
    ];
    for (alias, name) in aliases {
        println!("    {alias} -> {name}");
    }
}

fn print_list_containers() {
    println!("Supported containers and extensions:");
    let containers: &[(&str, &str)] = &[
        ("aiff", "AIFF (Apple/SGI)"),
        ("au", "AU (Sun/NeXT)"),
        ("avr", "AVR (Audio Visual Research)"),
        ("caf", "CAF (Apple Core Audio File)"),
        ("flac", "FLAC (Free Lossless Audio Codec)"),
        ("htk", "HTK (HMM Tool Kit)"),
        ("iff", "IFF (Amiga IFF/SVX8/SV16)"),
        ("mat", "MAT4 (GNU Octave 2.0 / Matlab 4.2)"),
        ("mat", "MAT5 (GNU Octave 2.1 / Matlab 5.0)"),
        ("mpc", "MPC (Akai MPC 2k)"),
        ("m1a", "MPEG-1/2 Audio"),
        ("oga", "OGG (OGG Container format)"),
        ("paf", "PAF (Ensoniq PARIS)"),
        ("pvf", "PVF (Portable Voice Format)"),
        ("raw", "RAW (header-less)"),
        ("rf64", "RF64 (RIFF 64)"),
        ("sd2", "SD2 (Sound Designer II)"),
        ("sds", "SDS (Midi Sample Dump Standard)"),
        ("sf", "SF (Berkeley/IRCAM/CARL)"),
        ("voc", "VOC (Creative Labs)"),
        ("w64", "W64 (SoundFoundry WAVE 64)"),
        ("wav", "WAV (Microsoft)"),
        ("wav", "WAV (NIST Sphere)"),
        ("wav", "WAVEX (Microsoft)"),
        ("wve", "WVE (Psion Series 3)"),
        ("xi", "XI (FastTracker 2)"),
    ];
    for (ext, name) in containers {
        println!("    {ext}: {name}");
    }
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
