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
    "anemotaxis",
    anemotaxis_body,
    anemotaxis_start,
    xscreensaver::hacks2d::anemotaxis::start
);
saver!(
    "apollonian",
    apollonian_body,
    apollonian_start,
    xscreensaver::hacks2d::apollonian::start
);
saver!(
    "binaryhorizon",
    binaryhorizon_body,
    binaryhorizon_start,
    xscreensaver::hacks2d::binaryhorizon::start
);
saver!(
    "binaryring",
    binaryring_body,
    binaryring_start,
    xscreensaver::hacks2d::binaryring::start
);
saver!(
    "blitspin",
    blitspin_body,
    blitspin_start,
    xscreensaver::hacks2d::blitspin::start
);
saver!(
    "bouboule",
    bouboule_body,
    bouboule_start,
    xscreensaver::hacks2d::bouboule::start
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
    "bumps",
    bumps_body,
    bumps_start,
    xscreensaver::hacks2d::bumps::start
);
saver!(
    "ccurve",
    ccurve_body,
    ccurve_start,
    xscreensaver::hacks2d::ccurve::start
);
saver!(
    "cloudlife",
    cloudlife_body,
    cloudlife_start,
    xscreensaver::hacks2d::cloudlife::start
);
saver!(
    "compass",
    compass_body,
    compass_start,
    xscreensaver::hacks2d::compass::start
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
    "demon",
    demon_body,
    demon_start,
    xscreensaver::hacks2d::demon::start
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
    "distort",
    distort_body,
    distort_start,
    xscreensaver::hacks2d::distort::start
);
saver!(
    "drift",
    drift_body,
    drift_start,
    xscreensaver::hacks2d::drift::start
);
saver!(
    "droste",
    droste_body,
    droste_start,
    xscreensaver::hacks2d::droste::start
);
saver!(
    "epicycle",
    epicycle_body,
    epicycle_start,
    xscreensaver::hacks2d::epicycle::start
);
saver!(
    "euler2d",
    euler2d_body,
    euler2d_start,
    xscreensaver::hacks2d::euler2d::start
);
saver!(
    "eruption",
    eruption_body,
    eruption_start,
    xscreensaver::hacks2d::eruption::start
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
    "fireworkx",
    fireworkx_body,
    fireworkx_start,
    xscreensaver::hacks2d::fireworkx::start
);
saver!(
    "flame",
    flame_body,
    flame_start,
    xscreensaver::hacks2d::flame::start
);
saver!(
    "fluidballs",
    fluidballs_body,
    fluidballs_start,
    xscreensaver::hacks2d::fluidballs::start
);
saver!(
    "forest",
    forest_body,
    forest_start,
    xscreensaver::hacks2d::forest::start
);
saver!(
    "goop",
    goop_body,
    goop_start,
    xscreensaver::hacks2d::goop::start
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
    "interaggregate",
    interaggregate_body,
    interaggregate_start,
    xscreensaver::hacks2d::interaggregate::start
);
saver!(
    "interference",
    interference_body,
    interference_start,
    xscreensaver::hacks2d::interference::start
);
saver!(
    "intermomentary",
    intermomentary_body,
    intermomentary_start,
    xscreensaver::hacks2d::intermomentary::start
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
    "lightning",
    lightning_body,
    lightning_start,
    xscreensaver::hacks2d::lightning::start
);
saver!(
    "lisa",
    lisa_body,
    lisa_start,
    xscreensaver::hacks2d::lisa::start
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
    "lmorph",
    lmorph_body,
    lmorph_start,
    xscreensaver::hacks2d::lmorph::start
);
saver!(
    "marbling",
    marbling_body,
    marbling_start,
    xscreensaver::hacks2d::marbling::start
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
    "petri",
    petri_body,
    petri_start,
    xscreensaver::hacks2d::petri::start
);
saver!(
    "piecewise",
    piecewise_body,
    piecewise_start,
    xscreensaver::hacks2d::piecewise::start
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
    "qix",
    qix_body,
    qix_start,
    xscreensaver::hacks2d::qix::start
);
saver!(
    "rdbomb",
    rdbomb_body,
    rdbomb_start,
    xscreensaver::hacks2d::rdbomb::start
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
    "scooter",
    scooter_body,
    scooter_start,
    xscreensaver::hacks2d::scooter::start
);
saver!(
    "shadebobs",
    shadebobs_body,
    shadebobs_start,
    xscreensaver::hacks2d::shadebobs::start
);
saver!(
    "rotzoomer",
    rotzoomer_body,
    rotzoomer_start,
    xscreensaver::hacks2d::rotzoomer::start
);
saver!(
    "sierpinski",
    sierpinski_body,
    sierpinski_start,
    xscreensaver::hacks2d::sierpinski::start
);
saver!(
    "slidescreen",
    slidescreen_body,
    slidescreen_start,
    xscreensaver::hacks2d::slidescreen::start
);
saver!(
    "slip",
    slip_body,
    slip_start,
    xscreensaver::hacks2d::slip::start
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
    "spotlight",
    spotlight_body,
    spotlight_start,
    xscreensaver::hacks2d::spotlight::start
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
    "substrate",
    substrate_body,
    substrate_start,
    xscreensaver::hacks2d::substrate::start
);
saver!(
    "t3d",
    t3d_body,
    t3d_start,
    xscreensaver::hacks2d::t3d::start
);
saver!(
    "tessellimage",
    tessellimage_body,
    tessellimage_start,
    xscreensaver::hacks2d::tessellimage::start
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
    "twang",
    twang_body,
    twang_start,
    xscreensaver::hacks2d::twang::start
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
    "whirlygig",
    whirlygig_body,
    whirlygig_start,
    xscreensaver::hacks2d::whirlygig::start
);
saver!(
    "worm",
    worm_body,
    worm_start,
    xscreensaver::hacks2d::worm::start
);
saver!(
    "wormhole",
    wormhole_body,
    wormhole_start,
    xscreensaver::hacks2d::wormhole::start
);
saver!(
    "zoom",
    zoom_body,
    zoom_start,
    xscreensaver::hacks2d::zoom::start
);
saver!(
    "xflame",
    xflame_body,
    xflame_start,
    xscreensaver::hacks2d::xflame::start
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
        slug: "anemotaxis",
        label: "Anemotaxis",
        start: anemotaxis_start,
    },
    Entry {
        slug: "apollonian",
        label: "Apollonian",
        start: apollonian_start,
    },
    Entry {
        slug: "binaryhorizon",
        label: "Binary Horizon",
        start: binaryhorizon_start,
    },
    Entry {
        slug: "binaryring",
        label: "Binary Ring",
        start: binaryring_start,
    },
    Entry {
        slug: "blitspin",
        label: "Blit Spin",
        start: blitspin_start,
    },
    Entry {
        slug: "bouboule",
        label: "Bouboule",
        start: bouboule_start,
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
        slug: "bumps",
        label: "Bumps",
        start: bumps_start,
    },
    Entry {
        slug: "ccurve",
        label: "C Curve",
        start: ccurve_start,
    },
    Entry {
        slug: "cloudlife",
        label: "Cloud Life",
        start: cloudlife_start,
    },
    Entry {
        slug: "compass",
        label: "Compass",
        start: compass_start,
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
        slug: "demon",
        label: "Demon",
        start: demon_start,
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
        slug: "distort",
        label: "Distort",
        start: distort_start,
    },
    Entry {
        slug: "drift",
        label: "Drift",
        start: drift_start,
    },
    Entry {
        slug: "droste",
        label: "Droste",
        start: droste_start,
    },
    Entry {
        slug: "epicycle",
        label: "Epicycle",
        start: epicycle_start,
    },
    Entry {
        slug: "euler2d",
        label: "Euler 2D",
        start: euler2d_start,
    },
    Entry {
        slug: "eruption",
        label: "Eruption",
        start: eruption_start,
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
        slug: "fireworkx",
        label: "Fireworkx",
        start: fireworkx_start,
    },
    Entry {
        slug: "flame",
        label: "Flame",
        start: flame_start,
    },
    Entry {
        slug: "fluidballs",
        label: "Fluid Balls",
        start: fluidballs_start,
    },
    Entry {
        slug: "forest",
        label: "Forest",
        start: forest_start,
    },
    Entry {
        slug: "goop",
        label: "Goop",
        start: goop_start,
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
        slug: "interaggregate",
        label: "Interaggregate",
        start: interaggregate_start,
    },
    Entry {
        slug: "interference",
        label: "Interference",
        start: interference_start,
    },
    Entry {
        slug: "intermomentary",
        label: "Intermomentary",
        start: intermomentary_start,
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
        slug: "lightning",
        label: "Lightning",
        start: lightning_start,
    },
    Entry {
        slug: "lisa",
        label: "Lisa",
        start: lisa_start,
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
        slug: "lmorph",
        label: "LMorph",
        start: lmorph_start,
    },
    Entry {
        slug: "marbling",
        label: "Marbling",
        start: marbling_start,
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
        slug: "petri",
        label: "Petri",
        start: petri_start,
    },
    Entry {
        slug: "piecewise",
        label: "Piecewise",
        start: piecewise_start,
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
        slug: "qix",
        label: "Qix",
        start: qix_start,
    },
    Entry {
        slug: "rdbomb",
        label: "RD-Bomb",
        start: rdbomb_start,
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
        slug: "scooter",
        label: "Scooter",
        start: scooter_start,
    },
    Entry {
        slug: "shadebobs",
        label: "Shade Bobs",
        start: shadebobs_start,
    },
    Entry {
        slug: "rotzoomer",
        label: "Rot Zoomer",
        start: rotzoomer_start,
    },
    Entry {
        slug: "sierpinski",
        label: "Sierpinski",
        start: sierpinski_start,
    },
    Entry {
        slug: "slidescreen",
        label: "Slide Screen",
        start: slidescreen_start,
    },
    Entry {
        slug: "slip",
        label: "Slip",
        start: slip_start,
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
        slug: "spotlight",
        label: "Spotlight",
        start: spotlight_start,
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
        slug: "substrate",
        label: "Substrate",
        start: substrate_start,
    },
    Entry {
        slug: "t3d",
        label: "T3D",
        start: t3d_start,
    },
    Entry {
        slug: "tessellimage",
        label: "Tessellimage",
        start: tessellimage_start,
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
        slug: "twang",
        label: "Twang",
        start: twang_start,
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
        slug: "whirlygig",
        label: "Whirlygig",
        start: whirlygig_start,
    },
    Entry {
        slug: "worm",
        label: "Worm",
        start: worm_start,
    },
    Entry {
        slug: "wormhole",
        label: "Wormhole",
        start: wormhole_start,
    },
    Entry {
        slug: "zoom",
        label: "Zoom",
        start: zoom_start,
    },
    Entry {
        slug: "xflame",
        label: "XFlame",
        start: xflame_start,
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
