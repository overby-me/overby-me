// Standard MIDI File parser + event pretty-printer.
//
// Mirrors the C tool stack:
//   - `src/tools/midifile.c` opens the SMF, walks tracks, returns events
//   - `src/tools/midievent.c::midi_event_dump` formats each event
//
// Used by `pw-mididump <file.mid>` with byte-identical output.

use std::fs::File;
use std::io::{self, Read};

const DEFAULT_TEMPO_USPQN: u32 = 500_000; // 120 BPM, the SMF default.

#[derive(Debug, Clone, Copy)]
pub struct FileInfo {
    pub format: u16,
    pub ntracks: u16,
    pub division: u16,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub track: u16,
    pub sec: f64,
    pub data: Vec<u8>,
    /// Offset into `data` where the meta-event payload starts (for 0xff
    /// meta events). For non-meta events this is 0.
    pub meta_offset: usize,
    /// Tempo for 0xff 0x51 events, in microseconds per quarter note.
    pub meta_tempo_uspqn: u32,
}

pub fn read_file(path: &str) -> io::Result<(FileInfo, Vec<Event>)> {
    // C's open_read does fread(buf, 14, 1, fp) for the MThd header,
    // then loops reading MTrk chunks. Mirror that bounded behavior so
    // unbounded sources like /dev/zero produce a parse error rather
    // than hanging. We cap at 16 MiB which is well above any realistic
    // SMF.
    const MAX_BYTES: usize = 16 * 1024 * 1024;
    let mut bytes = Vec::new();
    if path == "-" {
        // SMF from stdin (matches C's pw-mididump open_read for "-").
        io::Read::take(io::stdin(), MAX_BYTES as u64).read_to_end(&mut bytes)?;
    } else {
        let f = File::open(path)?;
        io::Read::take(f, MAX_BYTES as u64).read_to_end(&mut bytes)?;
    }

    parse(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[derive(Debug)]
pub struct ParseError(pub String);
impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ParseError {}

fn err<T>(msg: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError(msg.into()))
}

#[derive(Debug, Clone)]
struct Track<'a> {
    id: u16,
    body: &'a [u8],
    pos: usize,
    tick: i64,
    eof: bool,
    /// MIDI running-status byte. SMF tracks may omit the status byte for
    /// channel events when it would repeat the previous one.
    running_status: u8,
}

