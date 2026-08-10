//! The saver registry: one lazily-loaded wasm chunk per screensaver.
//!
//! # Why this file looks the way it does
//!
//! `#[component(lazy)]` is not usable here. It hardcodes the split module name
//! to `"lazy"`, so every lazy component in the app would share one chunk and
//! opening any saver would download all of them. The `lazy_loader!` macro
//! underneath it does take a module name, so each saver declares its own.
//!
//! Each chunk exports exactly one function, the saver's own `start`, which
//! **runs** rather than returning a pointer to something the host will run.
//! That direction is what makes splitting work: the splitter follows real calls
//! out of the exported function, so the hack's code and data are reachable from
//! that chunk and nowhere else. An earlier version handed back a `SaverDef`
//! containing a constructor pointer, and every hack stayed in the main module
//! because the only thing that ever *called* it was main-resident code.
//!
//! The shared runtime (framebuffer, Xlib façade, `Runner`, the panel) stays in
//! the main module on purpose. `wasm-split` 0.7.9 never emits a shared chunk
//! (`build_split_chunks` computes an empty set), so anything reachable from two
//! split modules but not from `main` would be copied into *both*.
//!
//! For the same reason nothing here may touch `xscreensaver::all()` or
//! `xscreensaver::find()`: those name every saver's entry point, which would
//! drag the entire collection back into the main module.

use std::future::Future;
use std::pin::Pin;

use xscreensaver::runtime::{Runner, StartArgs};

type RunnerFuture = Pin<Box<dyn Future<Output = Option<Runner>>>>;

/// One saver, as the router sees it before its code is loaded.
pub struct Entry {
    pub slug: &'static str,
    pub label: &'static str,
    /// Downloads the saver's chunk if it is not already resident, then starts
    /// it. Resolves immediately once the chunk is in memory, so restarting a
    /// saver after a settings change costs nothing extra. `None` if the chunk
    /// could not be fetched.
    pub start: fn(StartArgs) -> RunnerFuture,
}

/// Declare a saver: the entry point its chunk exports, and the loader that
/// awaits it.
///
/// Without the `split` feature this compiles to a direct call, so `dx serve`
/// works normally and there is no wasm machinery in the build at all.
macro_rules! saver {
    ($slug:literal, $body:ident, $load:ident, $path:path) => {
        fn $body(args: StartArgs) -> Runner {
            $path(args)
        }

        #[cfg(feature = "split")]
        fn $load(args: StartArgs) -> RunnerFuture {
            Box::pin(async {
                // The module name is the slug, so the emitted chunk is
                // recognisable in the network tab and in the bundle.
                static MODULE: wasm_split::LazyLoader<StartArgs, Runner> =
            wasm_split::lazy_loader!(extern $slug fn $body(props: StartArgs) -> Runner);
                if MODULE.load().await {
                    MODULE.call(args).ok()
                } else {
                    None
                }
            })
        }

        #[cfg(not(feature = "split"))]
        fn $load(args: StartArgs) -> RunnerFuture {
            Box::pin(async { Some($body(args)) })
        }
    };
}

