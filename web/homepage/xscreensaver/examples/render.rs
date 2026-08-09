//! Render a saver to a PPM file, for looking at a port with your own eyes.
//!
//! The automated tests can tell you a hack drew something, kept changing and
//! did not panic. They cannot tell you it looks like the original. This is how
//! you check that: render some frames and compare against the upstream hack (or
//! against its video link in `SaverDef::about`).
//!
//! ```sh
//! cargo run --example render -- munch 640 480 400 /tmp/munch.ppm
//! cargo run --example render -- rorschach 640 480 900 /tmp/r.ppm ysymmetry=true
//! magick /tmp/munch.ppm /tmp/munch.png
//! ```
//!
//! PPM because it keeps this crate dependency-free; any image tool reads it.

use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 6 {
        eprintln!(
            "usage: {} <slug> <width> <height> <frames> <out.ppm> [query]\n\navailable: {}",
            args[0],
            xscreensaver::all()
                .iter()
                .map(|s| s.def.slug)
                .collect::<Vec<_>>()
                .join(" ")
        );
        std::process::exit(2);
    }

    fn number<T: std::str::FromStr>(what: &str, raw: &str) -> T {
        match raw.parse() {
            Ok(v) => v,
            Err(_) => {
                eprintln!("{what} must be a number, got {raw:?}");
                std::process::exit(2);
            }
        }
    }

    let slug = &args[1];
    let width: i32 = number("width", &args[2]);
    let height: i32 = number("height", &args[3]);
    let frames: usize = number("frames", &args[4]);
    let out = &args[5];
    let query = args.get(6).cloned().unwrap_or_default();

    let Some(saver) = xscreensaver::find(slug) else {
        eprintln!("no such saver: {slug}");
        std::process::exit(1);
    };

    // A fixed seed, so re-running after an edit shows what the edit changed and
    // nothing else.
    let mut runner = (saver.start)(xscreensaver::runtime::StartArgs::new(
        width, height, &query, 20260809,
    ));
    for _ in 0..frames {
        runner.step();
    }

    let fb = runner.dpy.win_ref();
    let mut ppm = Vec::with_capacity((fb.width() * fb.height() * 3 + 32) as usize);
    let _ = write!(ppm, "P6\n{} {}\n255\n", fb.width(), fb.height());
    for p in fb.pixels() {
        let (r, g, b) = xscreensaver::runtime::color::unrgb(*p);
        ppm.extend_from_slice(&[r, g, b]);
    }
    if let Err(e) = std::fs::write(out, ppm) {
        eprintln!("could not write {out}: {e}");
        std::process::exit(1);
    }
    eprintln!("{slug}: {frames} frames at {width}x{height} -> {out}");
}