pub fn parse(bytes: &[u8]) -> Result<(FileInfo, Vec<Event>), ParseError> {
    if bytes.len() < 14 || &bytes[0..4] != b"MThd" {
        // Match upstream pw-mididump's `read_mthd` behavior: it returns
        // -EINVAL when the file is shorter than the MThd chunk header
        // or when the magic doesn't match. The C tool prints `%m` =
        // "Invalid argument".
        return err("Invalid argument");
    }
    let _length = be32(&bytes[4..8]);
    let format = be16(&bytes[8..10]);
    let ntracks = be16(&bytes[10..12]);
    let division = be16(&bytes[12..14]);
    let info = FileInfo {
        format,
        ntracks,
        division,
    };

    let mut tracks: Vec<Track> = Vec::with_capacity(ntracks as usize);
    let mut p = 14;
    for id in 0..ntracks {
        if p + 8 > bytes.len() || &bytes[p..p + 4] != b"MTrk" {
            // C's read_mtrk returns -EINVAL when fread can't read the
            // 8-byte MTrk header; pw-mididump prints `Invalid argument`.
            return err("Invalid argument".to_string());
        }
        let size = be32(&bytes[p + 4..p + 8]) as usize;
        let start = p + 8;
        // Upstream `midifile.c` reads exactly `size` bytes for the body.
        // Clamp to file length in case the header lies (preventing OOB).
        let end = (start + size).min(bytes.len());
        tracks.push(Track {
            id,
            body: &bytes[start..end],
            pos: 0,
            tick: 0,
            eof: false,
            running_status: 0,
        });
        p = end;
    }

    // Prime each track with its first delta-time so we know its tick.
    // An empty MTrk (size=0) has no events — mark it eof rather than
    // erroring on the varlen read.
    for t in tracks.iter_mut() {
        if t.body.is_empty() {
            t.eof = true;
            continue;
        }
        let dt = read_varlen(t)?;
        t.tick += dt as i64;
    }

    let mut tempo_uspqn = DEFAULT_TEMPO_USPQN;
    let mut tick_sec = tick_seconds(tempo_uspqn, division);
    let mut current_tick: i64 = 0;
    let mut current_sec: f64 = 0.0;

    let mut events: Vec<Event> = Vec::new();

    loop {
        // Pick the track with the smallest current `tick`.
        let next_idx = tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.eof && t.pos <= t.body.len())
            .filter(|(_, t)| t.pos < t.body.len() || t.body.is_empty())
            .min_by_key(|(_, t)| t.tick)
            .map(|(i, _)| i);

        let i = match next_idx {
            Some(i) => i,
            None => break,
        };

        // Advance the global clock to this track's tick.
        let new_tick = tracks[i].tick;
        let delta = (new_tick - current_tick).max(0) as f64;
        current_sec += delta * tick_sec;
        current_tick = new_tick;

        let id = tracks[i].id;

        let event_start = tracks[i].pos;
        let event = read_event(&mut tracks[i])?;

        match event {
            DecodedEvent::Channel { data } => {
                events.push(Event {
                    track: id,
                    sec: current_sec,
                    data,
                    meta_offset: 0,
                    meta_tempo_uspqn: 0,
                });
            }
            DecodedEvent::SysEx { data } => {
                events.push(Event {
                    track: id,
                    sec: current_sec,
                    data,
                    meta_offset: 0,
                    meta_tempo_uspqn: 0,
                });
            }
            DecodedEvent::Meta { meta_type, payload } => {
                let mut data = vec![0xff, meta_type];
                data.extend_from_slice(&payload);
                let meta_offset = 2;
                let meta_tempo = if meta_type == 0x51 && payload.len() >= 3 {
                    ((payload[0] as u32) << 16) | ((payload[1] as u32) << 8) | (payload[2] as u32)
                } else {
                    0
                };
                if meta_type == 0x51 && meta_tempo != 0 {
                    tempo_uspqn = meta_tempo;
                    tick_sec = tick_seconds(tempo_uspqn, division);
                }
                events.push(Event {
                    track: id,
                    sec: current_sec,
                    data,
                    meta_offset,
                    meta_tempo_uspqn: meta_tempo,
                });
                if meta_type == 0x2f {
                    tracks[i].eof = true;
                    continue;
                }
            }
        }

        // Read next delta-time on this track.
        if tracks[i].pos >= tracks[i].body.len() {
            tracks[i].eof = true;
            continue;
        }
        let dt = read_varlen(&mut tracks[i])?;
        tracks[i].tick += dt as i64;
        let _ = event_start;
    }

    Ok((info, events))
}

fn tick_seconds(tempo_uspqn: u32, division: u16) -> f64 {
    if division & 0x8000 != 0 {
        // SMPTE: high bit = -fps, low byte = ticks per frame.
        let fps = (-(division as i16) >> 8) as f64;
        let tpf = (division & 0xff) as f64;
        if fps > 0.0 && tpf > 0.0 {
            return 1.0 / (fps * tpf);
        }
        return 0.0;
    }
    let tpqn = (division & 0x7fff) as f64;
    if tpqn == 0.0 {
        return 0.0;
    }
    (tempo_uspqn as f64 / 1_000_000.0) / tpqn
}

fn be16(s: &[u8]) -> u16 {
    ((s[0] as u16) << 8) | (s[1] as u16)
}

fn be32(s: &[u8]) -> u32 {
    ((s[0] as u32) << 24) | ((s[1] as u32) << 16) | ((s[2] as u32) << 8) | (s[3] as u32)
}

fn read_varlen(t: &mut Track) -> Result<u32, ParseError> {
    let mut value: u32 = 0;
    loop {
        if t.pos >= t.body.len() {
            return err("unexpected EOF in varlen");
        }
        let b = t.body[t.pos];
        t.pos += 1;
        value = (value << 7) | ((b & 0x7f) as u32);
        if b & 0x80 == 0 {
            return Ok(value);
        }
    }
}

fn read_byte(t: &mut Track) -> Result<u8, ParseError> {
    if t.pos >= t.body.len() {
        return err("unexpected EOF");
    }
    let b = t.body[t.pos];
    t.pos += 1;
    Ok(b)
}

enum DecodedEvent {
    Channel { data: Vec<u8> },
    SysEx { data: Vec<u8> },
    Meta { meta_type: u8, payload: Vec<u8> },
}