saver!(
    "anemone",
    anemone_body,
    anemone_start,
    xscreensaver::hacks2d::anemone::start
);
saver!(
    "boxfit",
    boxfit_body,
    boxfit_start,
    xscreensaver::hacks2d::boxfit::start
);
saver!(
    "braid",
    braid_body,
    braid_start,
    xscreensaver::hacks2d::braid::start
);
saver!(
    "cloudlife",
    cloudlife_body,
    cloudlife_start,
    xscreensaver::hacks2d::cloudlife::start
);
saver!(
    "coral",
    coral_body,
    coral_start,
    xscreensaver::hacks2d::coral::start
);
saver!(
    "critical",
    critical_body,
    critical_start,
    xscreensaver::hacks2d::critical::start
);
saver!(
    "cwaves",
    cwaves_body,
    cwaves_start,
    xscreensaver::hacks2d::cwaves::start
);
saver!(
    "deco",
    deco_body,
    deco_start,
    xscreensaver::hacks2d::deco::start
);
saver!(
    "cynosure",
    cynosure_body,
    cynosure_start,
    xscreensaver::hacks2d::cynosure::start
);
saver!(
    "decayscreen",
    decayscreen_body,
    decayscreen_start,
    xscreensaver::hacks2d::decayscreen::start
);
saver!(
    "deluxe",
    deluxe_body,
    deluxe_start,
    xscreensaver::hacks2d::deluxe::start
);
saver!(
    "discrete",
    discrete_body,
    discrete_start,
    xscreensaver::hacks2d::discrete::start
);
saver!(
    "fuzzyflakes",
    fuzzyflakes_body,
    fuzzyflakes_start,
    xscreensaver::hacks2d::fuzzyflakes::start
);
saver!(
    "galaxy",
    galaxy_body,
    galaxy_start,
    xscreensaver::hacks2d::galaxy::start
);
saver!(
    "greynetic",
    greynetic_body,
    greynetic_start,
    xscreensaver::hacks2d::greynetic::start
);
saver!(
    "fadeplot",
    fadeplot_body,
    fadeplot_start,
    xscreensaver::hacks2d::fadeplot::start
);
saver!(
    "fiberlamp",
    fiberlamp_body,
    fiberlamp_start,
    xscreensaver::hacks2d::fiberlamp::start
);
saver!(
    "flame",
    flame_body,
    flame_start,
    xscreensaver::hacks2d::flame::start
);
saver!(
    "forest",
    forest_body,
    forest_start,
    xscreensaver::hacks2d::forest::start
);
saver!(
    "grav",
    grav_body,
    grav_start,
    xscreensaver::hacks2d::grav::start
);
saver!(
    "halo",
    halo_body,
    halo_start,
    xscreensaver::hacks2d::halo::start
);
saver!(
    "halftone",
    halftone_body,
    halftone_start,
    xscreensaver::hacks2d::halftone::start
);
saver!(
    "helix",
    helix_body,
    helix_start,
    xscreensaver::hacks2d::helix::start
);
saver!(
    "hexadrop",
    hexadrop_body,
    hexadrop_start,
    xscreensaver::hacks2d::hexadrop::start
);
saver!(
    "hopalong",
    hopalong_body,
    hopalong_start,
    xscreensaver::hacks2d::hopalong::start
);
saver!(
    "hypercube",
    hypercube_body,
    hypercube_start,
    xscreensaver::hacks2d::hypercube::start
);
saver!(
    "ifs",
    ifs_body,
    ifs_start,
    xscreensaver::hacks2d::ifs::start
);
saver!(
    "imsmap",
    imsmap_body,
    imsmap_start,
    xscreensaver::hacks2d::imsmap::start
);
saver!(
    "julia",
    julia_body,
    julia_start,
    xscreensaver::hacks2d::julia::start
);
saver!(
    "kaleidescope",
    kaleidescope_body,
    kaleidescope_start,
    xscreensaver::hacks2d::kaleidescope::start
);
saver!(
    "laser",
    laser_body,
    laser_start,
    xscreensaver::hacks2d::laser::start
);
saver!(
    "kumppa",
    kumppa_body,
    kumppa_start,
    xscreensaver::hacks2d::kumppa::start
);
saver!(
    "lcdscrub",
    lcdscrub_body,
    lcdscrub_start,
    xscreensaver::hacks2d::lcdscrub::start
);
saver!(
    "lissie",
    lissie_body,
    lissie_start,
    xscreensaver::hacks2d::lissie::start
);
saver!(
    "moire",
    moire_body,
    moire_start,
    xscreensaver::hacks2d::moire::start
);
saver!(
    "metaballs",
    metaballs_body,
    metaballs_start,
    xscreensaver::hacks2d::metaballs::start
);
saver!(
    "moire2",
    moire2_body,
    moire2_start,
    xscreensaver::hacks2d::moire2::start
);
saver!(
    "mountain",
    mountain_body,
    mountain_start,
    xscreensaver::hacks2d::mountain::start
);
saver!(
    "munch",
    munch_body,
    munch_start,
    xscreensaver::hacks2d::munch::start
);
saver!(
    "pedal",
    pedal_body,
    pedal_start,
    xscreensaver::hacks2d::pedal::start
);
saver!(
    "popsquares",
    popsquares_body,
    popsquares_start,
    xscreensaver::hacks2d::popsquares::start
);
saver!(
    "pyro",
    pyro_body,
    pyro_start,
    xscreensaver::hacks2d::pyro::start
);
saver!(
    "rocks",
    rocks_body,
    rocks_start,
    xscreensaver::hacks2d::rocks::start
);
saver!(
    "rorschach",
    rorschach_body,
    rorschach_start,
    xscreensaver::hacks2d::rorschach::start
);

