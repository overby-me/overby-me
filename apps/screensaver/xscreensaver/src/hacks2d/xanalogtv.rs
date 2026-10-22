//! Port of `hacks/xanalogtv.c`.
//!
//! ```text
//! xanalogtv, Copyright (c) 2003-2018 Trevor Blackwell <tlb@tlb.org>
//!
//! Permission to use, copy, modify, distribute, and sell this software and its
//! documentation for any purpose is hereby granted without fee, provided that
//! the above copyright notice appear in all copies and that both that
//! copyright notice and this permission notice appear in supporting
//! documentation.  No representations are made about the suitability of this
//! software for any purpose.  It is provided "as is" without express or
//! implied warranty.
//!
//! Simulate test patterns on an analog TV. Concept similar to xteevee
//! in this distribution, but a totally different implementation based
//! on the simulation of an analog TV set in utils/analogtv.c. Much
//! more realistic, but needs more video card bandwidth.
//!
//! It flips around through simulated channels 2 through 13. Some show
//! pictures from your images directory, some show color bars, and some
//! just have static. Some channels receive two stations simultaneously
//! so you see a ghostly, misaligned image.
//! ```
//!
//! This is the saver [`crate::runtime::analogtv`] was written for, and the only
//! one that uses the whole of it. Six stations are put on the air, each with
//! its own picture: the SMPTE bars with the time of day on them, the test cards
//! a station used to leave up overnight, and whatever photograph the host
//! supplies. Twelve channels then tune between them at random strengths, and
//! two of the stations can land on the same channel, which is what a ghost is.
//!
//! Nothing on screen is drawn. Everything is put into a signal and received.

#[cfg(not(target_arch = "wasm32"))]
use crate::runtime::Saver;
use crate::runtime::analogtv::{self, AnalogTv, Input, Reception, TvFont};
use crate::runtime::{
    About, Dpy, ImageLoad, Opt, Pixmap, Runner, SaverDef, Screenhack, StartArgs, XEvent, XImage,
    frand, png, random, random_below, screenhack_event_helper,
};

/// Channels 2 through 13 on VHF.
const N_CHANNELS: usize = 12;
/// How many stations one channel can receive at once. Two is a ghost.
const MAX_MULTICHAN: usize = 2;
const MAX_STATIONS: usize = 6;

/// What one channel is tuned to: up to two stations, how much snow, and how
/// long to stay on it.
#[derive(Default)]
struct ChanSetting {
    recs: [Option<(usize, Reception)>; MAX_MULTICHAN],
    noise_level: f64,
    dur: i32,
}

/// What a station is showing. The bars redraw every second because the clock on
/// them is the real time of day.
enum Programme {
    Colorbars,
    Fixed,
}

struct XAnalogTv {
    tv: AnalogTv,
    ugly_font: TvFont,
    stations: Vec<Input>,
    programmes: Vec<Programme>,
    logo: Option<(XImage, Option<Pixmap>)>,
    /// The picture the host is fetching, and where it lands when it does.
    photo: Pixmap,
    photo_load: Option<ImageLoad>,
    photo_station: Option<usize>,
    curinputi: usize,
    change_ticks: i32,
    chansettings: Vec<ChanSetting>,
    change_now: i32,
    colorbars_only_p: bool,
    /// Whole milliseconds since the saver started, which is what upstream
    /// measures channel dwell times in.
    basetime: f64,
}