fn read_event(t: &mut Track) -> Result<DecodedEvent, ParseError> {
    let mut status = read_byte(t)?;
    let starts_with_status = status & 0x80 != 0;

    if !starts_with_status {
        // Running status: reuse last channel-event status, rewind to
        // re-read the byte we just consumed as the first data byte.
        t.pos -= 1;
        if t.running_status == 0 {
            return err("running status with no prior channel status");
        }
        status = t.running_status;
    } else if status < 0xf0 {
        t.running_status = status;
    } else if status == 0xf0 || status == 0xf7 {
        // System exclusive: variable-length payload.
        let len = read_varlen(t)? as usize;
        if t.pos + len > t.body.len() {
            return err("SysEx payload truncated");
        }
        let mut data = vec![status];
        data.extend_from_slice(&t.body[t.pos..t.pos + len]);
        t.pos += len;
        return Ok(DecodedEvent::SysEx { data });
    } else if status == 0xff {
        let meta_type = read_byte(t)?;
        // Meta event needs a varlen length byte. C's midifile is tolerant
        // of truncated EOT — `FF 2F` at end-of-buffer (no length byte) is
        // accepted as End-of-Track with empty payload.
        if t.pos >= t.body.len() {
            return Ok(DecodedEvent::Meta { meta_type, payload: Vec::new() });
        }
        let len = read_varlen(t)? as usize;
        if t.pos + len > t.body.len() {
            // Truncated payload — clamp to available bytes.
            let payload = t.body[t.pos..].to_vec();
            t.pos = t.body.len();
            return Ok(DecodedEvent::Meta { meta_type, payload });
        }
        let payload = t.body[t.pos..t.pos + len].to_vec();
        t.pos += len;
        return Ok(DecodedEvent::Meta { meta_type, payload });
    } else {
        // Other system-realtime / common bytes
        t.running_status = 0;
    }

    // Channel events.
    let nybble = status & 0xf0;
    let data_count = match nybble {
        0x80 | 0x90 | 0xa0 | 0xb0 | 0xe0 => 2,
        0xc0 | 0xd0 => 1,
        _ => 0, // 0xf1..0xf6 / 0xf8..0xfe
    };
    let payload_count = match status {
        0xf1 | 0xf3 => 1,
        0xf2 => 2,
        _ => data_count,
    };
    let mut data = vec![status];
    for _ in 0..payload_count {
        data.push(read_byte(t)?);
    }
    Ok(DecodedEvent::Channel { data })
}

// ── Pretty-printer (matches src/tools/midievent.c output) ────────────────

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

const EVENT_NAMES: [&str; 9] = [
    "Text",
    "Copyright",
    "Sequence/Track Name",
    "Instrument",
    "Lyric",
    "Marker",
    "Cue Point",
    "Program Name",
    "Device (Port) Name",
];

const SMPTE_RATES: [&str; 4] = [
    "24 fps",
    "25 fps",
    "30 fps (drop frame)",
    "30 fps (non drop frame)",
];

const MAJOR_KEYS: [&str; 19] = [
    "Unknown major",
    "Fb",
    "Cb",
    "Gb",
    "Db",
    "Ab",
    "Eb",
    "Bb",
    "F",
    "C",
    "G",
    "D",
    "A",
    "E",
    "B",
    "F#",
    "C#",
    "G#",
    "Unknown major",
];

const MINOR_KEYS: [&str; 19] = [
    "Unknown minor",
    "Dbm",
    "Abm",
    "Ebm",
    "Bbm",
    "Fm",
    "Cm",
    "Gm",
    "Dm",
    "Am",
    "Em",
    "Bm",
    "F#m",
    "C#m",
    "G#m",
    "D#m",
    "A#m",
    "E#m",
    "Unknown minor",
];

