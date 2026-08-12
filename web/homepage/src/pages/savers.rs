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

use xscreensaver::runtime::Runner3d;
use xscreensaver::runtime::{Runner, StartArgs};
use xscreensaver::shadertoy::Shadertoy;

type RunnerFuture = Pin<Box<dyn Future<Output = Option<Runner>>>>;
type ShadertoyFuture = Pin<Box<dyn Future<Output = Option<Shadertoy>>>>;
type Runner3dFuture = Pin<Box<dyn Future<Output = Option<Runner3d>>>>;

/// Which engine a saver needs, and how to start it.
///
/// Downloads the saver's chunk if it is not already resident, then starts it.
/// Resolves immediately once the chunk is in memory, so restarting a saver
/// after a settings change costs nothing extra. `None` if the chunk could not
/// be fetched.
///
/// The arms cannot be collapsed: a canvas has one context for its lifetime, so
/// the stage has to know before it mounts whether it is going to be rasterising
/// into a 2D context or handing geometry or fragments to WebGL2. The two GL
/// arms are separate for a smaller reason, that they want different context
/// options and a different shader.
pub enum Start {
    /// A 2D hack, drawing into a software framebuffer.
    Fb(fn(StartArgs) -> RunnerFuture),
    /// An OpenGL saver, drawing vertex batches.
    Gl3d(fn(StartArgs) -> Runner3dFuture),
    /// A Shadertoy program, drawing as a fragment shader.
    Gl(fn(StartArgs) -> ShadertoyFuture),
}

/// One saver, as the router sees it before its code is loaded.
pub struct Entry {
    pub slug: &'static str,
    pub label: &'static str,
    pub start: Start,
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

/// The same, for an OpenGL saver.
macro_rules! gl3d_saver {
    ($slug:literal, $body:ident, $load:ident, $path:path) => {
        fn $body(args: StartArgs) -> Runner3d {
            $path(args)
        }

        #[cfg(feature = "split")]
        fn $load(args: StartArgs) -> Runner3dFuture {
            Box::pin(async {
                static MODULE: wasm_split::LazyLoader<StartArgs, Runner3d> =
            wasm_split::lazy_loader!(extern $slug fn $body(props: StartArgs) -> Runner3d);
                if MODULE.load().await {
                    MODULE.call(args).ok()
                } else {
                    None
                }
            })
        }

        #[cfg(not(feature = "split"))]
        fn $load(args: StartArgs) -> Runner3dFuture {
            Box::pin(async { Some($body(args)) })
        }
    };
}

/// The same, for a Shadertoy saver. Its chunk holds the program text rather
/// than code, since the runner is shared and lives in the main module.
macro_rules! gl_saver {
    ($slug:literal, $body:ident, $load:ident, $path:path) => {
        fn $body(args: StartArgs) -> Shadertoy {
            $path(args)
        }

        #[cfg(feature = "split")]
        fn $load(args: StartArgs) -> ShadertoyFuture {
            Box::pin(async {
                static MODULE: wasm_split::LazyLoader<StartArgs, Shadertoy> =
            wasm_split::lazy_loader!(extern $slug fn $body(props: StartArgs) -> Shadertoy);
                if MODULE.load().await {
                    MODULE.call(args).ok()
                } else {
                    None
                }
            })
        }

        #[cfg(not(feature = "split"))]
        fn $load(args: StartArgs) -> ShadertoyFuture {
            Box::pin(async { Some($body(args)) })
        }
    };
}

saver!(
    "ant",
    ant_body,
    ant_start,
    xscreensaver::hacks2d::ant::start
);
saver!(
    "abstractile",
    abstractile_body,
    abstractile_start,
    xscreensaver::hacks2d::abstractile::start
);
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
    "apple2",
    apple2_body,
    apple2_start,
    xscreensaver::hacks2d::apple2::start
);
saver!(
    "attraction",
    attraction_body,
    attraction_start,
    xscreensaver::hacks2d::attraction::start
);
saver!(
    "barcode",
    barcode_body,
    barcode_start,
    xscreensaver::hacks2d::barcode::start
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
    "blaster",
    blaster_body,
    blaster_start,
    xscreensaver::hacks2d::blaster::start
);
saver!(
    "blitspin",
    blitspin_body,
    blitspin_start,
    xscreensaver::hacks2d::blitspin::start
);
saver!(
    "bsod",
    bsod_body,
    bsod_start,
    xscreensaver::hacks2d::bsod::start
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
    "bubbles",
    bubbles_body,
    bubbles_start,
    xscreensaver::hacks2d::bubbles::start
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
    "celtic",
    celtic_body,
    celtic_start,
    xscreensaver::hacks2d::celtic::start
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
    "crystal",
    crystal_body,
    crystal_start,
    xscreensaver::hacks2d::crystal::start
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
    "filmleader",
    filmleader_body,
    filmleader_start,
    xscreensaver::hacks2d::filmleader::start
);
saver!(
    "fireworkx",
    fireworkx_body,
    fireworkx_start,
    xscreensaver::hacks2d::fireworkx::start
);
saver!(
    "flag",
    flag_body,
    flag_start,
    xscreensaver::hacks2d::flag::start
);
saver!(
    "flame",
    flame_body,
    flame_start,
    xscreensaver::hacks2d::flame::start
);
saver!(
    "flow",
    flow_body,
    flow_start,
    xscreensaver::hacks2d::flow::start
);
saver!(
    "fluidballs",
    fluidballs_body,
    fluidballs_start,
    xscreensaver::hacks2d::fluidballs::start
);
saver!(
    "fontglide",
    fontglide_body,
    fontglide_start,
    xscreensaver::hacks2d::fontglide::start
);
saver!(
    "forest",
    forest_body,
    forest_start,
    xscreensaver::hacks2d::forest::start
);
saver!(
    "glitchpeg",
    glitchpeg_body,
    glitchpeg_start,
    xscreensaver::hacks2d::glitchpeg::start
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
    "hyperball",
    hyperball_body,
    hyperball_start,
    xscreensaver::hacks2d::hyperball::start
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
    "juggle",
    juggle_body,
    juggle_start,
    xscreensaver::hacks2d::juggle::start
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
    "loop",
    loop_body,
    loop_start,
    xscreensaver::hacks2d::r#loop::start
);
saver!(
    "m6502",
    m6502_body,
    m6502_start,
    xscreensaver::hacks2d::m6502::start
);
saver!(
    "marbling",
    marbling_body,
    marbling_start,
    xscreensaver::hacks2d::marbling::start
);
saver!(
    "maze",
    maze_body,
    maze_start,
    xscreensaver::hacks2d::maze::start
);
saver!(
    "memscroller",
    memscroller_body,
    memscroller_start,
    xscreensaver::hacks2d::memscroller::start
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
    "nerverot",
    nerverot_body,
    nerverot_start,
    xscreensaver::hacks2d::nerverot::start
);
saver!(
    "noseguy",
    noseguy_body,
    noseguy_start,
    xscreensaver::hacks2d::noseguy::start
);
saver!(
    "pacman",
    pacman_body,
    pacman_start,
    xscreensaver::hacks2d::pacman::start
);
saver!(
    "pedal",
    pedal_body,
    pedal_start,
    xscreensaver::hacks2d::pedal::start
);
saver!(
    "penetrate",
    penetrate_body,
    penetrate_start,
    xscreensaver::hacks2d::penetrate::start
);
saver!(
    "penrose",
    penrose_body,
    penrose_start,
    xscreensaver::hacks2d::penrose::start
);
saver!(
    "petri",
    petri_body,
    petri_start,
    xscreensaver::hacks2d::petri::start
);
saver!(
    "phosphor",
    phosphor_body,
    phosphor_start,
    xscreensaver::hacks2d::phosphor::start
);
saver!(
    "piecewise",
    piecewise_body,
    piecewise_start,
    xscreensaver::hacks2d::piecewise::start
);
saver!(
    "polyominoes",
    polyominoes_body,
    polyominoes_start,
    xscreensaver::hacks2d::polyominoes::start
);
saver!(
    "pong",
    pong_body,
    pong_start,
    xscreensaver::hacks2d::pong::start
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
    "ripples",
    ripples_body,
    ripples_start,
    xscreensaver::hacks2d::ripples::start
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
    "speedmine",
    speedmine_body,
    speedmine_start,
    xscreensaver::hacks2d::speedmine::start
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
    "strange",
    strange_body,
    strange_start,
    xscreensaver::hacks2d::strange::start
);
saver!(
    "substrate",
    substrate_body,
    substrate_start,
    xscreensaver::hacks2d::substrate::start
);
saver!(
    "swirl",
    swirl_body,
    swirl_start,
    xscreensaver::hacks2d::swirl::start
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
    "vermiculate",
    vermiculate_body,
    vermiculate_start,
    xscreensaver::hacks2d::vermiculate::start
);
saver!(
    "vfeedback",
    vfeedback_body,
    vfeedback_start,
    xscreensaver::hacks2d::vfeedback::start
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
    "xanalogtv",
    xanalogtv_body,
    xanalogtv_start,
    xscreensaver::hacks2d::xanalogtv::start
);
saver!(
    "xflame",
    xflame_body,
    xflame_start,
    xscreensaver::hacks2d::xflame::start
);
saver!(
    "xjack",
    xjack_body,
    xjack_start,
    xscreensaver::hacks2d::xjack::start
);
saver!(
    "xlyap",
    xlyap_body,
    xlyap_start,
    xscreensaver::hacks2d::xlyap::start
);
saver!(
    "xmatrix",
    xmatrix_body,
    xmatrix_start,
    xscreensaver::hacks2d::xmatrix::start
);
saver!(
    "xrayswarm",
    xrayswarm_body,
    xrayswarm_start,
    xscreensaver::hacks2d::xrayswarm::start
);
saver!(
    "xspirograph",
    xspirograph_body,
    xspirograph_start,
    xscreensaver::hacks2d::xspirograph::start
);