saver!(
    "rotor",
    rotor_body,
    rotor_start,
    xscreensaver::hacks2d::rotor::start
);
saver!(
    "shadebobs",
    shadebobs_body,
    shadebobs_start,
    xscreensaver::hacks2d::shadebobs::start
);
saver!(
    "sierpinski",
    sierpinski_body,
    sierpinski_start,
    xscreensaver::hacks2d::sierpinski::start
);
saver!(
    "sphere",
    sphere_body,
    sphere_start,
    xscreensaver::hacks2d::sphere::start
);
saver!(
    "spiral",
    spiral_body,
    spiral_start,
    xscreensaver::hacks2d::spiral::start
);
saver!(
    "squiral",
    squiral_body,
    squiral_start,
    xscreensaver::hacks2d::squiral::start
);
saver!(
    "starfish",
    starfish_body,
    starfish_start,
    xscreensaver::hacks2d::starfish::start
);
saver!(
    "thornbird",
    thornbird_body,
    thornbird_start,
    xscreensaver::hacks2d::thornbird::start
);
saver!(
    "triangle",
    triangle_body,
    triangle_start,
    xscreensaver::hacks2d::triangle::start
);
saver!(
    "vines",
    vines_body,
    vines_start,
    xscreensaver::hacks2d::vines::start
);
saver!(
    "truchet",
    truchet_body,
    truchet_start,
    xscreensaver::hacks2d::truchet::start
);
saver!(
    "wander",
    wander_body,
    wander_start,
    xscreensaver::hacks2d::wander::start
);
saver!(
    "whirlwindwarp",
    whirlwindwarp_body,
    whirlwindwarp_start,
    xscreensaver::hacks2d::whirlwindwarp::start
);
saver!(
    "worm",
    worm_body,
    worm_start,
    xscreensaver::hacks2d::worm::start
);
saver!(
    "xspirograph",
    xspirograph_body,
    xspirograph_start,
    xscreensaver::hacks2d::xspirograph::start
);