const PROGRAM_NAMES: [&str; 128] = [
    "Acoustic Grand",
    "Bright Acoustic",
    "Electric Grand",
    "Honky-Tonk",
    "Electric Piano 1",
    "Electric Piano 2",
    "Harpsichord",
    "Clavinet",
    "Celesta",
    "Glockenspiel",
    "Music Box",
    "Vibraphone",
    "Marimba",
    "Xylophone",
    "Tubular Bells",
    "Dulcimer",
    "Drawbar Organ",
    "Percussive Organ",
    "Rock Organ",
    "Church Organ",
    "Reed Organ",
    "Accoridan",
    "Harmonica",
    "Tango Accordion",
    "Nylon String Guitar",
    "Steel String Guitar",
    "Electric Jazz Guitar",
    "Electric Clean Guitar",
    "Electric Muted Guitar",
    "Overdriven Guitar",
    "Distortion Guitar",
    "Guitar Harmonics",
    "Acoustic Bass",
    "Electric Bass (fingered)",
    "Electric Bass (picked)",
    "Fretless Bass",
    "Slap Bass 1",
    "Slap Bass 2",
    "Synth Bass 1",
    "Synth Bass 2",
    "Violin",
    "Viola",
    "Cello",
    "Contrabass",
    "Tremolo Strings",
    "Pizzicato Strings",
    "Orchestral Strings",
    "Timpani",
    "String Ensemble 1",
    "String Ensemble 2",
    "SynthStrings 1",
    "SynthStrings 2",
    "Choir Aahs",
    "Voice Oohs",
    "Synth Voice",
    "Orchestra Hit",
    "Trumpet",
    "Trombone",
    "Tuba",
    "Muted Trumpet",
    "French Horn",
    "Brass Section",
    "SynthBrass 1",
    "SynthBrass 2",
    "Soprano Sax",
    "Alto Sax",
    "Tenor Sax",
    "Baritone Sax",
    "Oboe",
    "English Horn",
    "Bassoon",
    "Clarinet",
    "Piccolo",
    "Flute",
    "Recorder",
    "Pan Flute",
    "Blown Bottle",
    "Skakuhachi",
    "Whistle",
    "Ocarina",
    "Lead 1 (square)",
    "Lead 2 (sawtooth)",
    "Lead 3 (calliope)",
    "Lead 4 (chiff)",
    "Lead 5 (charang)",
    "Lead 6 (voice)",
    "Lead 7 (fifths)",
    "Lead 8 (bass+lead)",
    "Pad 1 (new age)",
    "Pad 2 (warm)",
    "Pad 3 (polysynth)",
    "Pad 4 (choir)",
    "Pad 5 (bowed)",
    "Pad 6 (metallic)",
    "Pad 7 (halo)",
    "Pad 8 (sweep)",
    "FX 1 (rain)",
    "FX 2 (soundtrack)",
    "FX 3 (crystal)",
    "FX 4 (atmosphere)",
    "FX 5 (brightness)",
    "FX 6 (goblins)",
    "FX 7 (echoes)",
    "FX 8 (sci-fi)",
    "Sitar",
    "Banjo",
    "Shamisen",
    "Koto",
    "Kalimba",
    "Bagpipe",
    "Fiddle",
    "Shanai",
    "Tinkle Bell",
    "Agogo",
    "Steel Drums",
    "Woodblock",
    "Taiko Drum",
    "Melodic Tom",
    "Synth Drum",
    "Reverse Cymbal",
    "Guitar Fret Noise",
    "Breath Noise",
    "Seashore",
    "Bird Tweet",
    "Telephone Ring",
    "Helicopter",
    "Applause",
    "Gunshot",
];