impl XAnalogTv {
    /// The SMPTE colour bars, with the set's name and the time on them.
    ///
    /// Upstream redraws these once a second through the station's `updater`
    /// callback, which is how the clock ticks; here the caller does it, which
    /// is the same thing without the function pointer.
    fn update_smpte_colorbars(&mut self, station: usize, d: &Dpy) {
        // Luma, chroma and phase for each of the seven bars, from the partial
        // spec at broadcastengineering.com.
        const TOP_CB: [[f64; 3]; 7] = [
            [75.0, 0.0, 0.0],    /* gray */
            [69.0, 31.0, 167.0], /* yellow */
            [56.0, 44.0, 283.5], /* cyan */
            [48.0, 41.0, 240.5], /* green */
            [36.0, 41.0, 60.5],  /* magenta */
            [28.0, 44.0, 103.5], /* red */
            [15.0, 31.0, 347.0], /* blue */
        ];
        const MID_CB: [[f64; 3]; 7] = [
            [15.0, 31.0, 347.0], /* blue */
            [7.0, 0.0, 0.0],     /* black */
            [36.0, 41.0, 60.5],  /* magenta */
            [7.0, 0.0, 0.0],     /* black */
            [56.0, 44.0, 283.5], /* cyan */
            [7.0, 0.0, 0.0],     /* black */
            [75.0, 0.0, 0.0],    /* gray */
        ];

        let black_ntsc = analogtv::lcp_to_ntsc(0.0, 0.0, 0.0);
        let input = &mut self.stations[station];
        input.setup_sync(true, false);
        input.setup_teletext();

        for col in 0..7 {
            let (l, r) = (col as f64 / 7.0, (col + 1) as f64 / 7.0);
            let t = TOP_CB[col];
            input.draw_solid_rel_lcp(l, r, 0.00, 0.68, t[0], t[1], t[2]);
            let m = MID_CB[col];
            input.draw_solid_rel_lcp(l, r, 0.68, 0.75, m[0], m[1], m[2]);
        }

        input.draw_solid_rel_lcp(0.0, 1.0 / 6.0, 0.75, 1.00, 7.0, 40.0, 303.0); /* -I */
        input.draw_solid_rel_lcp(1.0 / 6.0, 2.0 / 6.0, 0.75, 1.00, 100.0, 0.0, 0.0); /* white */
        input.draw_solid_rel_lcp(2.0 / 6.0, 3.0 / 6.0, 0.75, 1.00, 7.0, 40.0, 33.0); /* +Q */
        input.draw_solid_rel_lcp(3.0 / 6.0, 4.0 / 6.0, 0.75, 1.00, 7.0, 0.0, 0.0); /* black */
        input.draw_solid_rel_lcp(12.0 / 18.0, 13.0 / 18.0, 0.75, 1.00, 3.0, 0.0, 0.0); /* black -4 */
        input.draw_solid_rel_lcp(13.0 / 18.0, 14.0 / 18.0, 0.75, 1.00, 7.0, 0.0, 0.0); /* black */
        input.draw_solid_rel_lcp(14.0 / 18.0, 15.0 / 18.0, 0.75, 1.00, 11.0, 0.0, 0.0); /* black +4 */
        input.draw_solid_rel_lcp(5.0 / 6.0, 1.0, 0.75, 1.00, 7.0, 0.0, 0.0); /* black */

        let mut ypos = analogtv::V as i32 / 5;
        let xpos = analogtv::VIS_START as i32 + analogtv::VIS_LEN as i32 / 2;

        // Upstream puts the machine's hostname here, which is the one thing a
        // page in a browser has no way to know. The site it is on is the
        // nearest true equivalent.
        input.draw_string_centered(&self.ugly_font, "overby.me", xpos, ypos, black_ntsc);
        ypos += self.ugly_font.char_h * 5 / 2;

        // The flame comes out blue rather than red, and that is upstream's
        // arithmetic rather than a mistake here: the modulator keys the colour
        // subcarrier to its own index, and the offset this lands at is two
        // samples out of four, which is half a cycle, which is the opposite
        // hue. It works out the same at every window size, since the offset is
        // a fraction of the width both times.
        if let Some((logo, mask)) = &self.logo {
            let w2 = (f64::from(d.width()) * 0.2) as i32;
            let h2 = (f64::from(d.height()) * 0.2) as i32;
            self.tv.load_ximage(
                input,
                logo,
                mask.as_ref(),
                (d.width() - w2) / 2,
                (f64::from(d.height()) * 0.28) as i32,
                w2,
                h2,
            );
        }

        ypos += 58;

        let secs = d.wall_clock() as i64;
        let stamp = format!(
            "{:02}:{:02}:{:02} ",
            (secs / 3600) % 24,
            (secs / 60) % 60,
            secs % 60
        );
        let input = &mut self.stations[station];
        input.draw_string_centered(&self.ugly_font, &stamp, xpos, ypos, black_ntsc);
    }

    /// Milliseconds since the saver started.
    fn ticks(&self, d: &Dpy) -> i32 {
        ((d.time - self.basetime) * 1000.0) as i32
    }
}