/// Every saver, by slug. Only the slug, the label and a function pointer live
/// here; the code behind each one arrives on demand.
pub static SAVERS: &[Entry] = &[
    Entry {
        slug: "anemone",
        label: "Anemone",
        start: anemone_start,
    },
    Entry {
        slug: "boxfit",
        label: "Box Fit",
        start: boxfit_start,
    },
    Entry {
        slug: "braid",
        label: "Braid",
        start: braid_start,
    },
    Entry {
        slug: "cloudlife",
        label: "Cloud Life",
        start: cloudlife_start,
    },
    Entry {
        slug: "coral",
        label: "Coral",
        start: coral_start,
    },
    Entry {
        slug: "critical",
        label: "Critical",
        start: critical_start,
    },
    Entry {
        slug: "cwaves",
        label: "C Waves",
        start: cwaves_start,
    },
    Entry {
        slug: "deco",
        label: "Deco",
        start: deco_start,
    },
    Entry {
        slug: "cynosure",
        label: "Cynosure",
        start: cynosure_start,
    },
    Entry {
        slug: "decayscreen",
        label: "Decay Screen",
        start: decayscreen_start,
    },
    Entry {
        slug: "deluxe",
        label: "Deluxe",
        start: deluxe_start,
    },
    Entry {
        slug: "discrete",
        label: "Discrete",
        start: discrete_start,
    },
    Entry {
        slug: "fuzzyflakes",
        label: "Fuzzy Flakes",
        start: fuzzyflakes_start,
    },
    Entry {
        slug: "galaxy",
        label: "Galaxy",
        start: galaxy_start,
    },
    Entry {
        slug: "greynetic",
        label: "Greynetic",
        start: greynetic_start,
    },
    Entry {
        slug: "fadeplot",
        label: "Fade Plot",
        start: fadeplot_start,
    },
    Entry {
        slug: "fiberlamp",
        label: "Fiber Lamp",
        start: fiberlamp_start,
    },
    Entry {
        slug: "flame",
        label: "Flame",
        start: flame_start,
    },
    Entry {
        slug: "forest",
        label: "Forest",
        start: forest_start,
    },
    Entry {
        slug: "grav",
        label: "Grav",
        start: grav_start,
    },
    Entry {
        slug: "halo",
        label: "Halo",
        start: halo_start,
    },
    Entry {
        slug: "halftone",
        label: "Halftone",
        start: halftone_start,
    },
    Entry {
        slug: "helix",
        label: "Helix",
        start: helix_start,
    },
    Entry {
        slug: "hexadrop",
        label: "Hexadrop",
        start: hexadrop_start,
    },
    Entry {
        slug: "hopalong",
        label: "Hopalong",
        start: hopalong_start,
    },
    Entry {
        slug: "hypercube",
        label: "Hypercube",
        start: hypercube_start,
    },
    Entry {
        slug: "ifs",
        label: "IFS",
        start: ifs_start,
    },
    Entry {
        slug: "imsmap",
        label: "IMS Map",
        start: imsmap_start,
    },
    Entry {
        slug: "julia",
        label: "Julia",
        start: julia_start,
    },
    Entry {
        slug: "kaleidescope",
        label: "Kaleidescope",
        start: kaleidescope_start,
    },
    Entry {
        slug: "laser",
        label: "Laser",
        start: laser_start,
    },
    Entry {
        slug: "kumppa",
        label: "Kumppa",
        start: kumppa_start,
    },
    Entry {
        slug: "lcdscrub",
        label: "LCD Scrub",
        start: lcdscrub_start,
    },
    Entry {
        slug: "lissie",
        label: "Lissie",
        start: lissie_start,
    },
    Entry {
        slug: "moire",
        label: "Moiré",
        start: moire_start,
    },
    Entry {
        slug: "metaballs",
        label: "Meta Balls",
        start: metaballs_start,
    },
    Entry {
        slug: "moire2",
        label: "Moiré 2",
        start: moire2_start,
    },
    Entry {
        slug: "mountain",
        label: "Mountain",
        start: mountain_start,
    },
    Entry {
        slug: "munch",
        label: "Munch",
        start: munch_start,
    },
    Entry {
        slug: "pedal",
        label: "Pedal",
        start: pedal_start,
    },
    Entry {
        slug: "popsquares",
        label: "Pop Squares",
        start: popsquares_start,
    },
    Entry {
        slug: "pyro",
        label: "Pyro",
        start: pyro_start,
    },
    Entry {
        slug: "rocks",
        label: "Rocks",
        start: rocks_start,
    },
    Entry {
        slug: "rorschach",
        label: "Rorschach",
        start: rorschach_start,
    },
    Entry {
        slug: "rotor",
        label: "Rotor",
        start: rotor_start,
    },
    Entry {
        slug: "shadebobs",
        label: "Shade Bobs",
        start: shadebobs_start,
    },
    Entry {
        slug: "sierpinski",
        label: "Sierpinski",
        start: sierpinski_start,
    },
    Entry {
        slug: "sphere",
        label: "Sphere",
        start: sphere_start,
    },
    Entry {
        slug: "spiral",
        label: "Spiral",
        start: spiral_start,
    },
    Entry {
        slug: "squiral",
        label: "Squiral",
        start: squiral_start,
    },
    Entry {
        slug: "starfish",
        label: "Starfish",
        start: starfish_start,
    },
    Entry {
        slug: "thornbird",
        label: "Thornbird",
        start: thornbird_start,
    },
    Entry {
        slug: "triangle",
        label: "Triangle",
        start: triangle_start,
    },
    Entry {
        slug: "vines",
        label: "Vines",
        start: vines_start,
    },
    Entry {
        slug: "truchet",
        label: "Truchet",
        start: truchet_start,
    },
    Entry {
        slug: "wander",
        label: "Wander",
        start: wander_start,
    },
    Entry {
        slug: "whirlwindwarp",
        label: "Whirlwind Warp",
        start: whirlwindwarp_start,
    },
    Entry {
        slug: "worm",
        label: "Worm",
        start: worm_start,
    },
    Entry {
        slug: "xspirograph",
        label: "XSpirograph",
        start: xspirograph_start,
    },
];

/// Look a saver up by its URL slug.
pub fn find(slug: &str) -> Option<&'static Entry> {
    SAVERS.iter().find(|e| e.slug == slug)
}

/// Pick one at random. Used by `/screensaver`, which then redirects to it.
pub fn random() -> &'static Entry {
    let i = (js_sys::Math::random() * SAVERS.len() as f64) as usize;
    &SAVERS[i.min(SAVERS.len() - 1)]
}