fn controller_name(ctrl: u8) -> &'static str {
    match ctrl {
        0 => "Bank Select (coarse)",
        1 => "Modulation Wheel (coarse)",
        2 => "Breath controller (coarse)",
        4 => "Foot Pedal (coarse)",
        5 => "Portamento Time (coarse)",
        6 => "Data Entry (coarse)",
        7 => "Volume (coarse)",
        8 => "Balance (coarse)",
        10 => "Pan position (coarse)",
        11 => "Expression (coarse)",
        12 => "Effect Control 1 (coarse)",
        13 => "Effect Control 2 (coarse)",
        16 => "General Purpose Slider 1",
        17 => "General Purpose Slider 2",
        18 => "General Purpose Slider 3",
        19 => "General Purpose Slider 4",
        32 => "Bank Select (fine)",
        33 => "Modulation Wheel (fine)",
        34 => "Breath \t (fine)",
        36 => "Foot Pedal (fine)",
        37 => "Portamento Time (fine)",
        38 => "Data Entry (fine)",
        39 => "Volume (fine)",
        40 => "Balance (fine)",
        42 => "Pan position (fine)",
        43 => "Expression (fine)",
        44 => "Effect Control 1 (fine)",
        45 => "Effect Control 2 (fine)",
        64 => "Hold Pedal (on/off)",
        65 => "Portamento (on/off)",
        66 => "Sustenuto Pedal (on/off)",
        67 => "Soft Pedal (on/off)",
        68 => "Legato Pedal (on/off)",
        69 => "Hold 2 Pedal (on/off)",
        70 => "Sound Variation",
        71 => "Sound Timbre",
        72 => "Sound Release Time",
        73 => "Sound Attack Time",
        74 => "Sound Brightness",
        75 => "Sound Control 6",
        76 => "Sound Control 7",
        77 => "Sound Control 8",
        78 => "Sound Control 9",
        79 => "Sound Control 10",
        80 => "General Purpose Button 1 (on/off)",
        81 => "General Purpose Button 2 (on/off)",
        82 => "General Purpose Button 3 (on/off)",
        83 => "General Purpose Button 4 (on/off)",
        91 => "Effects Level",
        92 => "Tremulo Level",
        93 => "Chorus Level",
        94 => "Celeste Level",
        95 => "Phaser Level",
        96 => "Data Button increment",
        97 => "Data Button decrement",
        98 => "Non-registered Parameter (fine)",
        99 => "Non-registered Parameter (coarse)",
        100 => "Registered Parameter (fine)",
        101 => "Registered Parameter (coarse)",
        120 => "All Sound Off",
        121 => "All Controllers Off",
        122 => "Local Keyboard (on/off)",
        123 => "All Notes Off",
        124 => "Omni Mode Off",
        125 => "Omni Mode On",
        126 => "Mono Operation",
        127 => "Poly Operation",
        _ => "Unknown",
    }
}

fn note_name(n: u8) -> String {
    let octave = (n as i32) / 12 - 1;
    format!("{:>3}{}", NOTE_NAMES[(n % 12) as usize], octave)
}

fn dump_mem(out: &mut String, label: &str, data: &[u8]) {
    out.push_str(label);
    out.push_str(": ");
    for &b in data {
        out.push_str(&format!("{:02x} ", b));
    }
}

pub fn format_event(ev: &Event) -> String {
    let mut s = String::new();
    s.push_str(&format!("track:{:>2} sec:{:.6} ", ev.track, ev.sec));
    let d = &ev.data;
    if d.is_empty() {
        return s;
    }
    let st = d[0];
    match st {
        0x80..=0x8f => s.push_str(&format!(
            "Note Off   (channel {:>2}): note {}, velocity {:>3}",
            (st & 0x0f) + 1,
            note_name(d[1]),
            d[2],
        )),
        0x90..=0x9f => s.push_str(&format!(
            "Note On    (channel {:>2}): note {}, velocity {:>3}",
            (st & 0x0f) + 1,
            note_name(d[1]),
            d[2],
        )),
        0xa0..=0xaf => s.push_str(&format!(
            "Aftertouch (channel {:>2}): note {}, pressure {:>3}",
            (st & 0x0f) + 1,
            note_name(d[1]),
            d[2],
        )),
        0xb0..=0xbf => s.push_str(&format!(
            "Controller (channel {:>2}): controller {:>3} ({}), value {:>3}",
            (st & 0x0f) + 1,
            d[1],
            controller_name(d[1]),
            d[2],
        )),
        0xc0..=0xcf => s.push_str(&format!(
            "Program    (channel {:>2}): program {:>3} ({})",
            (st & 0x0f) + 1,
            d[1],
            PROGRAM_NAMES[(d[1] & 0x7f) as usize],
        )),
        0xd0..=0xdf => s.push_str(&format!(
            "Channel Pressure (channel {:>2}): pressure {:>3}",
            (st & 0x0f) + 1,
            d[1],
        )),
        0xe0..=0xef => {
            let value = (((d[2] as i32) << 7) | (d[1] as i32)) - 0x2000;
            s.push_str(&format!(
                "Pitch Bend (channel {:>2}): value {}",
                (st & 0x0f) + 1,
                value,
            ));
        }
        0xf0 | 0xf7 => dump_mem(&mut s, "SysEx", d),
        0xf1 => s.push_str(&format!(
            "MIDI Time Code Quarter Frame: type {} values {}",
            st >> 4,
            st & 0xf,
        )),
        0xf2 => s.push_str(&format!(
            "Song Position Pointer: value {}",
            ((d[1] as i32) << 7) | (d[0] as i32),
        )),
        0xf3 => s.push_str(&format!("Song Select: value {}", d[0] & 0x7f)),
        0xf6 => s.push_str("Tune Request"),
        0xf8 => s.push_str("Timing Clock"),
        0xfa => s.push_str("Start Sequence"),
        0xfb => s.push_str("Continue Sequence"),
        0xfc => s.push_str("Stop Sequence"),
        0xfe => s.push_str("Active Sensing"),
        0xff => format_meta(&mut s, ev),
        _ => dump_mem(&mut s, "Unknown", d),
    }
    s
}