impl Screenhack for XAnalogTv {
    fn draw(&mut self, d: &mut Dpy) -> u32 {
        // A photograph, if the host is offering one, into the station that was
        // set aside for it.
        if let Some(station) = self.photo_station {
            // The loader draws into whatever drawable it is handed and answers
            // `None` once it has, whether that took a round trip or was the
            // colour bars it falls back to when nobody is offering pictures.
            let pending = self.photo_load.take();
            let mut target = std::mem::replace(&mut self.photo, Pixmap::new(1, 1));
            self.photo_load = d.load_image_into(&mut target, pending);
            self.photo = target;
            if self.photo_load.is_none() {
                let (w, h) = (self.photo.width(), self.photo.height());
                let img = self.photo.sub_image(0, 0, w, h);
                self.stations[station].setup_sync(true, random_below(20) == 0);
                self.tv
                    .load_ximage(&mut self.stations[station], &img, None, 0, 0, 0, 0);
                self.stations[station].setup_teletext();
                self.photo_station = None;
            }
        }

        let curticks = self.ticks(d);
        let auto_change = i32::from(curticks >= self.change_ticks && self.tv.powerup > 10.0);

        if self.change_now != 0 || auto_change != 0 {
            let n = N_CHANNELS as i32;
            self.curinputi =
                ((self.curinputi as i32 + self.change_now + auto_change + n) % n) as usize;
            self.change_now = 0;
            self.change_ticks = curticks + self.chansettings[self.curinputi].dur;
            /* Set channel change noise flag */
            self.tv.channel_change_cycles = 200_000;
        }

        // Redraw the bars if this channel is showing them, then drift the
        // ghosting station's carrier.
        for i in 0..MAX_MULTICHAN {
            let Some((station, _)) = self.chansettings[self.curinputi].recs[i] else {
                continue;
            };
            if matches!(self.programmes[station], Programme::Colorbars) {
                self.update_smpte_colorbars(station, d);
            }
            if let Some((_, rec)) = &mut self.chansettings[self.curinputi].recs[i] {
                let drift = rec.freqerr;
                rec.ofs = (rec.ofs + drift as usize) % analogtv::SIGNAL_LEN;
            }
        }

        self.tv.powerup = (f64::from(curticks) * 0.001) as f32;

        for r in self.chansettings[self.curinputi].recs.iter_mut().flatten() {
            r.1.update();
        }
        let cs = &self.chansettings[self.curinputi];
        let recs: Vec<(&Reception, &Input)> = cs
            .recs
            .iter()
            .flatten()
            .map(|(station, rec)| (rec, &self.stations[*station]))
            .collect();
        self.tv.draw(d.win(), cs.noise_level, &recs);

        5000
    }

    fn reshape(&mut self, _d: &mut Dpy, width: i32, height: i32) {
        self.tv.configure(width, height);
    }

    fn event(&mut self, _d: &mut Dpy, event: &XEvent) -> bool {
        match *event {
            XEvent::ButtonPress { button, .. } => {
                self.change_now = if button == 2 || button == 3 || button == 5 {
                    -1
                } else {
                    1
                };
                true
            }
            XEvent::KeyPress { key } => {
                match key {
                    ' ' | '\t' | '\r' | '\n' => self.change_now = 1,
                    '\u{8}' => self.change_now = -1,
                    _ => self.change_now = if random() & 1 != 0 { 1 } else { -1 },
                }
                true
            }
            _ => screenhack_event_helper(event),
        }
    }
}