gl3d_saver!(
    "dnalogo",
    dnalogo_body,
    dnalogo_start,
    xscreensaver::hacks3d::dnalogo::start
);
gl3d_saver!(
    "juggler3d",
    juggler3d_body,
    juggler3d_start,
    xscreensaver::hacks3d::juggler3d::start
);
gl3d_saver!(
    "flurry",
    flurry_body,
    flurry_start,
    xscreensaver::hacks3d::flurry::start
);
gl3d_saver!(
    "atlantis",
    atlantis_body,
    atlantis_start,
    xscreensaver::hacks3d::atlantis::start
);
gl3d_saver!(
    "crackberg",
    crackberg_body,
    crackberg_start,
    xscreensaver::hacks3d::crackberg::start
);
gl3d_saver!(
    "cubicgrid",
    cubicgrid_body,
    cubicgrid_start,
    xscreensaver::hacks3d::cubicgrid::start
);
gl3d_saver!(
    "handsy",
    handsy_body,
    handsy_start,
    xscreensaver::hacks3d::handsy::start
);
gl3d_saver!(
    "headroom",
    headroom_body,
    headroom_start,
    xscreensaver::hacks3d::headroom::start
);
gl3d_saver!(
    "highvoltage",
    highvoltage_body,
    highvoltage_start,
    xscreensaver::hacks3d::highvoltage::start
);
gl3d_saver!(
    "winduprobot",
    winduprobot_body,
    winduprobot_start,
    xscreensaver::hacks3d::winduprobot::start
);
gl3d_saver!(
    "sproingies",
    sproingies_body,
    sproingies_start,
    xscreensaver::hacks3d::sproingies::start
);
gl3d_saver!(
    "carousel",
    carousel_body,
    carousel_start,
    xscreensaver::hacks3d::carousel::start
);
gl3d_saver!(
    "chompytower",
    chompytower_body,
    chompytower_start,
    xscreensaver::hacks3d::chompytower::start
);
gl3d_saver!(
    "skytentacles",
    skytentacles_body,
    skytentacles_start,
    xscreensaver::hacks3d::skytentacles::start
);
gl3d_saver!(
    "gltext",
    gltext_body,
    gltext_start,
    xscreensaver::hacks3d::gltext::start
);
gl3d_saver!(
    "glmatrix",
    glmatrix_body,
    glmatrix_start,
    xscreensaver::hacks3d::glmatrix::start
);
gl3d_saver!(
    "starwars",
    starwars_body,
    starwars_start,
    xscreensaver::hacks3d::starwars::start
);
gl3d_saver!(
    "fliptext",
    fliptext_body,
    fliptext_start,
    xscreensaver::hacks3d::fliptext::start
);
gl3d_saver!(
    "flipflop",
    flipflop_body,
    flipflop_start,
    xscreensaver::hacks3d::flipflop::start
);
gl3d_saver!(
    "flipscreen3d",
    flipscreen3d_body,
    flipscreen3d_start,
    xscreensaver::hacks3d::flipscreen3d::start
);
gl3d_saver!(
    "peepers",
    peepers_body,
    peepers_start,
    xscreensaver::hacks3d::peepers::start
);
gl3d_saver!(
    "photopile",
    photopile_body,
    photopile_start,
    xscreensaver::hacks3d::photopile::start
);
gl3d_saver!(
    "gflux",
    gflux_body,
    gflux_start,
    xscreensaver::hacks3d::gflux::start
);
gl3d_saver!(
    "hexstrut",
    hexstrut_body,
    hexstrut_start,
    xscreensaver::hacks3d::hexstrut::start
);
gl3d_saver!(
    "sballs",
    sballs_body,
    sballs_start,
    xscreensaver::hacks3d::sballs::start
);
gl3d_saver!(
    "sierpinski3d",
    sierpinski3d_body,
    sierpinski3d_start,
    xscreensaver::hacks3d::sierpinski3d::start
);
gl3d_saver!(
    "noof",
    noof_body,
    noof_start,
    xscreensaver::hacks3d::noof::start
);
gl3d_saver!(
    "moebius",
    moebius_body,
    moebius_start,
    xscreensaver::hacks3d::moebius::start
);
gl3d_saver!(
    "moebiusgears",
    moebiusgears_body,
    moebiusgears_start,
    xscreensaver::hacks3d::moebiusgears::start
);
gl3d_saver!(
    "mirrorblob",
    mirrorblob_body,
    mirrorblob_start,
    xscreensaver::hacks3d::mirrorblob::start
);
gl3d_saver!(
    "maze3d",
    maze3d_body,
    maze3d_start,
    xscreensaver::hacks3d::maze3d::start
);
gl3d_saver!(
    "nakagin",
    nakagin_body,
    nakagin_start,
    xscreensaver::hacks3d::nakagin::start
);
gl3d_saver!(
    "menger",
    menger_body,
    menger_start,
    xscreensaver::hacks3d::menger::start
);
gl3d_saver!(
    "hypnowheel",
    hypnowheel_body,
    hypnowheel_start,
    xscreensaver::hacks3d::hypnowheel::start
);
gl3d_saver!(
    "cubestack",
    cubestack_body,
    cubestack_start,
    xscreensaver::hacks3d::cubestack::start
);
gl3d_saver!(
    "cubestorm",
    cubestorm_body,
    cubestorm_start,
    xscreensaver::hacks3d::cubestorm::start
);
gl3d_saver!(
    "vigilance",
    vigilance_body,
    vigilance_start,
    xscreensaver::hacks3d::vigilance::start
);
gl3d_saver!(
    "voronoi",
    voronoi_body,
    voronoi_start,
    xscreensaver::hacks3d::voronoi::start
);
gl3d_saver!(
    "antinspect",
    antinspect_body,
    antinspect_start,
    xscreensaver::hacks3d::antinspect::start
);
gl3d_saver!(
    "antmaze",
    antmaze_body,
    antmaze_start,
    xscreensaver::hacks3d::antmaze::start
);
gl3d_saver!(
    "antspotlight",
    antspotlight_body,
    antspotlight_start,
    xscreensaver::hacks3d::antspotlight::start
);
gl3d_saver!(
    "atunnel",
    atunnel_body,
    atunnel_start,
    xscreensaver::hacks3d::atunnel::start
);
gl3d_saver!(
    "beats",
    beats_body,
    beats_start,
    xscreensaver::hacks3d::beats::start
);
gl3d_saver!(
    "crumbler",
    crumbler_body,
    crumbler_start,
    xscreensaver::hacks3d::crumbler::start
);
gl3d_saver!(
    "cube21",
    cube21_body,
    cube21_start,
    xscreensaver::hacks3d::cube21::start
);
gl3d_saver!(
    "cubetwist",
    cubetwist_body,
    cubetwist_start,
    xscreensaver::hacks3d::cubetwist::start
);
gl3d_saver!(
    "cubenetic",
    cubenetic_body,
    cubenetic_start,
    xscreensaver::hacks3d::cubenetic::start
);
gl3d_saver!(
    "raverhoop",
    raverhoop_body,
    raverhoop_start,
    xscreensaver::hacks3d::raverhoop::start
);
gl3d_saver!(
    "romanboy",
    romanboy_body,
    romanboy_start,
    xscreensaver::hacks3d::romanboy::start
);
gl3d_saver!(
    "razzledazzle",
    razzledazzle_body,
    razzledazzle_start,
    xscreensaver::hacks3d::razzledazzle::start
);
gl3d_saver!(
    "rubik",
    rubik_body,
    rubik_start,
    xscreensaver::hacks3d::rubik::start
);
gl3d_saver!(
    "rubikblocks",
    rubikblocks_body,
    rubikblocks_start,
    xscreensaver::hacks3d::rubikblocks::start
);
gl3d_saver!(
    "discoball",
    discoball_body,
    discoball_start,
    xscreensaver::hacks3d::discoball::start
);
gl3d_saver!(
    "dumpsterfire",
    dumpsterfire_body,
    dumpsterfire_start,
    xscreensaver::hacks3d::dumpsterfire::start
);
gl3d_saver!(
    "endgame",
    endgame_body,
    endgame_start,
    xscreensaver::hacks3d::endgame::start
);
gl3d_saver!(
    "energystream",
    energystream_body,
    energystream_start,
    xscreensaver::hacks3d::energystream::start
);
gl3d_saver!(
    "pinion",
    pinion_body,
    pinion_start,
    xscreensaver::hacks3d::pinion::start
);
gl3d_saver!(
    "polyhedra",
    polyhedra_body,
    polyhedra_start,
    xscreensaver::hacks3d::polyhedra::start
);
gl3d_saver!(
    "providence",
    providence_body,
    providence_start,
    xscreensaver::hacks3d::providence::start
);
gl3d_saver!(
    "pulsar",
    pulsar_body,
    pulsar_start,
    xscreensaver::hacks3d::pulsar::start
);
gl3d_saver!(
    "quasicrystal",
    quasicrystal_body,
    quasicrystal_start,
    xscreensaver::hacks3d::quasicrystal::start
);
gl3d_saver!(
    "kallisti",
    kallisti_body,
    kallisti_start,
    xscreensaver::hacks3d::kallisti::start
);
gl3d_saver!(
    "klein",
    klein_body,
    klein_start,
    xscreensaver::hacks3d::klein::start
);
gl3d_saver!(
    "lament",
    lament_body,
    lament_start,
    xscreensaver::hacks3d::lament::start
);
gl3d_saver!(
    "lavalite",
    lavalite_body,
    lavalite_start,
    xscreensaver::hacks3d::lavalite::start
);
gl3d_saver!(
    "lockward",
    lockward_body,
    lockward_start,
    xscreensaver::hacks3d::lockward::start
);
gl3d_saver!(
    "glsnake",
    glsnake_body,
    glsnake_start,
    xscreensaver::hacks3d::glsnake::start
);
gl3d_saver!(
    "gravitywell",
    gravitywell_body,
    gravitywell_start,
    xscreensaver::hacks3d::gravitywell::start
);
gl3d_saver!(
    "hextrail",
    hextrail_body,
    hextrail_start,
    xscreensaver::hacks3d::hextrail::start
);
gl3d_saver!(
    "bouncingcow",
    bouncingcow_body,
    bouncingcow_start,
    xscreensaver::hacks3d::bouncingcow::start
);
gl3d_saver!(
    "boxed",
    boxed_body,
    boxed_start,
    xscreensaver::hacks3d::boxed::start
);
gl3d_saver!(
    "bubble3d",
    bubble3d_body,
    bubble3d_start,
    xscreensaver::hacks3d::bubble3d::start
);
gl3d_saver!(
    "cage",
    cage_body,
    cage_start,
    xscreensaver::hacks3d::cage::start
);
gl3d_saver!(
    "circuit",
    circuit_body,
    circuit_start,
    xscreensaver::hacks3d::circuit::start
);
gl3d_saver!(
    "cityflow",
    cityflow_body,
    cityflow_start,
    xscreensaver::hacks3d::cityflow::start
);
gl3d_saver!(
    "blocktube",
    blocktube_body,
    blocktube_start,
    xscreensaver::hacks3d::blocktube::start
);
gl3d_saver!(
    "boing",
    boing_body,
    boing_start,
    xscreensaver::hacks3d::boing::start
);
gl3d_saver!(
    "blinkbox",
    blinkbox_body,
    blinkbox_start,
    xscreensaver::hacks3d::blinkbox::start
);
gl3d_saver!(
    "surfaces",
    surfaces_body,
    surfaces_start,
    xscreensaver::hacks3d::surfaces::start
);
gl3d_saver!(
    "tronbit",
    tronbit_body,
    tronbit_start,
    xscreensaver::hacks3d::tronbit::start
);
gl3d_saver!(
    "morph3d",
    morph3d_body,
    morph3d_start,
    xscreensaver::hacks3d::morph3d::start
);
gl3d_saver!(
    "hydrostat",
    hydrostat_body,
    hydrostat_start,
    xscreensaver::hacks3d::hydrostat::start
);
gl3d_saver!(
    "topblock",
    topblock_body,
    topblock_start,
    xscreensaver::hacks3d::topblock::start
);
gl3d_saver!(
    "skulloop",
    skulloop_body,
    skulloop_start,
    xscreensaver::hacks3d::skulloop::start
);
gl3d_saver!(
    "spheremonics",
    spheremonics_body,
    spheremonics_start,
    xscreensaver::hacks3d::spheremonics::start
);
gl3d_saver!(
    "hypertorus",
    hypertorus_body,
    hypertorus_start,
    xscreensaver::hacks3d::hypertorus::start
);
gl3d_saver!(
    "tangram",
    tangram_body,
    tangram_start,
    xscreensaver::hacks3d::tangram::start
);
gl3d_saver!(
    "papercube",
    papercube_body,
    papercube_start,
    xscreensaver::hacks3d::papercube::start
);
gl3d_saver!(
    "engine",
    engine_body,
    engine_start,
    xscreensaver::hacks3d::engine::start
);
gl3d_saver!(
    "esper",
    esper_body,
    esper_start,
    xscreensaver::hacks3d::esper::start
);
gl3d_saver!(
    "etruscanvenus",
    etruscanvenus_body,
    etruscanvenus_start,
    xscreensaver::hacks3d::etruscanvenus::start
);
gl3d_saver!(
    "molecule",
    molecule_body,
    molecule_start,
    xscreensaver::hacks3d::molecule::start
);
gl3d_saver!(
    "projectiveplane",
    projectiveplane_body,
    projectiveplane_start,
    xscreensaver::hacks3d::projectiveplane::start
);
gl3d_saver!(
    "polytopes",
    polytopes_body,
    polytopes_start,
    xscreensaver::hacks3d::polytopes::start
);
gl3d_saver!(
    "queens",
    queens_body,
    queens_start,
    xscreensaver::hacks3d::queens::start
);
gl3d_saver!(
    "geodesic",
    geodesic_body,
    geodesic_start,
    xscreensaver::hacks3d::geodesic::start
);
gl3d_saver!(
    "geodesicgears",
    geodesicgears_body,
    geodesicgears_start,
    xscreensaver::hacks3d::geodesicgears::start
);
gl3d_saver!(
    "glforestfire",
    glforestfire_body,
    glforestfire_start,
    xscreensaver::hacks3d::glforestfire::start
);
gl3d_saver!(
    "gleidescope",
    gleidescope_body,
    gleidescope_start,
    xscreensaver::hacks3d::gleidescope::start
);
gl3d_saver!(
    "glslideshow",
    glslideshow_body,
    glslideshow_start,
    xscreensaver::hacks3d::glslideshow::start
);
gl3d_saver!(
    "hilbert",
    hilbert_body,
    hilbert_start,
    xscreensaver::hacks3d::hilbert::start
);
gl3d_saver!(
    "jigsaw",
    jigsaw_body,
    jigsaw_start,
    xscreensaver::hacks3d::jigsaw::start
);
gl3d_saver!(
    "superquadrics",
    superquadrics_body,
    superquadrics_start,
    xscreensaver::hacks3d::superquadrics::start
);
gl3d_saver!(
    "unknownpleasures",
    unknownpleasures_body,
    unknownpleasures_start,
    xscreensaver::hacks3d::unknownpleasures::start
);
gl3d_saver!(
    "stairs",
    stairs_body,
    stairs_start,
    xscreensaver::hacks3d::stairs::start
);
gl3d_saver!(
    "stonerview",
    stonerview_body,
    stonerview_start,
    xscreensaver::hacks3d::stonerview::start
);
gl3d_saver!(
    "splitflap",
    splitflap_body,
    splitflap_start,
    xscreensaver::hacks3d::splitflap::start
);
gl3d_saver!(
    "splodesic",
    splodesic_body,
    splodesic_start,
    xscreensaver::hacks3d::splodesic::start
);
gl3d_saver!(
    "jigglypuff",
    jigglypuff_body,
    jigglypuff_start,
    xscreensaver::hacks3d::jigglypuff::start
);
gl3d_saver!(
    "kaleidocycle",
    kaleidocycle_body,
    kaleidocycle_start,
    xscreensaver::hacks3d::kaleidocycle::start
);
gl3d_saver!(
    "glschool",
    glschool_body,
    glschool_start,
    xscreensaver::hacks3d::glschool::start
);
gl3d_saver!(
    "flyingtoasters",
    flyingtoasters_body,
    flyingtoasters_start,
    xscreensaver::hacks3d::flyingtoasters::start
);
gl3d_saver!(
    "gears",
    gears_body,
    gears_start,
    xscreensaver::hacks3d::gears::start
);
gl3d_saver!(
    "gibson",
    gibson_body,
    gibson_start,
    xscreensaver::hacks3d::gibson::start
);
gl3d_saver!(
    "glblur",
    glblur_body,
    glblur_start,
    xscreensaver::hacks3d::glblur::start
);
gl3d_saver!(
    "glhanoi",
    glhanoi_body,
    glhanoi_start,
    xscreensaver::hacks3d::glhanoi::start
);
gl3d_saver!(
    "glknots",
    glknots_body,
    glknots_start,
    xscreensaver::hacks3d::glknots::start
);
gl3d_saver!(
    "dangerball",
    dangerball_body,
    dangerball_start,
    xscreensaver::hacks3d::dangerball::start
);
gl3d_saver!(
    "deepstars",
    deepstars_body,
    deepstars_start,
    xscreensaver::hacks3d::deepstars::start
);
gl_saver!(
    "alienbeacon",
    alienbeacon_body,
    alienbeacon_start,
    xscreensaver::shadertoy::alienbeacon::start
);
gl_saver!(
    "batteredplanet",
    batteredplanet_body,
    batteredplanet_start,
    xscreensaver::shadertoy::batteredplanet::start
);
gl_saver!(
    "bestill",
    bestill_body,
    bestill_start,
    xscreensaver::shadertoy::bestill::start
);
gl_saver!(
    "bubblecolors",
    bubblecolors_body,
    bubblecolors_start,
    xscreensaver::shadertoy::bubblecolors::start
);
gl_saver!(
    "darktransit",
    darktransit_body,
    darktransit_start,
    xscreensaver::shadertoy::darktransit::start
);
gl_saver!(
    "downfall",
    downfall_body,
    downfall_start,
    xscreensaver::shadertoy::downfall::start
);
gl_saver!(
    "driftclouds",
    driftclouds_body,
    driftclouds_start,
    xscreensaver::shadertoy::driftclouds::start
);
gl_saver!(
    "elementalring",
    elementalring_body,
    elementalring_start,
    xscreensaver::shadertoy::elementalring::start
);
gl_saver!(
    "fluxcore",
    fluxcore_body,
    fluxcore_start,
    xscreensaver::shadertoy::fluxcore::start
);
gl_saver!(
    "gimbalharmonics",
    gimbalharmonics_body,
    gimbalharmonics_start,
    xscreensaver::shadertoy::gimbalharmonics::start
);
gl_saver!(
    "goldenapollian",
    goldenapollian_body,
    goldenapollian_start,
    xscreensaver::shadertoy::goldenapollian::start
);
gl_saver!(
    "hexplasma",
    hexplasma_body,
    hexplasma_start,
    xscreensaver::shadertoy::hexplasma::start
);
gl_saver!(
    "logarithmiccircles",
    logarithmiccircles_body,
    logarithmiccircles_start,
    xscreensaver::shadertoy::logarithmiccircles::start
);
gl_saver!(
    "neongravity",
    neongravity_body,
    neongravity_start,
    xscreensaver::shadertoy::neongravity::start
);
gl_saver!(
    "neontriangulator",
    neontriangulator_body,
    neontriangulator_start,
    xscreensaver::shadertoy::neontriangulator::start
);
gl_saver!(
    "noxfire",
    noxfire_body,
    noxfire_start,
    xscreensaver::shadertoy::noxfire::start
);
gl_saver!(
    "prococean",
    prococean_body,
    prococean_start,
    xscreensaver::shadertoy::prococean::start
);
gl_saver!(
    "protophore",
    protophore_body,
    protophore_start,
    xscreensaver::shadertoy::protophore::start
);
gl_saver!(
    "rigrekt",
    rigrekt_body,
    rigrekt_start,
    xscreensaver::shadertoy::rigrekt::start
);
gl_saver!(
    "selfreflect",
    selfreflect_body,
    selfreflect_start,
    xscreensaver::shadertoy::selfreflect::start
);
gl_saver!(
    "skyline",
    skyline_body,
    skyline_start,
    xscreensaver::shadertoy::skyline::start
);
gl_saver!(
    "stardome",
    stardome_body,
    stardome_start,
    xscreensaver::shadertoy::stardome::start
);
gl_saver!(
    "starnest",
    starnest_body,
    starnest_start,
    xscreensaver::shadertoy::starnest::start
);
gl_saver!(
    "stripeytorus",
    stripeytorus_body,
    stripeytorus_start,
    xscreensaver::shadertoy::stripeytorus::start
);
gl_saver!(
    "synthwavecity",
    synthwavecity_body,
    synthwavecity_start,
    xscreensaver::shadertoy::synthwavecity::start
);
gl_saver!(
    "topologica",
    topologica_body,
    topologica_start,
    xscreensaver::shadertoy::topologica::start
);
gl_saver!(
    "trainmandala",
    trainmandala_body,
    trainmandala_start,
    xscreensaver::shadertoy::trainmandala::start
);
gl_saver!(
    "trizm",
    trizm_body,
    trizm_start,
    xscreensaver::shadertoy::trizm::start
);
gl_saver!(
    "truchetzoom",
    truchetzoom_body,
    truchetzoom_start,
    xscreensaver::shadertoy::truchetzoom::start
);
gl_saver!(
    "universeball",
    universeball_body,
    universeball_start,
    xscreensaver::shadertoy::universeball::start
);