fn format_meta(out: &mut String, ev: &Event) {
    out.push_str("Meta: ");
    let d = &ev.data;
    if d.len() < 2 {
        dump_mem(out, "Invalid", d);
        return;
    }
    let mtype = d[1];
    let meta = &d[ev.meta_offset..];
    match mtype {
        0x00 if meta.len() >= 2 => {
            out.push_str(&format!("Sequence Number {:>3} {:>3}", meta[0], meta[1]));
        }
        0x01..=0x09 => {
            let name = EVENT_NAMES[(mtype - 1) as usize];
            let text = std::str::from_utf8(meta).unwrap_or("");
            out.push_str(&format!("{name}: {text}"));
        }
        0x20 if !meta.is_empty() => {
            out.push_str(&format!("Channel Prefix: {:03}", meta[0]));
        }
        0x21 if !meta.is_empty() => {
            out.push_str(&format!("Midi Port: {:03}", meta[0]));
        }
        0x2f => out.push_str("End Of Track"),
        0x51 => {
            let bpm = 60_000_000.0 / ev.meta_tempo_uspqn as f64;
            out.push_str(&format!(
                "Tempo: {} microseconds per quarter note, {:.2} BPM",
                ev.meta_tempo_uspqn, bpm,
            ));
        }
        0x54 if meta.len() >= 5 => {
            out.push_str(&format!(
                "SMPTE Offset: {} {:02}:{:02}:{:02}:{:02}.{:03}",
                SMPTE_RATES[((meta[0] & 0x60) >> 5) as usize],
                meta[0] & 0x1f,
                meta[1],
                meta[2],
                meta[3],
                meta[4],
            ));
        }
        0x58 if meta.len() >= 4 => {
            let denom = 1u32 << meta[1] as u32;
            out.push_str(&format!(
                "Time Signature: {}/{}, {} clocks per click, {} notated 32nd notes per quarter note",
                meta[0], denom, meta[2], meta[3],
            ));
        }
        0x59 if meta.len() >= 2 => {
            let sf = meta[0] as i8;
            let kind = if sf > 0 { "sharps" } else { "flats" };
            let table = if meta[1] == 0 {
                &MAJOR_KEYS
            } else {
                &MINOR_KEYS
            };
            let idx = ((sf as i32) + 9).clamp(0, 18) as usize;
            out.push_str(&format!(
                "Key Signature: {} {}: {}",
                sf.abs(),
                kind,
                table[idx],
            ));
        }
        0x7f => dump_mem(out, "Sequencer", d),
        _ => dump_mem(out, "Invalid", d),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_format0_smf() {
        // MThd: format=0, ntracks=1, division=480.
        // MTrk: 8 bytes header + delta(0) NoteOn(0x90, 60, 100), delta(120) NoteOff(0x80, 60, 0),
        //       delta(0) EndOfTrack(0xff, 0x2f, 0x00)
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&6u32.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(&480u16.to_be_bytes());
        bytes.extend_from_slice(b"MTrk");

        let mut track = Vec::new();
        track.push(0x00); // delta-time 0
        track.extend_from_slice(&[0x90, 60, 100]);
        track.push(120); // delta-time 120
        track.extend_from_slice(&[0x80, 60, 0]);
        track.push(0x00);
        track.extend_from_slice(&[0xff, 0x2f, 0x00]);

        bytes.extend_from_slice(&(track.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&track);

        let (info, events) = parse(&bytes).unwrap();
        assert_eq!(info.format, 0);
        assert_eq!(info.ntracks, 1);
        assert_eq!(info.division, 480);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].data[0], 0x90);
        assert_eq!(events[1].data[0], 0x80);
        assert_eq!(events[2].data[0], 0xff);
        assert_eq!(events[2].data[1], 0x2f);
    }

    #[test]
    fn note_format() {
        // Middle C (note 60) → "  C4" (right-aligned in 3 chars + octave).
        assert_eq!(note_name(60), "  C4");
        // C# (note 61).
        assert_eq!(note_name(61), " C#4");
    }
}