fn init(d: &mut Dpy) -> Box<dyn Screenhack> {
    let delay = d.res.int("delay").max(1);

    let mut tv = AnalogTv::new(d.width(), d.height());
    tv.set_defaults(
        d.res.float("TVColor") as f32,
        d.res.float("TVTint") as f32,
        d.res.float("TVBrightness") as f32,
        d.res.float("TVContrast") as f32,
    );
    if random_below(4) == 0 {
        tv.tint_control += (frand(2.0) - 1.0).powi(7) as f32 * 180.0;
    }
    tv.color_control += frand(0.3) as f32;
    tv.powerup = 0.0;

    let colorbars_only_p = d.res.bool("colorbarsOnly");
    let no_colorbars_p = d.res.bool("noColorbars");

    let logo = png::decode(crate::images::LOGO_180);
    let ugly_font = TvFont::sheet_6x10().unwrap_or_else(|| TvFont::new(7, 10));

    let mut st = XAnalogTv {
        tv,
        ugly_font,
        stations: (0..MAX_STATIONS).map(|_| Input::new()).collect(),
        programmes: Vec::new(),
        logo,
        photo: Pixmap::new(analogtv::PIC_LEN as i32, analogtv::PIC_LEN as i32 * 3 / 4),
        photo_load: None,
        photo_station: None,
        curinputi: 0,
        change_ticks: 0,
        chansettings: Vec::new(),
        change_now: 0,
        colorbars_only_p,
        basetime: d.time,
    };

    // Put six stations on the air. Station 0 is always the bars; one in five of
    // the rest gets a test card, and whatever is left waits for a photograph.
    for i in 0..MAX_STATIONS {
        let bars = !no_colorbars_p && (i == 0 || st.colorbars_only_p);
        if bars {
            st.programmes.push(Programme::Colorbars);
            continue;
        }
        let card = (!no_colorbars_p && random_below(5) == 0) || st.photo_station.is_some();
        if card {
            let cards = crate::images::TESTCARDS;
            let j = random_below(cards.len() as i32) as usize;
            if let Some((img, _)) = png::decode(cards[j]) {
                st.stations[i].setup_sync(true, false);
                st.tv
                    .load_ximage(&mut st.stations[i], &img, None, 0, 0, 0, 0);
                st.stations[i].setup_teletext();
            }
            st.programmes.push(Programme::Fixed);
            continue;
        }
        // One station waits for the host's picture. Upstream loads one into
        // every free channel; there is only ever one picture to be had here, so
        // the rest take test cards above.
        st.photo_station = Some(i);
        st.stations[i].setup_sync(true, false);
        st.programmes.push(Programme::Fixed);
    }

    // Twelve channels, each tuned to one or two of those stations. A channel
    // that gets no station is the one showing nothing but snow.
    let mut last_station = 42usize;
    for _ in 0..N_CHANNELS {
        let mut cs = ChanSetting {
            noise_level: 0.06,
            dur: 1000 * delay,
            ..ChanSetting::default()
        };
        if random_below(6) == 0 {
            cs.dur = 600;
        } else {
            for slot in 0..MAX_MULTICHAN {
                let mut station;
                loop {
                    station = random_below(MAX_STATIONS as i32) as usize;
                    if station != last_station || random_below(10) == 0 {
                        break;
                    }
                }
                last_station = station;
                let mut rec = Reception {
                    level: frand(1.0).powf(3.0) * 2.0 + 0.05,
                    ofs: (random() as usize) % analogtv::SIGNAL_LEN,
                    multipath: if random_below(3) != 0 {
                        frand(1.0)
                    } else {
                        0.0
                    },
                    ..Reception::default()
                };
                if slot != 0 {
                    // Only the ghosting station gets a frequency error; on the
                    // one you are actually watching it would just look broken.
                    rec.freqerr = (frand(2.0) - 1.0) * 3.0;
                }
                let strong = rec.level > 0.3;
                cs.recs[slot] = Some((station, rec));
                if strong || random_below(4) != 0 {
                    break;
                }
            }
        }
        st.chansettings.push(cs);
    }

    st.change_ticks = st.chansettings[0].dur + 1500;
    Box::new(st)
}

const DEFAULTS: &[&str] = &[
    ".background:	        black",
    ".foreground:	        white",
    "*delay:	        5",
    "*colorbarsOnly:      False",
    "*noColorbars:        False",
    "*image:              ",
    "*TVColor:         70",
    "*TVTint:          5",
    "*TVBrightness:    2",
    "*TVContrast:    150",
];

/// The two knob defaults here come from the C rather than from
/// `hacks/config/xanalogtv.xml`, which is the one place the two disagree: the
/// XML seeds its dialog with a brightness of 3 and a contrast of 1000, and at
/// a contrast of 1000 the picture is a white rectangle. Run with no arguments,
/// which is how a screen saver is run, upstream uses the C values. The ranges
/// are the XML's.
const OPTS: &[Opt] = &[
    Opt::boolean("colorbarsOnly", "Colorbars only", "False"),
    Opt::slider("TVColor", "Color Knob", 0.0, 400.0, 5.0, 0, "70"),
    Opt::slider("TVTint", "Tint Knob", 0.0, 360.0, 5.0, 0, "5"),
    Opt::slider("TVBrightness", "Brightness Knob", -75.0, 100.0, 1.0, 0, "2"),
    Opt::slider("TVContrast", "Contrast Knob", 0.0, 1500.0, 10.0, 0, "150"),
];

pub static DEF: SaverDef = SaverDef {
    slug: "xanalogtv",
    label: "XAnalogTV",
    defaults: DEFAULTS,
    opts: OPTS,
    about: About {
        author: "Trevor Blackwell",
        year: "2003",
        video: Some("https://www.youtube.com/watch?v=VmM1KkFsry0"),
        blurb: "An old television, tuning through twelve channels of test cards and snow.",
    },
};

/// The saver's entry point, and the one function its wasm chunk exports.
pub fn start(args: StartArgs) -> Runner {
    Runner::start(&DEF, init, args)
}

#[cfg(not(target_arch = "wasm32"))]
pub static SAVER: Saver = Saver { def: &DEF, start };