/// Every saver, by slug. Only the slug, the label and a function pointer live
/// here; the code behind each one arrives on demand.
pub static SAVERS: &[Entry] = &[
    Entry {
        slug: "ant",
        label: "Ant",
        start: Start::Fb(ant_start),
    },
    Entry {
        slug: "abstractile",
        label: "Abstractile",
        start: Start::Fb(abstractile_start),
    },
    Entry {
        slug: "anemone",
        label: "Anemone",
        start: Start::Fb(anemone_start),
    },
    Entry {
        slug: "anemotaxis",
        label: "Anemotaxis",
        start: Start::Fb(anemotaxis_start),
    },
    Entry {
        slug: "apollonian",
        label: "Apollonian",
        start: Start::Fb(apollonian_start),
    },
    Entry {
        slug: "apple2",
        label: "Apple ][",
        start: Start::Fb(apple2_start),
    },
    Entry {
        slug: "attraction",
        label: "Attraction",
        start: Start::Fb(attraction_start),
    },
    Entry {
        slug: "barcode",
        label: "Barcode",
        start: Start::Fb(barcode_start),
    },
    Entry {
        slug: "binaryhorizon",
        label: "Binary Horizon",
        start: Start::Fb(binaryhorizon_start),
    },
    Entry {
        slug: "binaryring",
        label: "Binary Ring",
        start: Start::Fb(binaryring_start),
    },
    Entry {
        slug: "blaster",
        label: "Blaster",
        start: Start::Fb(blaster_start),
    },
    Entry {
        slug: "blitspin",
        label: "Blit Spin",
        start: Start::Fb(blitspin_start),
    },
    Entry {
        slug: "bouboule",
        label: "Bouboule",
        start: Start::Fb(bouboule_start),
    },
    Entry {
        slug: "bsod",
        label: "BSOD",
        start: Start::Fb(bsod_start),
    },
    Entry {
        slug: "boxfit",
        label: "Box Fit",
        start: Start::Fb(boxfit_start),
    },
    Entry {
        slug: "braid",
        label: "Braid",
        start: Start::Fb(braid_start),
    },
    Entry {
        slug: "bubbles",
        label: "Bubbles",
        start: Start::Fb(bubbles_start),
    },
    Entry {
        slug: "bumps",
        label: "Bumps",
        start: Start::Fb(bumps_start),
    },
    Entry {
        slug: "ccurve",
        label: "C Curve",
        start: Start::Fb(ccurve_start),
    },
    Entry {
        slug: "celtic",
        label: "Celtic",
        start: Start::Fb(celtic_start),
    },
    Entry {
        slug: "cloudlife",
        label: "Cloud Life",
        start: Start::Fb(cloudlife_start),
    },
    Entry {
        slug: "compass",
        label: "Compass",
        start: Start::Fb(compass_start),
    },
    Entry {
        slug: "coral",
        label: "Coral",
        start: Start::Fb(coral_start),
    },
    Entry {
        slug: "critical",
        label: "Critical",
        start: Start::Fb(critical_start),
    },
    Entry {
        slug: "crystal",
        label: "Crystal",
        start: Start::Fb(crystal_start),
    },
    Entry {
        slug: "cwaves",
        label: "C Waves",
        start: Start::Fb(cwaves_start),
    },
    Entry {
        slug: "deco",
        label: "Deco",
        start: Start::Fb(deco_start),
    },
    Entry {
        slug: "cynosure",
        label: "Cynosure",
        start: Start::Fb(cynosure_start),
    },
    Entry {
        slug: "decayscreen",
        label: "Decay Screen",
        start: Start::Fb(decayscreen_start),
    },
    Entry {
        slug: "deluxe",
        label: "Deluxe",
        start: Start::Fb(deluxe_start),
    },
    Entry {
        slug: "demon",
        label: "Demon",
        start: Start::Fb(demon_start),
    },
    Entry {
        slug: "discrete",
        label: "Discrete",
        start: Start::Fb(discrete_start),
    },
    Entry {
        slug: "fuzzyflakes",
        label: "Fuzzy Flakes",
        start: Start::Fb(fuzzyflakes_start),
    },
    Entry {
        slug: "galaxy",
        label: "Galaxy",
        start: Start::Fb(galaxy_start),
    },
    Entry {
        slug: "greynetic",
        label: "Greynetic",
        start: Start::Fb(greynetic_start),
    },
    Entry {
        slug: "distort",
        label: "Distort",
        start: Start::Fb(distort_start),
    },
    Entry {
        slug: "drift",
        label: "Drift",
        start: Start::Fb(drift_start),
    },
    Entry {
        slug: "droste",
        label: "Droste",
        start: Start::Fb(droste_start),
    },
    Entry {
        slug: "epicycle",
        label: "Epicycle",
        start: Start::Fb(epicycle_start),
    },
    Entry {
        slug: "euler2d",
        label: "Euler 2D",
        start: Start::Fb(euler2d_start),
    },
    Entry {
        slug: "eruption",
        label: "Eruption",
        start: Start::Fb(eruption_start),
    },
    Entry {
        slug: "fadeplot",
        label: "Fade Plot",
        start: Start::Fb(fadeplot_start),
    },
    Entry {
        slug: "fiberlamp",
        label: "Fiber Lamp",
        start: Start::Fb(fiberlamp_start),
    },
    Entry {
        slug: "filmleader",
        label: "Film Leader",
        start: Start::Fb(filmleader_start),
    },
    Entry {
        slug: "fireworkx",
        label: "Fireworkx",
        start: Start::Fb(fireworkx_start),
    },
    Entry {
        slug: "flag",
        label: "Flag",
        start: Start::Fb(flag_start),
    },
    Entry {
        slug: "flame",
        label: "Flame",
        start: Start::Fb(flame_start),
    },
    Entry {
        slug: "flow",
        label: "Flow",
        start: Start::Fb(flow_start),
    },
    Entry {
        slug: "fluidballs",
        label: "Fluid Balls",
        start: Start::Fb(fluidballs_start),
    },
    Entry {
        slug: "fontglide",
        label: "Font Glide",
        start: Start::Fb(fontglide_start),
    },
    Entry {
        slug: "forest",
        label: "Forest",
        start: Start::Fb(forest_start),
    },
    Entry {
        slug: "glitchpeg",
        label: "GlitchPEG",
        start: Start::Fb(glitchpeg_start),
    },
    Entry {
        slug: "goop",
        label: "Goop",
        start: Start::Fb(goop_start),
    },
    Entry {
        slug: "grav",
        label: "Grav",
        start: Start::Fb(grav_start),
    },
    Entry {
        slug: "halo",
        label: "Halo",
        start: Start::Fb(halo_start),
    },
    Entry {
        slug: "halftone",
        label: "Halftone",
        start: Start::Fb(halftone_start),
    },
    Entry {
        slug: "helix",
        label: "Helix",
        start: Start::Fb(helix_start),
    },
    Entry {
        slug: "hexadrop",
        label: "Hexadrop",
        start: Start::Fb(hexadrop_start),
    },
    Entry {
        slug: "hopalong",
        label: "Hopalong",
        start: Start::Fb(hopalong_start),
    },
    Entry {
        slug: "hyperball",
        label: "Hyperball",
        start: Start::Fb(hyperball_start),
    },
    Entry {
        slug: "hypercube",
        label: "Hypercube",
        start: Start::Fb(hypercube_start),
    },
    Entry {
        slug: "ifs",
        label: "IFS",
        start: Start::Fb(ifs_start),
    },
    Entry {
        slug: "imsmap",
        label: "IMS Map",
        start: Start::Fb(imsmap_start),
    },
    Entry {
        slug: "interaggregate",
        label: "Interaggregate",
        start: Start::Fb(interaggregate_start),
    },
    Entry {
        slug: "interference",
        label: "Interference",
        start: Start::Fb(interference_start),
    },
    Entry {
        slug: "intermomentary",
        label: "Intermomentary",
        start: Start::Fb(intermomentary_start),
    },
    Entry {
        slug: "juggle",
        label: "Juggle",
        start: Start::Fb(juggle_start),
    },
    Entry {
        slug: "julia",
        label: "Julia",
        start: Start::Fb(julia_start),
    },
    Entry {
        slug: "kaleidescope",
        label: "Kaleidescope",
        start: Start::Fb(kaleidescope_start),
    },
    Entry {
        slug: "lament",
        label: "Lament",
        start: Start::Gl3d(lament_start),
    },
    Entry {
        slug: "laser",
        label: "Laser",
        start: Start::Fb(laser_start),
    },
    Entry {
        slug: "kumppa",
        label: "Kumppa",
        start: Start::Fb(kumppa_start),
    },
    Entry {
        slug: "lcdscrub",
        label: "LCD Scrub",
        start: Start::Fb(lcdscrub_start),
    },
    Entry {
        slug: "lightning",
        label: "Lightning",
        start: Start::Fb(lightning_start),
    },
    Entry {
        slug: "lisa",
        label: "Lisa",
        start: Start::Fb(lisa_start),
    },
    Entry {
        slug: "lissie",
        label: "Lissie",
        start: Start::Fb(lissie_start),
    },
    Entry {
        slug: "moire",
        label: "Moiré",
        start: Start::Fb(moire_start),
    },
    Entry {
        slug: "lmorph",
        label: "LMorph",
        start: Start::Fb(lmorph_start),
    },
    Entry {
        slug: "loop",
        label: "Loop",
        start: Start::Fb(loop_start),
    },
    Entry {
        slug: "m6502",
        label: "m6502",
        start: Start::Fb(m6502_start),
    },
    Entry {
        slug: "marbling",
        label: "Marbling",
        start: Start::Fb(marbling_start),
    },
    Entry {
        slug: "maze",
        label: "Maze",
        start: Start::Fb(maze_start),
    },
    Entry {
        slug: "memscroller",
        label: "Mem Scroller",
        start: Start::Fb(memscroller_start),
    },
    Entry {
        slug: "metaballs",
        label: "Meta Balls",
        start: Start::Fb(metaballs_start),
    },
    Entry {
        slug: "moire2",
        label: "Moiré 2",
        start: Start::Fb(moire2_start),
    },
    Entry {
        slug: "mountain",
        label: "Mountain",
        start: Start::Fb(mountain_start),
    },
    Entry {
        slug: "munch",
        label: "Munch",
        start: Start::Fb(munch_start),
    },
    Entry {
        slug: "nerverot",
        label: "Nerve Rot",
        start: Start::Fb(nerverot_start),
    },
    Entry {
        slug: "noseguy",
        label: "Nose Guy",
        start: Start::Fb(noseguy_start),
    },
    Entry {
        slug: "pacman",
        label: "Pac-Man",
        start: Start::Fb(pacman_start),
    },
    Entry {
        slug: "pedal",
        label: "Pedal",
        start: Start::Fb(pedal_start),
    },
    Entry {
        slug: "penetrate",
        label: "Penetrate",
        start: Start::Fb(penetrate_start),
    },
    Entry {
        slug: "penrose",
        label: "Penrose",
        start: Start::Fb(penrose_start),
    },
    Entry {
        slug: "petri",
        label: "Petri",
        start: Start::Fb(petri_start),
    },
    Entry {
        slug: "phosphor",
        label: "Phosphor",
        start: Start::Fb(phosphor_start),
    },
    Entry {
        slug: "piecewise",
        label: "Piecewise",
        start: Start::Fb(piecewise_start),
    },
    Entry {
        slug: "polyominoes",
        label: "Polyominoes",
        start: Start::Fb(polyominoes_start),
    },
    Entry {
        slug: "pong",
        label: "Pong",
        start: Start::Fb(pong_start),
    },
    Entry {
        slug: "popsquares",
        label: "Pop Squares",
        start: Start::Fb(popsquares_start),
    },
    Entry {
        slug: "pyro",
        label: "Pyro",
        start: Start::Fb(pyro_start),
    },
    Entry {
        slug: "qix",
        label: "Qix",
        start: Start::Fb(qix_start),
    },
    Entry {
        slug: "rdbomb",
        label: "RD-Bomb",
        start: Start::Fb(rdbomb_start),
    },
    Entry {
        slug: "ripples",
        label: "Ripples",
        start: Start::Fb(ripples_start),
    },
    Entry {
        slug: "rocks",
        label: "Rocks",
        start: Start::Fb(rocks_start),
    },
    Entry {
        slug: "rorschach",
        label: "Rorschach",
        start: Start::Fb(rorschach_start),
    },
    Entry {
        slug: "rotor",
        label: "Rotor",
        start: Start::Fb(rotor_start),
    },
    Entry {
        slug: "scooter",
        label: "Scooter",
        start: Start::Fb(scooter_start),
    },
    Entry {
        slug: "shadebobs",
        label: "Shade Bobs",
        start: Start::Fb(shadebobs_start),
    },
    Entry {
        slug: "rotzoomer",
        label: "Rot Zoomer",
        start: Start::Fb(rotzoomer_start),
    },
    Entry {
        slug: "sierpinski",
        label: "Sierpinski",
        start: Start::Fb(sierpinski_start),
    },
    Entry {
        slug: "slidescreen",
        label: "Slide Screen",
        start: Start::Fb(slidescreen_start),
    },
    Entry {
        slug: "slip",
        label: "Slip",
        start: Start::Fb(slip_start),
    },
    Entry {
        slug: "speedmine",
        label: "Speed Mine",
        start: Start::Fb(speedmine_start),
    },
    Entry {
        slug: "sphere",
        label: "Sphere",
        start: Start::Fb(sphere_start),
    },
    Entry {
        slug: "spiral",
        label: "Spiral",
        start: Start::Fb(spiral_start),
    },
    Entry {
        slug: "spotlight",
        label: "Spotlight",
        start: Start::Fb(spotlight_start),
    },
    Entry {
        slug: "squiral",
        label: "Squiral",
        start: Start::Fb(squiral_start),
    },
    Entry {
        slug: "starfish",
        label: "Starfish",
        start: Start::Fb(starfish_start),
    },
    Entry {
        slug: "strange",
        label: "Strange",
        start: Start::Fb(strange_start),
    },
    Entry {
        slug: "substrate",
        label: "Substrate",
        start: Start::Fb(substrate_start),
    },
    Entry {
        slug: "swirl",
        label: "Swirl",
        start: Start::Fb(swirl_start),
    },
    Entry {
        slug: "t3d",
        label: "T3D",
        start: Start::Fb(t3d_start),
    },
    Entry {
        slug: "tessellimage",
        label: "Tessellimage",
        start: Start::Fb(tessellimage_start),
    },
    Entry {
        slug: "thornbird",
        label: "Thornbird",
        start: Start::Fb(thornbird_start),
    },
    Entry {
        slug: "triangle",
        label: "Triangle",
        start: Start::Fb(triangle_start),
    },
    Entry {
        slug: "twang",
        label: "Twang",
        start: Start::Fb(twang_start),
    },
    Entry {
        slug: "vines",
        label: "Vines",
        start: Start::Fb(vines_start),
    },
    Entry {
        slug: "truchet",
        label: "Truchet",
        start: Start::Fb(truchet_start),
    },
    Entry {
        slug: "vermiculate",
        label: "Vermiculate",
        start: Start::Fb(vermiculate_start),
    },
    Entry {
        slug: "vfeedback",
        label: "VFeedback",
        start: Start::Fb(vfeedback_start),
    },
    Entry {
        slug: "wander",
        label: "Wander",
        start: Start::Fb(wander_start),
    },
    Entry {
        slug: "whirlwindwarp",
        label: "Whirlwind Warp",
        start: Start::Fb(whirlwindwarp_start),
    },
    Entry {
        slug: "whirlygig",
        label: "Whirlygig",
        start: Start::Fb(whirlygig_start),
    },
    Entry {
        slug: "worm",
        label: "Worm",
        start: Start::Fb(worm_start),
    },
    Entry {
        slug: "wormhole",
        label: "Wormhole",
        start: Start::Fb(wormhole_start),
    },
    Entry {
        slug: "zoom",
        label: "Zoom",
        start: Start::Fb(zoom_start),
    },
    Entry {
        slug: "xanalogtv",
        label: "XAnalogTV",
        start: Start::Fb(xanalogtv_start),
    },
    Entry {
        slug: "xflame",
        label: "XFlame",
        start: Start::Fb(xflame_start),
    },
    Entry {
        slug: "xjack",
        label: "XJack",
        start: Start::Fb(xjack_start),
    },
    Entry {
        slug: "xlyap",
        label: "XLyap",
        start: Start::Fb(xlyap_start),
    },
    Entry {
        slug: "xmatrix",
        label: "XMatrix",
        start: Start::Fb(xmatrix_start),
    },
    Entry {
        slug: "xrayswarm",
        label: "XRaySwarm",
        start: Start::Fb(xrayswarm_start),
    },
    Entry {
        slug: "xspirograph",
        label: "XSpirograph",
        start: Start::Fb(xspirograph_start),
    },
    Entry {
        slug: "dnalogo",
        label: "DNA Logo",
        start: Start::Gl3d(dnalogo_start),
    },
    Entry {
        slug: "juggler3d",
        label: "Juggler 3D",
        start: Start::Gl3d(juggler3d_start),
    },
    Entry {
        slug: "flurry",
        label: "Flurry",
        start: Start::Gl3d(flurry_start),
    },
    Entry {
        slug: "atlantis",
        label: "Atlantis",
        start: Start::Gl3d(atlantis_start),
    },
    Entry {
        slug: "antinspect",
        label: "Ant Inspect",
        start: Start::Gl3d(antinspect_start),
    },
    Entry {
        slug: "antmaze",
        label: "Ant Maze",
        start: Start::Gl3d(antmaze_start),
    },
    Entry {
        slug: "antspotlight",
        label: "Ant Spotlight",
        start: Start::Gl3d(antspotlight_start),
    },
    Entry {
        slug: "atunnel",
        label: "Atunnel",
        start: Start::Gl3d(atunnel_start),
    },
    Entry {
        slug: "beats",
        label: "Beats",
        start: Start::Gl3d(beats_start),
    },
    Entry {
        slug: "blinkbox",
        label: "Blink Box",
        start: Start::Gl3d(blinkbox_start),
    },
    Entry {
        slug: "blocktube",
        label: "Block Tube",
        start: Start::Gl3d(blocktube_start),
    },
    Entry {
        slug: "boing",
        label: "Boing",
        start: Start::Gl3d(boing_start),
    },
    Entry {
        slug: "bouncingcow",
        label: "Bouncing Cow",
        start: Start::Gl3d(bouncingcow_start),
    },
    Entry {
        slug: "boxed",
        label: "Boxed",
        start: Start::Gl3d(boxed_start),
    },
    Entry {
        slug: "bubble3d",
        label: "Bubble 3D",
        start: Start::Gl3d(bubble3d_start),
    },
    Entry {
        slug: "cage",
        label: "Cage",
        start: Start::Gl3d(cage_start),
    },
    Entry {
        slug: "cityflow",
        label: "City Flow",
        start: Start::Gl3d(cityflow_start),
    },
    Entry {
        slug: "circuit",
        label: "Circuit",
        start: Start::Gl3d(circuit_start),
    },
    Entry {
        slug: "crackberg",
        label: "Crackberg",
        start: Start::Gl3d(crackberg_start),
    },
    Entry {
        slug: "crumbler",
        label: "Crumbler",
        start: Start::Gl3d(crumbler_start),
    },
    Entry {
        slug: "cube21",
        label: "Cube 21",
        start: Start::Gl3d(cube21_start),
    },
    Entry {
        slug: "cubenetic",
        label: "Cubenetic",
        start: Start::Gl3d(cubenetic_start),
    },
    Entry {
        slug: "cubestack",
        label: "Cube Stack",
        start: Start::Gl3d(cubestack_start),
    },
    Entry {
        slug: "cubestorm",
        label: "Cube Storm",
        start: Start::Gl3d(cubestorm_start),
    },
    Entry {
        slug: "cubetwist",
        label: "Cube Twist",
        start: Start::Gl3d(cubetwist_start),
    },
    Entry {
        slug: "cubicgrid",
        label: "Cubic Grid",
        start: Start::Gl3d(cubicgrid_start),
    },
    Entry {
        slug: "dangerball",
        label: "Danger Ball",
        start: Start::Gl3d(dangerball_start),
    },
    Entry {
        slug: "deepstars",
        label: "Deep Stars",
        start: Start::Gl3d(deepstars_start),
    },
    Entry {
        slug: "discoball",
        label: "Discoball",
        start: Start::Gl3d(discoball_start),
    },
    Entry {
        slug: "dumpsterfire",
        label: "Dumpster Fire",
        start: Start::Gl3d(dumpsterfire_start),
    },
    Entry {
        slug: "endgame",
        label: "Endgame",
        start: Start::Gl3d(endgame_start),
    },
    Entry {
        slug: "energystream",
        label: "Energy Stream",
        start: Start::Gl3d(energystream_start),
    },
    Entry {
        slug: "flyingtoasters",
        label: "Flying Toasters",
        start: Start::Gl3d(flyingtoasters_start),
    },
    Entry {
        slug: "gears",
        label: "Gears",
        start: Start::Gl3d(gears_start),
    },
    Entry {
        slug: "gibson",
        label: "Gibson",
        start: Start::Gl3d(gibson_start),
    },
    Entry {
        slug: "glblur",
        label: "GL Blur",
        start: Start::Gl3d(glblur_start),
    },
    Entry {
        slug: "glhanoi",
        label: "GL Hanoi",
        start: Start::Gl3d(glhanoi_start),
    },
    Entry {
        slug: "glknots",
        label: "GL Knots",
        start: Start::Gl3d(glknots_start),
    },
    Entry {
        slug: "glschool",
        label: "GL School",
        start: Start::Gl3d(glschool_start),
    },
    Entry {
        slug: "glsnake",
        label: "GL Snake",
        start: Start::Gl3d(glsnake_start),
    },
    Entry {
        slug: "gravitywell",
        label: "Gravity Well",
        start: Start::Gl3d(gravitywell_start),
    },
    Entry {
        slug: "handsy",
        label: "Handsy",
        start: Start::Gl3d(handsy_start),
    },
    Entry {
        slug: "headroom",
        label: "Headroom",
        start: Start::Gl3d(headroom_start),
    },
    Entry {
        slug: "highvoltage",
        label: "High Voltage",
        start: Start::Gl3d(highvoltage_start),
    },
    Entry {
        slug: "winduprobot",
        label: "Windup Robot",
        start: Start::Gl3d(winduprobot_start),
    },
    Entry {
        slug: "sproingies",
        label: "Sproingies",
        start: Start::Gl3d(sproingies_start),
    },
    Entry {
        slug: "carousel",
        label: "Carousel",
        start: Start::Gl3d(carousel_start),
    },
    Entry {
        slug: "chompytower",
        label: "Chompy Tower",
        start: Start::Gl3d(chompytower_start),
    },
    Entry {
        slug: "skytentacles",
        label: "Sky Tentacles",
        start: Start::Gl3d(skytentacles_start),
    },
    Entry {
        slug: "gltext",
        label: "GL Text",
        start: Start::Gl3d(gltext_start),
    },
    Entry {
        slug: "glmatrix",
        label: "GL Matrix",
        start: Start::Gl3d(glmatrix_start),
    },
    Entry {
        slug: "starwars",
        label: "Star Wars",
        start: Start::Gl3d(starwars_start),
    },
    Entry {
        slug: "fliptext",
        label: "Flip Text",
        start: Start::Gl3d(fliptext_start),
    },
    Entry {
        slug: "flipflop",
        label: "Flip Flop",
        start: Start::Gl3d(flipflop_start),
    },
    Entry {
        slug: "flipscreen3d",
        label: "Flip Screen 3D",
        start: Start::Gl3d(flipscreen3d_start),
    },
    Entry {
        slug: "peepers",
        label: "Peepers",
        start: Start::Gl3d(peepers_start),
    },
    Entry {
        slug: "photopile",
        label: "Photopile",
        start: Start::Gl3d(photopile_start),
    },
    Entry {
        slug: "gflux",
        label: "GFlux",
        start: Start::Gl3d(gflux_start),
    },
    Entry {
        slug: "hexstrut",
        label: "Hex Strut",
        start: Start::Gl3d(hexstrut_start),
    },
    Entry {
        slug: "hextrail",
        label: "Hex Trail",
        start: Start::Gl3d(hextrail_start),
    },
    Entry {
        slug: "hypnowheel",
        label: "Hypnowheel",
        start: Start::Gl3d(hypnowheel_start),
    },
    Entry {
        slug: "jigglypuff",
        label: "Jiggly Puff",
        start: Start::Gl3d(jigglypuff_start),
    },
    Entry {
        slug: "kaleidocycle",
        label: "Kaleidocycle",
        start: Start::Gl3d(kaleidocycle_start),
    },
    Entry {
        slug: "kallisti",
        label: "Kallisti",
        start: Start::Gl3d(kallisti_start),
    },
    Entry {
        slug: "klein",
        label: "Klein",
        start: Start::Gl3d(klein_start),
    },
    Entry {
        slug: "lavalite",
        label: "Lavalite",
        start: Start::Gl3d(lavalite_start),
    },
    Entry {
        slug: "lockward",
        label: "Lockward",
        start: Start::Gl3d(lockward_start),
    },
    Entry {
        slug: "menger",
        label: "Menger",
        start: Start::Gl3d(menger_start),
    },
    Entry {
        slug: "moebius",
        label: "Moebius",
        start: Start::Gl3d(moebius_start),
    },
    Entry {
        slug: "moebiusgears",
        label: "Moebius Gears",
        start: Start::Gl3d(moebiusgears_start),
    },
    Entry {
        slug: "mirrorblob",
        label: "Mirror Blob",
        start: Start::Gl3d(mirrorblob_start),
    },
    Entry {
        slug: "maze3d",
        label: "Maze 3D",
        start: Start::Gl3d(maze3d_start),
    },
    Entry {
        slug: "nakagin",
        label: "Nakagin",
        start: Start::Gl3d(nakagin_start),
    },
    Entry {
        slug: "noof",
        label: "Noof",
        start: Start::Gl3d(noof_start),
    },
    Entry {
        slug: "papercube",
        label: "Paper Cube",
        start: Start::Gl3d(papercube_start),
    },
    Entry {
        slug: "pinion",
        label: "Pinion",
        start: Start::Gl3d(pinion_start),
    },
    Entry {
        slug: "projectiveplane",
        label: "Projective Plane",
        start: Start::Gl3d(projectiveplane_start),
    },
    Entry {
        slug: "polyhedra",
        label: "Polyhedra",
        start: Start::Gl3d(polyhedra_start),
    },
    Entry {
        slug: "polytopes",
        label: "Polytopes",
        start: Start::Gl3d(polytopes_start),
    },
    Entry {
        slug: "providence",
        label: "Providence",
        start: Start::Gl3d(providence_start),
    },
    Entry {
        slug: "pulsar",
        label: "Pulsar",
        start: Start::Gl3d(pulsar_start),
    },
    Entry {
        slug: "queens",
        label: "Queens",
        start: Start::Gl3d(queens_start),
    },
    Entry {
        slug: "quasicrystal",
        label: "Quasi-Crystal",
        start: Start::Gl3d(quasicrystal_start),
    },
    Entry {
        slug: "raverhoop",
        label: "Raver Hoop",
        start: Start::Gl3d(raverhoop_start),
    },
    Entry {
        slug: "romanboy",
        label: "Roman Boy",
        start: Start::Gl3d(romanboy_start),
    },
    Entry {
        slug: "razzledazzle",
        label: "Razzle Dazzle",
        start: Start::Gl3d(razzledazzle_start),
    },
    Entry {
        slug: "rubik",
        label: "Rubik",
        start: Start::Gl3d(rubik_start),
    },
    Entry {
        slug: "rubikblocks",
        label: "Rubik Blocks",
        start: Start::Gl3d(rubikblocks_start),
    },
    Entry {
        slug: "sballs",
        label: "Sballs",
        start: Start::Gl3d(sballs_start),
    },
    Entry {
        slug: "sierpinski3d",
        label: "Sierpinski 3D",
        start: Start::Gl3d(sierpinski3d_start),
    },
    Entry {
        slug: "splitflap",
        label: "Split Flap",
        start: Start::Gl3d(splitflap_start),
    },
    Entry {
        slug: "splodesic",
        label: "Splodesic",
        start: Start::Gl3d(splodesic_start),
    },
    Entry {
        slug: "stairs",
        label: "Stairs",
        start: Start::Gl3d(stairs_start),
    },
    Entry {
        slug: "stonerview",
        label: "Stoner View",
        start: Start::Gl3d(stonerview_start),
    },
    Entry {
        slug: "skulloop",
        label: "Skulloop",
        start: Start::Gl3d(skulloop_start),
    },
    Entry {
        slug: "spheremonics",
        label: "Spheremonics",
        start: Start::Gl3d(spheremonics_start),
    },
    Entry {
        slug: "superquadrics",
        label: "Superquadrics",
        start: Start::Gl3d(superquadrics_start),
    },
    Entry {
        slug: "surfaces",
        label: "Surfaces",
        start: Start::Gl3d(surfaces_start),
    },
    Entry {
        slug: "engine",
        label: "Engine",
        start: Start::Gl3d(engine_start),
    },
    Entry {
        slug: "esper",
        label: "Esper",
        start: Start::Gl3d(esper_start),
    },
    Entry {
        slug: "etruscanvenus",
        label: "Etruscan Venus",
        start: Start::Gl3d(etruscanvenus_start),
    },
    Entry {
        slug: "geodesic",
        label: "Geodesic",
        start: Start::Gl3d(geodesic_start),
    },
    Entry {
        slug: "geodesicgears",
        label: "Geodesic Gears",
        start: Start::Gl3d(geodesicgears_start),
    },
    Entry {
        slug: "glforestfire",
        label: "GL Forest Fire",
        start: Start::Gl3d(glforestfire_start),
    },
    Entry {
        slug: "gleidescope",
        label: "Gleidescope",
        start: Start::Gl3d(gleidescope_start),
    },
    Entry {
        slug: "glslideshow",
        label: "GL Slideshow",
        start: Start::Gl3d(glslideshow_start),
    },
    Entry {
        slug: "hilbert",
        label: "Hilbert",
        start: Start::Gl3d(hilbert_start),
    },
    Entry {
        slug: "jigsaw",
        label: "Jigsaw",
        start: Start::Gl3d(jigsaw_start),
    },
    Entry {
        slug: "hydrostat",
        label: "Hydrostat",
        start: Start::Gl3d(hydrostat_start),
    },
    Entry {
        slug: "hypertorus",
        label: "Hypertorus",
        start: Start::Gl3d(hypertorus_start),
    },
    Entry {
        slug: "molecule",
        label: "Molecule",
        start: Start::Gl3d(molecule_start),
    },
    Entry {
        slug: "morph3d",
        label: "Morph 3D",
        start: Start::Gl3d(morph3d_start),
    },
    Entry {
        slug: "tangram",
        label: "Tangram",
        start: Start::Gl3d(tangram_start),
    },
    Entry {
        slug: "topblock",
        label: "Top Block",
        start: Start::Gl3d(topblock_start),
    },
    Entry {
        slug: "tronbit",
        label: "TronBit",
        start: Start::Gl3d(tronbit_start),
    },
    Entry {
        slug: "unknownpleasures",
        label: "Unknown Pleasures",
        start: Start::Gl3d(unknownpleasures_start),
    },
    Entry {
        slug: "vigilance",
        label: "Vigilance",
        start: Start::Gl3d(vigilance_start),
    },
    Entry {
        slug: "voronoi",
        label: "Voronoi",
        start: Start::Gl3d(voronoi_start),
    },
    Entry {
        slug: "alienbeacon",
        label: "Alien Beacon",
        start: Start::Gl(alienbeacon_start),
    },
    Entry {
        slug: "batteredplanet",
        label: "Battered Planet",
        start: Start::Gl(batteredplanet_start),
    },
    Entry {
        slug: "bestill",
        label: "Be Still",
        start: Start::Gl(bestill_start),
    },
    Entry {
        slug: "bubblecolors",
        label: "Bubble Colors",
        start: Start::Gl(bubblecolors_start),
    },
    Entry {
        slug: "darktransit",
        label: "Dark Transit",
        start: Start::Gl(darktransit_start),
    },
    Entry {
        slug: "downfall",
        label: "Downfall",
        start: Start::Gl(downfall_start),
    },
    Entry {
        slug: "driftclouds",
        label: "Drift Clouds",
        start: Start::Gl(driftclouds_start),
    },
    Entry {
        slug: "elementalring",
        label: "Elemental Ring",
        start: Start::Gl(elementalring_start),
    },
    Entry {
        slug: "fluxcore",
        label: "Flux Core",
        start: Start::Gl(fluxcore_start),
    },
    Entry {
        slug: "gimbalharmonics",
        label: "Gimbal Harmonics",
        start: Start::Gl(gimbalharmonics_start),
    },
    Entry {
        slug: "goldenapollian",
        label: "Golden Apollian",
        start: Start::Gl(goldenapollian_start),
    },
    Entry {
        slug: "hexplasma",
        label: "Hex Plasma",
        start: Start::Gl(hexplasma_start),
    },
    Entry {
        slug: "logarithmiccircles",
        label: "Logarithmic Circles",
        start: Start::Gl(logarithmiccircles_start),
    },
    Entry {
        slug: "neongravity",
        label: "Neon Gravity",
        start: Start::Gl(neongravity_start),
    },
    Entry {
        slug: "neontriangulator",
        label: "Neon Triangulator",
        start: Start::Gl(neontriangulator_start),
    },
    Entry {
        slug: "noxfire",
        label: "Nox Fire",
        start: Start::Gl(noxfire_start),
    },
    Entry {
        slug: "prococean",
        label: "Proc Ocean",
        start: Start::Gl(prococean_start),
    },
    Entry {
        slug: "protophore",
        label: "Protophore",
        start: Start::Gl(protophore_start),
    },
    Entry {
        slug: "rigrekt",
        label: "Rig Rekt",
        start: Start::Gl(rigrekt_start),
    },
    Entry {
        slug: "selfreflect",
        label: "Self Reflect",
        start: Start::Gl(selfreflect_start),
    },
    Entry {
        slug: "skyline",
        label: "Skyline",
        start: Start::Gl(skyline_start),
    },
    Entry {
        slug: "stardome",
        label: "Star Dome",
        start: Start::Gl(stardome_start),
    },
    Entry {
        slug: "starnest",
        label: "Star Nest",
        start: Start::Gl(starnest_start),
    },
    Entry {
        slug: "stripeytorus",
        label: "Stripey Torus",
        start: Start::Gl(stripeytorus_start),
    },
    Entry {
        slug: "synthwavecity",
        label: "Synthwave City",
        start: Start::Gl(synthwavecity_start),
    },
    Entry {
        slug: "topologica",
        label: "Topologica",
        start: Start::Gl(topologica_start),
    },
    Entry {
        slug: "trainmandala",
        label: "Train Mandala",
        start: Start::Gl(trainmandala_start),
    },
    Entry {
        slug: "trizm",
        label: "Trizm",
        start: Start::Gl(trizm_start),
    },
    Entry {
        slug: "truchetzoom",
        label: "Truchet Zoom",
        start: Start::Gl(truchetzoom_start),
    },
    Entry {
        slug: "universeball",
        label: "Universe Ball",
        start: Start::Gl(universeball_start),
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
