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
/// The same, for an OpenGL saver.
/// The same, for a Shadertoy saver. Its chunk holds the program text rather
/// than code, since the runner is shared and lives in the main module.
/// What a chunk hands back: the engine the saver actually started.
///
/// # Why the savers are grouped, and why only some of them are split
///
/// One chunk per saver is what this file did first, and `wasm-split` 0.7.9
/// cannot do it. Above a threshold it miscompiles the indirect function table:
/// every route then dispatches to the wrong component, the app rewrites its
/// own URL to `/screensaver` and sits on "Picking a screensaver" forever, with
/// no error in the console, no failed request, and a build that reports
/// success. Ten savers to a chunk keeps the table small enough to come out
/// right.
///
/// The limit is on how much code leaves the main module, not on how many
/// chunks there are. Measured: 312 savers in 8 chunks fails, in 16 fails, and
/// individually fails; 160 savers in 16 chunks works, 240 does not. So the
/// first 16 groups are split and the rest stay resident. That is worth
/// having: it takes the main module from 18.6 MB to 12.2 MB, which is 6.6 MB
/// to 4.8 MB once compressed.
///
/// Anchoring the shared runtime in main by keeping one saver of each engine
/// resident was tried and does not help; the runtime still leaves.
///
/// If you add savers, check the split still works before shipping. It fails
/// silently, and `nu test-browser.nu <slug>` catches it: a broken split makes
/// every saver render the same placeholder, so identical mean and spread
/// across two different savers is the signature.
#[cfg(feature = "split")]
#[allow(clippy::large_enum_variant)]
pub enum Started {
    Fb(Runner),
    Gl3d(Runner3d),
    Gl(Shadertoy),
}
fn ant_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::ant::start(args)
}
fn abstractile_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::abstractile::start(args)
}
fn anemone_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::anemone::start(args)
}
fn anemotaxis_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::anemotaxis::start(args)
}
fn apollonian_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::apollonian::start(args)
}
fn apple2_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::apple2::start(args)
}
fn attraction_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::attraction::start(args)
}
fn barcode_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::barcode::start(args)
}
fn binaryhorizon_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::binaryhorizon::start(args)
}
fn binaryring_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::binaryring::start(args)
}
#[cfg(feature = "split")]
fn group_0_body(args: (u16, StartArgs)) -> Option<Started> {
    let (i, a) = args;
    match i {
        0 => Some(Started::Fb(ant_body(a))),
        1 => Some(Started::Fb(abstractile_body(a))),
        2 => Some(Started::Fb(anemone_body(a))),
        3 => Some(Started::Fb(anemotaxis_body(a))),
        4 => Some(Started::Fb(apollonian_body(a))),
        5 => Some(Started::Fb(apple2_body(a))),
        6 => Some(Started::Fb(attraction_body(a))),
        7 => Some(Started::Fb(barcode_body(a))),
        8 => Some(Started::Fb(binaryhorizon_body(a))),
        9 => Some(Started::Fb(binaryring_body(a))),
        _ => None,
    }
}

#[cfg(feature = "split")]
static GROUP_0: wasm_split::LazyLoader<(u16, StartArgs), Option<Started>> = wasm_split::lazy_loader!(extern "group_0" fn group_0_body(props: (u16, StartArgs)) -> Option<Started>);
#[cfg(feature = "split")]
fn ant_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_0.load().await {
            return None;
        }
        match GROUP_0.call((0, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn ant_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(ant_body(args)) })
}
#[cfg(feature = "split")]
fn abstractile_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_0.load().await {
            return None;
        }
        match GROUP_0.call((1, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn abstractile_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(abstractile_body(args)) })
}
#[cfg(feature = "split")]
fn anemone_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_0.load().await {
            return None;
        }
        match GROUP_0.call((2, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn anemone_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(anemone_body(args)) })
}
#[cfg(feature = "split")]
fn anemotaxis_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_0.load().await {
            return None;
        }
        match GROUP_0.call((3, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn anemotaxis_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(anemotaxis_body(args)) })
}
#[cfg(feature = "split")]
fn apollonian_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_0.load().await {
            return None;
        }
        match GROUP_0.call((4, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn apollonian_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(apollonian_body(args)) })
}
#[cfg(feature = "split")]
fn apple2_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_0.load().await {
            return None;
        }
        match GROUP_0.call((5, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn apple2_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(apple2_body(args)) })
}
#[cfg(feature = "split")]
fn attraction_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_0.load().await {
            return None;
        }
        match GROUP_0.call((6, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn attraction_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(attraction_body(args)) })
}
#[cfg(feature = "split")]
fn barcode_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_0.load().await {
            return None;
        }
        match GROUP_0.call((7, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn barcode_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(barcode_body(args)) })
}
#[cfg(feature = "split")]
fn binaryhorizon_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_0.load().await {
            return None;
        }
        match GROUP_0.call((8, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn binaryhorizon_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(binaryhorizon_body(args)) })
}
#[cfg(feature = "split")]
fn binaryring_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_0.load().await {
            return None;
        }
        match GROUP_0.call((9, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn binaryring_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(binaryring_body(args)) })
}
fn blaster_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::blaster::start(args)
}
fn blitspin_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::blitspin::start(args)
}
fn bsod_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::bsod::start(args)
}
fn bouboule_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::bouboule::start(args)
}
fn boxfit_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::boxfit::start(args)
}
fn braid_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::braid::start(args)
}
fn bubbles_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::bubbles::start(args)
}
fn bumps_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::bumps::start(args)
}
fn ccurve_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::ccurve::start(args)
}
fn celtic_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::celtic::start(args)
}
#[cfg(feature = "split")]
fn group_1_body(args: (u16, StartArgs)) -> Option<Started> {
    let (i, a) = args;
    match i {
        0 => Some(Started::Fb(blaster_body(a))),
        1 => Some(Started::Fb(blitspin_body(a))),
        2 => Some(Started::Fb(bsod_body(a))),
        3 => Some(Started::Fb(bouboule_body(a))),
        4 => Some(Started::Fb(boxfit_body(a))),
        5 => Some(Started::Fb(braid_body(a))),
        6 => Some(Started::Fb(bubbles_body(a))),
        7 => Some(Started::Fb(bumps_body(a))),
        8 => Some(Started::Fb(ccurve_body(a))),
        9 => Some(Started::Fb(celtic_body(a))),
        _ => None,
    }
}

#[cfg(feature = "split")]
static GROUP_1: wasm_split::LazyLoader<(u16, StartArgs), Option<Started>> = wasm_split::lazy_loader!(extern "group_1" fn group_1_body(props: (u16, StartArgs)) -> Option<Started>);
#[cfg(feature = "split")]
fn blaster_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_1.load().await {
            return None;
        }
        match GROUP_1.call((0, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn blaster_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(blaster_body(args)) })
}
#[cfg(feature = "split")]
fn blitspin_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_1.load().await {
            return None;
        }
        match GROUP_1.call((1, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn blitspin_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(blitspin_body(args)) })
}
#[cfg(feature = "split")]
fn bsod_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_1.load().await {
            return None;
        }
        match GROUP_1.call((2, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn bsod_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(bsod_body(args)) })
}
#[cfg(feature = "split")]
fn bouboule_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_1.load().await {
            return None;
        }
        match GROUP_1.call((3, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn bouboule_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(bouboule_body(args)) })
}
#[cfg(feature = "split")]
fn boxfit_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_1.load().await {
            return None;
        }
        match GROUP_1.call((4, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn boxfit_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(boxfit_body(args)) })
}
#[cfg(feature = "split")]
fn braid_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_1.load().await {
            return None;
        }
        match GROUP_1.call((5, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn braid_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(braid_body(args)) })
}
#[cfg(feature = "split")]
fn bubbles_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_1.load().await {
            return None;
        }
        match GROUP_1.call((6, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn bubbles_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(bubbles_body(args)) })
}
#[cfg(feature = "split")]
fn bumps_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_1.load().await {
            return None;
        }
        match GROUP_1.call((7, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn bumps_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(bumps_body(args)) })
}
#[cfg(feature = "split")]
fn ccurve_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_1.load().await {
            return None;
        }
        match GROUP_1.call((8, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn ccurve_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(ccurve_body(args)) })
}
#[cfg(feature = "split")]
fn celtic_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_1.load().await {
            return None;
        }
        match GROUP_1.call((9, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn celtic_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(celtic_body(args)) })
}
fn cloudlife_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::cloudlife::start(args)
}
fn compass_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::compass::start(args)
}
fn coral_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::coral::start(args)
}
fn critical_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::critical::start(args)
}
fn crystal_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::crystal::start(args)
}
fn cwaves_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::cwaves::start(args)
}
fn deco_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::deco::start(args)
}
fn cynosure_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::cynosure::start(args)
}
fn decayscreen_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::decayscreen::start(args)
}
fn deluxe_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::deluxe::start(args)
}
#[cfg(feature = "split")]
fn group_2_body(args: (u16, StartArgs)) -> Option<Started> {
    let (i, a) = args;
    match i {
        0 => Some(Started::Fb(cloudlife_body(a))),
        1 => Some(Started::Fb(compass_body(a))),
        2 => Some(Started::Fb(coral_body(a))),
        3 => Some(Started::Fb(critical_body(a))),
        4 => Some(Started::Fb(crystal_body(a))),
        5 => Some(Started::Fb(cwaves_body(a))),
        6 => Some(Started::Fb(deco_body(a))),
        7 => Some(Started::Fb(cynosure_body(a))),
        8 => Some(Started::Fb(decayscreen_body(a))),
        9 => Some(Started::Fb(deluxe_body(a))),
        _ => None,
    }
}

#[cfg(feature = "split")]
static GROUP_2: wasm_split::LazyLoader<(u16, StartArgs), Option<Started>> = wasm_split::lazy_loader!(extern "group_2" fn group_2_body(props: (u16, StartArgs)) -> Option<Started>);
#[cfg(feature = "split")]
fn cloudlife_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_2.load().await {
            return None;
        }
        match GROUP_2.call((0, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn cloudlife_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(cloudlife_body(args)) })
}
#[cfg(feature = "split")]
fn compass_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_2.load().await {
            return None;
        }
        match GROUP_2.call((1, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn compass_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(compass_body(args)) })
}
#[cfg(feature = "split")]
fn coral_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_2.load().await {
            return None;
        }
        match GROUP_2.call((2, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn coral_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(coral_body(args)) })
}
#[cfg(feature = "split")]
fn critical_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_2.load().await {
            return None;
        }
        match GROUP_2.call((3, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn critical_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(critical_body(args)) })
}
#[cfg(feature = "split")]
fn crystal_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_2.load().await {
            return None;
        }
        match GROUP_2.call((4, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn crystal_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(crystal_body(args)) })
}
#[cfg(feature = "split")]
fn cwaves_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_2.load().await {
            return None;
        }
        match GROUP_2.call((5, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn cwaves_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(cwaves_body(args)) })
}
#[cfg(feature = "split")]
fn deco_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_2.load().await {
            return None;
        }
        match GROUP_2.call((6, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn deco_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(deco_body(args)) })
}
#[cfg(feature = "split")]
fn cynosure_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_2.load().await {
            return None;
        }
        match GROUP_2.call((7, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn cynosure_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(cynosure_body(args)) })
}
#[cfg(feature = "split")]
fn decayscreen_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_2.load().await {
            return None;
        }
        match GROUP_2.call((8, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn decayscreen_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(decayscreen_body(args)) })
}
#[cfg(feature = "split")]
fn deluxe_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_2.load().await {
            return None;
        }
        match GROUP_2.call((9, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn deluxe_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(deluxe_body(args)) })
}
fn demon_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::demon::start(args)
}
fn discrete_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::discrete::start(args)
}
fn fuzzyflakes_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::fuzzyflakes::start(args)
}
fn galaxy_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::galaxy::start(args)
}
fn greynetic_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::greynetic::start(args)
}
fn distort_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::distort::start(args)
}
fn drift_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::drift::start(args)
}
fn droste_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::droste::start(args)
}
fn epicycle_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::epicycle::start(args)
}
fn euler2d_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::euler2d::start(args)
}
#[cfg(feature = "split")]
fn group_3_body(args: (u16, StartArgs)) -> Option<Started> {
    let (i, a) = args;
    match i {
        0 => Some(Started::Fb(demon_body(a))),
        1 => Some(Started::Fb(discrete_body(a))),
        2 => Some(Started::Fb(fuzzyflakes_body(a))),
        3 => Some(Started::Fb(galaxy_body(a))),
        4 => Some(Started::Fb(greynetic_body(a))),
        5 => Some(Started::Fb(distort_body(a))),
        6 => Some(Started::Fb(drift_body(a))),
        7 => Some(Started::Fb(droste_body(a))),
        8 => Some(Started::Fb(epicycle_body(a))),
        9 => Some(Started::Fb(euler2d_body(a))),
        _ => None,
    }
}

#[cfg(feature = "split")]
static GROUP_3: wasm_split::LazyLoader<(u16, StartArgs), Option<Started>> = wasm_split::lazy_loader!(extern "group_3" fn group_3_body(props: (u16, StartArgs)) -> Option<Started>);
#[cfg(feature = "split")]
fn demon_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_3.load().await {
            return None;
        }
        match GROUP_3.call((0, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn demon_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(demon_body(args)) })
}
#[cfg(feature = "split")]
fn discrete_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_3.load().await {
            return None;
        }
        match GROUP_3.call((1, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn discrete_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(discrete_body(args)) })
}
#[cfg(feature = "split")]
fn fuzzyflakes_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_3.load().await {
            return None;
        }
        match GROUP_3.call((2, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn fuzzyflakes_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(fuzzyflakes_body(args)) })
}
#[cfg(feature = "split")]
fn galaxy_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_3.load().await {
            return None;
        }
        match GROUP_3.call((3, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn galaxy_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(galaxy_body(args)) })
}
#[cfg(feature = "split")]
fn greynetic_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_3.load().await {
            return None;
        }
        match GROUP_3.call((4, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn greynetic_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(greynetic_body(args)) })
}
#[cfg(feature = "split")]
fn distort_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_3.load().await {
            return None;
        }
        match GROUP_3.call((5, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn distort_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(distort_body(args)) })
}
#[cfg(feature = "split")]
fn drift_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_3.load().await {
            return None;
        }
        match GROUP_3.call((6, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn drift_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(drift_body(args)) })
}
#[cfg(feature = "split")]
fn droste_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_3.load().await {
            return None;
        }
        match GROUP_3.call((7, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn droste_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(droste_body(args)) })
}
#[cfg(feature = "split")]
fn epicycle_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_3.load().await {
            return None;
        }
        match GROUP_3.call((8, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn epicycle_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(epicycle_body(args)) })
}
#[cfg(feature = "split")]
fn euler2d_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_3.load().await {
            return None;
        }
        match GROUP_3.call((9, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn euler2d_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(euler2d_body(args)) })
}
fn eruption_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::eruption::start(args)
}
fn fadeplot_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::fadeplot::start(args)
}
fn fiberlamp_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::fiberlamp::start(args)
}
fn filmleader_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::filmleader::start(args)
}
fn fireworkx_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::fireworkx::start(args)
}
fn flag_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::flag::start(args)
}
fn flame_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::flame::start(args)
}
fn flow_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::flow::start(args)
}
fn fluidballs_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::fluidballs::start(args)
}
fn fontglide_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::fontglide::start(args)
}
#[cfg(feature = "split")]
fn group_4_body(args: (u16, StartArgs)) -> Option<Started> {
    let (i, a) = args;
    match i {
        0 => Some(Started::Fb(eruption_body(a))),
        1 => Some(Started::Fb(fadeplot_body(a))),
        2 => Some(Started::Fb(fiberlamp_body(a))),
        3 => Some(Started::Fb(filmleader_body(a))),
        4 => Some(Started::Fb(fireworkx_body(a))),
        5 => Some(Started::Fb(flag_body(a))),
        6 => Some(Started::Fb(flame_body(a))),
        7 => Some(Started::Fb(flow_body(a))),
        8 => Some(Started::Fb(fluidballs_body(a))),
        9 => Some(Started::Fb(fontglide_body(a))),
        _ => None,
    }
}

#[cfg(feature = "split")]
static GROUP_4: wasm_split::LazyLoader<(u16, StartArgs), Option<Started>> = wasm_split::lazy_loader!(extern "group_4" fn group_4_body(props: (u16, StartArgs)) -> Option<Started>);
#[cfg(feature = "split")]
fn eruption_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_4.load().await {
            return None;
        }
        match GROUP_4.call((0, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn eruption_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(eruption_body(args)) })
}
#[cfg(feature = "split")]
fn fadeplot_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_4.load().await {
            return None;
        }
        match GROUP_4.call((1, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn fadeplot_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(fadeplot_body(args)) })
}
#[cfg(feature = "split")]
fn fiberlamp_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_4.load().await {
            return None;
        }
        match GROUP_4.call((2, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn fiberlamp_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(fiberlamp_body(args)) })
}
#[cfg(feature = "split")]
fn filmleader_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_4.load().await {
            return None;
        }
        match GROUP_4.call((3, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn filmleader_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(filmleader_body(args)) })
}
#[cfg(feature = "split")]
fn fireworkx_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_4.load().await {
            return None;
        }
        match GROUP_4.call((4, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn fireworkx_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(fireworkx_body(args)) })
}
#[cfg(feature = "split")]
fn flag_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_4.load().await {
            return None;
        }
        match GROUP_4.call((5, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn flag_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(flag_body(args)) })
}
#[cfg(feature = "split")]
fn flame_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_4.load().await {
            return None;
        }
        match GROUP_4.call((6, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn flame_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(flame_body(args)) })
}
#[cfg(feature = "split")]
fn flow_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_4.load().await {
            return None;
        }
        match GROUP_4.call((7, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn flow_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(flow_body(args)) })
}
#[cfg(feature = "split")]
fn fluidballs_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_4.load().await {
            return None;
        }
        match GROUP_4.call((8, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn fluidballs_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(fluidballs_body(args)) })
}
#[cfg(feature = "split")]
fn fontglide_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_4.load().await {
            return None;
        }
        match GROUP_4.call((9, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn fontglide_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(fontglide_body(args)) })
}
fn forest_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::forest::start(args)
}
fn glitchpeg_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::glitchpeg::start(args)
}
fn goop_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::goop::start(args)
}
fn grav_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::grav::start(args)
}
fn halo_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::halo::start(args)
}
fn halftone_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::halftone::start(args)
}
fn helix_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::helix::start(args)
}
fn hexadrop_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::hexadrop::start(args)
}
fn hopalong_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::hopalong::start(args)
}
fn hyperball_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::hyperball::start(args)
}
#[cfg(feature = "split")]
fn group_5_body(args: (u16, StartArgs)) -> Option<Started> {
    let (i, a) = args;
    match i {
        0 => Some(Started::Fb(forest_body(a))),
        1 => Some(Started::Fb(glitchpeg_body(a))),
        2 => Some(Started::Fb(goop_body(a))),
        3 => Some(Started::Fb(grav_body(a))),
        4 => Some(Started::Fb(halo_body(a))),
        5 => Some(Started::Fb(halftone_body(a))),
        6 => Some(Started::Fb(helix_body(a))),
        7 => Some(Started::Fb(hexadrop_body(a))),
        8 => Some(Started::Fb(hopalong_body(a))),
        9 => Some(Started::Fb(hyperball_body(a))),
        _ => None,
    }
}

#[cfg(feature = "split")]
static GROUP_5: wasm_split::LazyLoader<(u16, StartArgs), Option<Started>> = wasm_split::lazy_loader!(extern "group_5" fn group_5_body(props: (u16, StartArgs)) -> Option<Started>);
#[cfg(feature = "split")]
fn forest_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_5.load().await {
            return None;
        }
        match GROUP_5.call((0, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn forest_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(forest_body(args)) })
}
#[cfg(feature = "split")]
fn glitchpeg_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_5.load().await {
            return None;
        }
        match GROUP_5.call((1, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn glitchpeg_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(glitchpeg_body(args)) })
}
#[cfg(feature = "split")]
fn goop_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_5.load().await {
            return None;
        }
        match GROUP_5.call((2, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn goop_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(goop_body(args)) })
}
#[cfg(feature = "split")]
fn grav_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_5.load().await {
            return None;
        }
        match GROUP_5.call((3, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn grav_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(grav_body(args)) })
}
#[cfg(feature = "split")]
fn halo_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_5.load().await {
            return None;
        }
        match GROUP_5.call((4, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn halo_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(halo_body(args)) })
}
#[cfg(feature = "split")]
fn halftone_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_5.load().await {
            return None;
        }
        match GROUP_5.call((5, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn halftone_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(halftone_body(args)) })
}
#[cfg(feature = "split")]
fn helix_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_5.load().await {
            return None;
        }
        match GROUP_5.call((6, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn helix_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(helix_body(args)) })
}
#[cfg(feature = "split")]
fn hexadrop_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_5.load().await {
            return None;
        }
        match GROUP_5.call((7, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn hexadrop_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(hexadrop_body(args)) })
}
#[cfg(feature = "split")]
fn hopalong_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_5.load().await {
            return None;
        }
        match GROUP_5.call((8, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn hopalong_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(hopalong_body(args)) })
}
#[cfg(feature = "split")]
fn hyperball_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_5.load().await {
            return None;
        }
        match GROUP_5.call((9, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn hyperball_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(hyperball_body(args)) })
}
fn hypercube_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::hypercube::start(args)
}
fn ifs_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::ifs::start(args)
}
fn imsmap_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::imsmap::start(args)
}
fn interaggregate_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::interaggregate::start(args)
}
fn interference_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::interference::start(args)
}
fn intermomentary_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::intermomentary::start(args)
}
fn juggle_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::juggle::start(args)
}
fn julia_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::julia::start(args)
}
fn kaleidescope_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::kaleidescope::start(args)
}
fn laser_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::laser::start(args)
}
#[cfg(feature = "split")]
fn group_6_body(args: (u16, StartArgs)) -> Option<Started> {
    let (i, a) = args;
    match i {
        0 => Some(Started::Fb(hypercube_body(a))),
        1 => Some(Started::Fb(ifs_body(a))),
        2 => Some(Started::Fb(imsmap_body(a))),
        3 => Some(Started::Fb(interaggregate_body(a))),
        4 => Some(Started::Fb(interference_body(a))),
        5 => Some(Started::Fb(intermomentary_body(a))),
        6 => Some(Started::Fb(juggle_body(a))),
        7 => Some(Started::Fb(julia_body(a))),
        8 => Some(Started::Fb(kaleidescope_body(a))),
        9 => Some(Started::Fb(laser_body(a))),
        _ => None,
    }
}

#[cfg(feature = "split")]
static GROUP_6: wasm_split::LazyLoader<(u16, StartArgs), Option<Started>> = wasm_split::lazy_loader!(extern "group_6" fn group_6_body(props: (u16, StartArgs)) -> Option<Started>);
#[cfg(feature = "split")]
fn hypercube_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_6.load().await {
            return None;
        }
        match GROUP_6.call((0, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn hypercube_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(hypercube_body(args)) })
}
#[cfg(feature = "split")]
fn ifs_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_6.load().await {
            return None;
        }
        match GROUP_6.call((1, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn ifs_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(ifs_body(args)) })
}
#[cfg(feature = "split")]
fn imsmap_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_6.load().await {
            return None;
        }
        match GROUP_6.call((2, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn imsmap_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(imsmap_body(args)) })
}
#[cfg(feature = "split")]
fn interaggregate_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_6.load().await {
            return None;
        }
        match GROUP_6.call((3, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn interaggregate_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(interaggregate_body(args)) })
}
#[cfg(feature = "split")]
fn interference_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_6.load().await {
            return None;
        }
        match GROUP_6.call((4, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn interference_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(interference_body(args)) })
}
#[cfg(feature = "split")]
fn intermomentary_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_6.load().await {
            return None;
        }
        match GROUP_6.call((5, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn intermomentary_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(intermomentary_body(args)) })
}
#[cfg(feature = "split")]
fn juggle_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_6.load().await {
            return None;
        }
        match GROUP_6.call((6, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn juggle_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(juggle_body(args)) })
}
#[cfg(feature = "split")]
fn julia_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_6.load().await {
            return None;
        }
        match GROUP_6.call((7, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn julia_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(julia_body(args)) })
}
#[cfg(feature = "split")]
fn kaleidescope_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_6.load().await {
            return None;
        }
        match GROUP_6.call((8, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn kaleidescope_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(kaleidescope_body(args)) })
}
#[cfg(feature = "split")]
fn laser_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_6.load().await {
            return None;
        }
        match GROUP_6.call((9, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn laser_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(laser_body(args)) })
}
fn kumppa_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::kumppa::start(args)
}
fn lcdscrub_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::lcdscrub::start(args)
}
fn lightning_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::lightning::start(args)
}
fn lisa_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::lisa::start(args)
}
fn lissie_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::lissie::start(args)
}
fn mismunch_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::mismunch::start(args)
}
fn moire_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::moire::start(args)
}
fn lmorph_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::lmorph::start(args)
}
fn loop_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::r#loop::start(args)
}
fn m6502_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::m6502::start(args)
}
#[cfg(feature = "split")]
fn group_7_body(args: (u16, StartArgs)) -> Option<Started> {
    let (i, a) = args;
    match i {
        0 => Some(Started::Fb(kumppa_body(a))),
        1 => Some(Started::Fb(lcdscrub_body(a))),
        2 => Some(Started::Fb(lightning_body(a))),
        3 => Some(Started::Fb(lisa_body(a))),
        4 => Some(Started::Fb(lissie_body(a))),
        5 => Some(Started::Fb(mismunch_body(a))),
        6 => Some(Started::Fb(moire_body(a))),
        7 => Some(Started::Fb(lmorph_body(a))),
        8 => Some(Started::Fb(loop_body(a))),
        9 => Some(Started::Fb(m6502_body(a))),
        _ => None,
    }
}

#[cfg(feature = "split")]
static GROUP_7: wasm_split::LazyLoader<(u16, StartArgs), Option<Started>> = wasm_split::lazy_loader!(extern "group_7" fn group_7_body(props: (u16, StartArgs)) -> Option<Started>);
#[cfg(feature = "split")]
fn kumppa_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_7.load().await {
            return None;
        }
        match GROUP_7.call((0, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn kumppa_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(kumppa_body(args)) })
}
#[cfg(feature = "split")]
fn lcdscrub_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_7.load().await {
            return None;
        }
        match GROUP_7.call((1, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn lcdscrub_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(lcdscrub_body(args)) })
}
#[cfg(feature = "split")]
fn lightning_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_7.load().await {
            return None;
        }
        match GROUP_7.call((2, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn lightning_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(lightning_body(args)) })
}
#[cfg(feature = "split")]
fn lisa_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_7.load().await {
            return None;
        }
        match GROUP_7.call((3, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn lisa_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(lisa_body(args)) })
}
#[cfg(feature = "split")]
fn lissie_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_7.load().await {
            return None;
        }
        match GROUP_7.call((4, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn lissie_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(lissie_body(args)) })
}
#[cfg(feature = "split")]
fn mismunch_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_7.load().await {
            return None;
        }
        match GROUP_7.call((5, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn mismunch_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(mismunch_body(args)) })
}
#[cfg(feature = "split")]
fn moire_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_7.load().await {
            return None;
        }
        match GROUP_7.call((6, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn moire_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(moire_body(args)) })
}
#[cfg(feature = "split")]
fn lmorph_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_7.load().await {
            return None;
        }
        match GROUP_7.call((7, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn lmorph_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(lmorph_body(args)) })
}
#[cfg(feature = "split")]
fn loop_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_7.load().await {
            return None;
        }
        match GROUP_7.call((8, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn loop_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(loop_body(args)) })
}
#[cfg(feature = "split")]
fn m6502_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_7.load().await {
            return None;
        }
        match GROUP_7.call((9, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn m6502_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(m6502_body(args)) })
}
fn marbling_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::marbling::start(args)
}
fn maze_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::maze::start(args)
}
fn memscroller_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::memscroller::start(args)
}
fn metaballs_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::metaballs::start(args)
}
fn moire2_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::moire2::start(args)
}
fn mountain_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::mountain::start(args)
}
fn munch_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::munch::start(args)
}
fn nerverot_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::nerverot::start(args)
}
fn noseguy_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::noseguy::start(args)
}
fn pacman_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::pacman::start(args)
}
#[cfg(feature = "split")]
fn group_8_body(args: (u16, StartArgs)) -> Option<Started> {
    let (i, a) = args;
    match i {
        0 => Some(Started::Fb(marbling_body(a))),
        1 => Some(Started::Fb(maze_body(a))),
        2 => Some(Started::Fb(memscroller_body(a))),
        3 => Some(Started::Fb(metaballs_body(a))),
        4 => Some(Started::Fb(moire2_body(a))),
        5 => Some(Started::Fb(mountain_body(a))),
        6 => Some(Started::Fb(munch_body(a))),
        7 => Some(Started::Fb(nerverot_body(a))),
        8 => Some(Started::Fb(noseguy_body(a))),
        9 => Some(Started::Fb(pacman_body(a))),
        _ => None,
    }
}

#[cfg(feature = "split")]
static GROUP_8: wasm_split::LazyLoader<(u16, StartArgs), Option<Started>> = wasm_split::lazy_loader!(extern "group_8" fn group_8_body(props: (u16, StartArgs)) -> Option<Started>);
#[cfg(feature = "split")]
fn marbling_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_8.load().await {
            return None;
        }
        match GROUP_8.call((0, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn marbling_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(marbling_body(args)) })
}
#[cfg(feature = "split")]
fn maze_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_8.load().await {
            return None;
        }
        match GROUP_8.call((1, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn maze_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(maze_body(args)) })
}
#[cfg(feature = "split")]
fn memscroller_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_8.load().await {
            return None;
        }
        match GROUP_8.call((2, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn memscroller_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(memscroller_body(args)) })
}
#[cfg(feature = "split")]
fn metaballs_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_8.load().await {
            return None;
        }
        match GROUP_8.call((3, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn metaballs_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(metaballs_body(args)) })
}
#[cfg(feature = "split")]
fn moire2_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_8.load().await {
            return None;
        }
        match GROUP_8.call((4, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn moire2_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(moire2_body(args)) })
}
#[cfg(feature = "split")]
fn mountain_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_8.load().await {
            return None;
        }
        match GROUP_8.call((5, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn mountain_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(mountain_body(args)) })
}
#[cfg(feature = "split")]
fn munch_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_8.load().await {
            return None;
        }
        match GROUP_8.call((6, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn munch_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(munch_body(args)) })
}
#[cfg(feature = "split")]
fn nerverot_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_8.load().await {
            return None;
        }
        match GROUP_8.call((7, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn nerverot_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(nerverot_body(args)) })
}
#[cfg(feature = "split")]
fn noseguy_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_8.load().await {
            return None;
        }
        match GROUP_8.call((8, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn noseguy_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(noseguy_body(args)) })
}
#[cfg(feature = "split")]
fn pacman_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_8.load().await {
            return None;
        }
        match GROUP_8.call((9, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn pacman_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(pacman_body(args)) })
}
fn pedal_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::pedal::start(args)
}
fn penetrate_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::penetrate::start(args)
}
fn penrose_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::penrose::start(args)
}
fn petri_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::petri::start(args)
}
fn phosphor_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::phosphor::start(args)
}
fn piecewise_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::piecewise::start(args)
}
fn polyominoes_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::polyominoes::start(args)
}
fn pong_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::pong::start(args)
}
fn popsquares_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::popsquares::start(args)
}
fn pyro_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::pyro::start(args)
}
#[cfg(feature = "split")]
fn group_9_body(args: (u16, StartArgs)) -> Option<Started> {
    let (i, a) = args;
    match i {
        0 => Some(Started::Fb(pedal_body(a))),
        1 => Some(Started::Fb(penetrate_body(a))),
        2 => Some(Started::Fb(penrose_body(a))),
        3 => Some(Started::Fb(petri_body(a))),
        4 => Some(Started::Fb(phosphor_body(a))),
        5 => Some(Started::Fb(piecewise_body(a))),
        6 => Some(Started::Fb(polyominoes_body(a))),
        7 => Some(Started::Fb(pong_body(a))),
        8 => Some(Started::Fb(popsquares_body(a))),
        9 => Some(Started::Fb(pyro_body(a))),
        _ => None,
    }
}

#[cfg(feature = "split")]
static GROUP_9: wasm_split::LazyLoader<(u16, StartArgs), Option<Started>> = wasm_split::lazy_loader!(extern "group_9" fn group_9_body(props: (u16, StartArgs)) -> Option<Started>);
#[cfg(feature = "split")]
fn pedal_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_9.load().await {
            return None;
        }
        match GROUP_9.call((0, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn pedal_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(pedal_body(args)) })
}
#[cfg(feature = "split")]
fn penetrate_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_9.load().await {
            return None;
        }
        match GROUP_9.call((1, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn penetrate_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(penetrate_body(args)) })
}
#[cfg(feature = "split")]
fn penrose_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_9.load().await {
            return None;
        }
        match GROUP_9.call((2, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn penrose_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(penrose_body(args)) })
}
#[cfg(feature = "split")]
fn petri_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_9.load().await {
            return None;
        }
        match GROUP_9.call((3, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn petri_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(petri_body(args)) })
}
#[cfg(feature = "split")]
fn phosphor_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_9.load().await {
            return None;
        }
        match GROUP_9.call((4, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn phosphor_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(phosphor_body(args)) })
}
#[cfg(feature = "split")]
fn piecewise_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_9.load().await {
            return None;
        }
        match GROUP_9.call((5, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn piecewise_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(piecewise_body(args)) })
}
#[cfg(feature = "split")]
fn polyominoes_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_9.load().await {
            return None;
        }
        match GROUP_9.call((6, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn polyominoes_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(polyominoes_body(args)) })
}
#[cfg(feature = "split")]
fn pong_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_9.load().await {
            return None;
        }
        match GROUP_9.call((7, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn pong_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(pong_body(args)) })
}
#[cfg(feature = "split")]
fn popsquares_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_9.load().await {
            return None;
        }
        match GROUP_9.call((8, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn popsquares_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(popsquares_body(args)) })
}
#[cfg(feature = "split")]
fn pyro_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_9.load().await {
            return None;
        }
        match GROUP_9.call((9, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn pyro_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(pyro_body(args)) })
}
fn qix_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::qix::start(args)
}
fn rdbomb_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::rdbomb::start(args)
}
fn ripples_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::ripples::start(args)
}
fn rocks_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::rocks::start(args)
}
fn rorschach_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::rorschach::start(args)
}
fn rotor_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::rotor::start(args)
}
fn scooter_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::scooter::start(args)
}
fn shadebobs_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::shadebobs::start(args)
}
fn rotzoomer_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::rotzoomer::start(args)
}
fn sierpinski_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::sierpinski::start(args)
}
#[cfg(feature = "split")]
fn group_10_body(args: (u16, StartArgs)) -> Option<Started> {
    let (i, a) = args;
    match i {
        0 => Some(Started::Fb(qix_body(a))),
        1 => Some(Started::Fb(rdbomb_body(a))),
        2 => Some(Started::Fb(ripples_body(a))),
        3 => Some(Started::Fb(rocks_body(a))),
        4 => Some(Started::Fb(rorschach_body(a))),
        5 => Some(Started::Fb(rotor_body(a))),
        6 => Some(Started::Fb(scooter_body(a))),
        7 => Some(Started::Fb(shadebobs_body(a))),
        8 => Some(Started::Fb(rotzoomer_body(a))),
        9 => Some(Started::Fb(sierpinski_body(a))),
        _ => None,
    }
}

#[cfg(feature = "split")]
static GROUP_10: wasm_split::LazyLoader<(u16, StartArgs), Option<Started>> = wasm_split::lazy_loader!(extern "group_10" fn group_10_body(props: (u16, StartArgs)) -> Option<Started>);
#[cfg(feature = "split")]
fn qix_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_10.load().await {
            return None;
        }
        match GROUP_10.call((0, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn qix_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(qix_body(args)) })
}
#[cfg(feature = "split")]
fn rdbomb_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_10.load().await {
            return None;
        }
        match GROUP_10.call((1, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn rdbomb_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(rdbomb_body(args)) })
}
#[cfg(feature = "split")]
fn ripples_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_10.load().await {
            return None;
        }
        match GROUP_10.call((2, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn ripples_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(ripples_body(args)) })
}
#[cfg(feature = "split")]
fn rocks_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_10.load().await {
            return None;
        }
        match GROUP_10.call((3, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn rocks_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(rocks_body(args)) })
}
#[cfg(feature = "split")]
fn rorschach_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_10.load().await {
            return None;
        }
        match GROUP_10.call((4, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn rorschach_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(rorschach_body(args)) })
}
#[cfg(feature = "split")]
fn rotor_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_10.load().await {
            return None;
        }
        match GROUP_10.call((5, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn rotor_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(rotor_body(args)) })
}
#[cfg(feature = "split")]
fn scooter_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_10.load().await {
            return None;
        }
        match GROUP_10.call((6, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn scooter_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(scooter_body(args)) })
}
#[cfg(feature = "split")]
fn shadebobs_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_10.load().await {
            return None;
        }
        match GROUP_10.call((7, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn shadebobs_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(shadebobs_body(args)) })
}
#[cfg(feature = "split")]
fn rotzoomer_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_10.load().await {
            return None;
        }
        match GROUP_10.call((8, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn rotzoomer_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(rotzoomer_body(args)) })
}
#[cfg(feature = "split")]
fn sierpinski_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_10.load().await {
            return None;
        }
        match GROUP_10.call((9, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn sierpinski_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(sierpinski_body(args)) })
}
fn slidescreen_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::slidescreen::start(args)
}
fn slip_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::slip::start(args)
}
fn speedmine_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::speedmine::start(args)
}
fn sphere_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::sphere::start(args)
}
fn spiral_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::spiral::start(args)
}
fn spotlight_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::spotlight::start(args)
}
fn squiral_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::squiral::start(args)
}
fn starfish_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::starfish::start(args)
}
fn strange_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::strange::start(args)
}
fn substrate_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::substrate::start(args)
}
#[cfg(feature = "split")]
fn group_11_body(args: (u16, StartArgs)) -> Option<Started> {
    let (i, a) = args;
    match i {
        0 => Some(Started::Fb(slidescreen_body(a))),
        1 => Some(Started::Fb(slip_body(a))),
        2 => Some(Started::Fb(speedmine_body(a))),
        3 => Some(Started::Fb(sphere_body(a))),
        4 => Some(Started::Fb(spiral_body(a))),
        5 => Some(Started::Fb(spotlight_body(a))),
        6 => Some(Started::Fb(squiral_body(a))),
        7 => Some(Started::Fb(starfish_body(a))),
        8 => Some(Started::Fb(strange_body(a))),
        9 => Some(Started::Fb(substrate_body(a))),
        _ => None,
    }
}

#[cfg(feature = "split")]
static GROUP_11: wasm_split::LazyLoader<(u16, StartArgs), Option<Started>> = wasm_split::lazy_loader!(extern "group_11" fn group_11_body(props: (u16, StartArgs)) -> Option<Started>);
#[cfg(feature = "split")]
fn slidescreen_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_11.load().await {
            return None;
        }
        match GROUP_11.call((0, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn slidescreen_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(slidescreen_body(args)) })
}
#[cfg(feature = "split")]
fn slip_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_11.load().await {
            return None;
        }
        match GROUP_11.call((1, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn slip_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(slip_body(args)) })
}
#[cfg(feature = "split")]
fn speedmine_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_11.load().await {
            return None;
        }
        match GROUP_11.call((2, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn speedmine_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(speedmine_body(args)) })
}
#[cfg(feature = "split")]
fn sphere_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_11.load().await {
            return None;
        }
        match GROUP_11.call((3, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn sphere_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(sphere_body(args)) })
}
#[cfg(feature = "split")]
fn spiral_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_11.load().await {
            return None;
        }
        match GROUP_11.call((4, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn spiral_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(spiral_body(args)) })
}
#[cfg(feature = "split")]
fn spotlight_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_11.load().await {
            return None;
        }
        match GROUP_11.call((5, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn spotlight_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(spotlight_body(args)) })
}
#[cfg(feature = "split")]
fn squiral_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_11.load().await {
            return None;
        }
        match GROUP_11.call((6, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn squiral_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(squiral_body(args)) })
}
#[cfg(feature = "split")]
fn starfish_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_11.load().await {
            return None;
        }
        match GROUP_11.call((7, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn starfish_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(starfish_body(args)) })
}
#[cfg(feature = "split")]
fn strange_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_11.load().await {
            return None;
        }
        match GROUP_11.call((8, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn strange_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(strange_body(args)) })
}
#[cfg(feature = "split")]
fn substrate_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_11.load().await {
            return None;
        }
        match GROUP_11.call((9, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn substrate_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(substrate_body(args)) })
}
fn swirl_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::swirl::start(args)
}
fn t3d_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::t3d::start(args)
}
fn tessellimage_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::tessellimage::start(args)
}
fn thornbird_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::thornbird::start(args)
}
fn triangle_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::triangle::start(args)
}
fn twang_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::twang::start(args)
}
fn vidwhacker_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::vidwhacker::start(args)
}
fn vines_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::vines::start(args)
}
fn truchet_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::truchet::start(args)
}
fn vermiculate_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::vermiculate::start(args)
}
#[cfg(feature = "split")]
fn group_12_body(args: (u16, StartArgs)) -> Option<Started> {
    let (i, a) = args;
    match i {
        0 => Some(Started::Fb(swirl_body(a))),
        1 => Some(Started::Fb(t3d_body(a))),
        2 => Some(Started::Fb(tessellimage_body(a))),
        3 => Some(Started::Fb(thornbird_body(a))),
        4 => Some(Started::Fb(triangle_body(a))),
        5 => Some(Started::Fb(twang_body(a))),
        6 => Some(Started::Fb(vidwhacker_body(a))),
        7 => Some(Started::Fb(vines_body(a))),
        8 => Some(Started::Fb(truchet_body(a))),
        9 => Some(Started::Fb(vermiculate_body(a))),
        _ => None,
    }
}

#[cfg(feature = "split")]
static GROUP_12: wasm_split::LazyLoader<(u16, StartArgs), Option<Started>> = wasm_split::lazy_loader!(extern "group_12" fn group_12_body(props: (u16, StartArgs)) -> Option<Started>);
#[cfg(feature = "split")]
fn swirl_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_12.load().await {
            return None;
        }
        match GROUP_12.call((0, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn swirl_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(swirl_body(args)) })
}
#[cfg(feature = "split")]
fn t3d_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_12.load().await {
            return None;
        }
        match GROUP_12.call((1, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn t3d_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(t3d_body(args)) })
}
#[cfg(feature = "split")]
fn tessellimage_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_12.load().await {
            return None;
        }
        match GROUP_12.call((2, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn tessellimage_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(tessellimage_body(args)) })
}
#[cfg(feature = "split")]
fn thornbird_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_12.load().await {
            return None;
        }
        match GROUP_12.call((3, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn thornbird_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(thornbird_body(args)) })
}
#[cfg(feature = "split")]
fn triangle_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_12.load().await {
            return None;
        }
        match GROUP_12.call((4, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn triangle_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(triangle_body(args)) })
}
#[cfg(feature = "split")]
fn twang_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_12.load().await {
            return None;
        }
        match GROUP_12.call((5, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn twang_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(twang_body(args)) })
}
#[cfg(feature = "split")]
fn vidwhacker_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_12.load().await {
            return None;
        }
        match GROUP_12.call((6, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn vidwhacker_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(vidwhacker_body(args)) })
}
#[cfg(feature = "split")]
fn vines_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_12.load().await {
            return None;
        }
        match GROUP_12.call((7, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn vines_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(vines_body(args)) })
}
#[cfg(feature = "split")]
fn truchet_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_12.load().await {
            return None;
        }
        match GROUP_12.call((8, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn truchet_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(truchet_body(args)) })
}
#[cfg(feature = "split")]
fn vermiculate_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_12.load().await {
            return None;
        }
        match GROUP_12.call((9, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn vermiculate_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(vermiculate_body(args)) })
}
fn vfeedback_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::vfeedback::start(args)
}
fn wander_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::wander::start(args)
}
fn webcollage_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::webcollage::start(args)
}
fn whirlwindwarp_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::whirlwindwarp::start(args)
}
fn whirlygig_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::whirlygig::start(args)
}
fn worm_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::worm::start(args)
}
fn wormhole_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::wormhole::start(args)
}
fn zoom_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::zoom::start(args)
}
fn xanalogtv_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::xanalogtv::start(args)
}
fn xflame_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::xflame::start(args)
}
#[cfg(feature = "split")]
fn group_13_body(args: (u16, StartArgs)) -> Option<Started> {
    let (i, a) = args;
    match i {
        0 => Some(Started::Fb(vfeedback_body(a))),
        1 => Some(Started::Fb(wander_body(a))),
        2 => Some(Started::Fb(webcollage_body(a))),
        3 => Some(Started::Fb(whirlwindwarp_body(a))),
        4 => Some(Started::Fb(whirlygig_body(a))),
        5 => Some(Started::Fb(worm_body(a))),
        6 => Some(Started::Fb(wormhole_body(a))),
        7 => Some(Started::Fb(zoom_body(a))),
        8 => Some(Started::Fb(xanalogtv_body(a))),
        9 => Some(Started::Fb(xflame_body(a))),
        _ => None,
    }
}

#[cfg(feature = "split")]
static GROUP_13: wasm_split::LazyLoader<(u16, StartArgs), Option<Started>> = wasm_split::lazy_loader!(extern "group_13" fn group_13_body(props: (u16, StartArgs)) -> Option<Started>);
#[cfg(feature = "split")]
fn vfeedback_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_13.load().await {
            return None;
        }
        match GROUP_13.call((0, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn vfeedback_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(vfeedback_body(args)) })
}
#[cfg(feature = "split")]
fn wander_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_13.load().await {
            return None;
        }
        match GROUP_13.call((1, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn wander_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(wander_body(args)) })
}
#[cfg(feature = "split")]
fn webcollage_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_13.load().await {
            return None;
        }
        match GROUP_13.call((2, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn webcollage_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(webcollage_body(args)) })
}
#[cfg(feature = "split")]
fn whirlwindwarp_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_13.load().await {
            return None;
        }
        match GROUP_13.call((3, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn whirlwindwarp_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(whirlwindwarp_body(args)) })
}
#[cfg(feature = "split")]
fn whirlygig_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_13.load().await {
            return None;
        }
        match GROUP_13.call((4, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn whirlygig_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(whirlygig_body(args)) })
}
#[cfg(feature = "split")]
fn worm_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_13.load().await {
            return None;
        }
        match GROUP_13.call((5, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn worm_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(worm_body(args)) })
}
#[cfg(feature = "split")]
fn wormhole_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_13.load().await {
            return None;
        }
        match GROUP_13.call((6, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn wormhole_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(wormhole_body(args)) })
}
#[cfg(feature = "split")]
fn zoom_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_13.load().await {
            return None;
        }
        match GROUP_13.call((7, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn zoom_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(zoom_body(args)) })
}
#[cfg(feature = "split")]
fn xanalogtv_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_13.load().await {
            return None;
        }
        match GROUP_13.call((8, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn xanalogtv_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(xanalogtv_body(args)) })
}
#[cfg(feature = "split")]
fn xflame_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_13.load().await {
            return None;
        }
        match GROUP_13.call((9, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn xflame_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(xflame_body(args)) })
}
fn xjack_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::xjack::start(args)
}
fn xlyap_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::xlyap::start(args)
}
fn xmatrix_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::xmatrix::start(args)
}
fn xrayswarm_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::xrayswarm::start(args)
}
fn xspirograph_body(args: StartArgs) -> Runner {
    xscreensaver::hacks2d::xspirograph::start(args)
}
fn dnalogo_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::dnalogo::start(args)
}
fn extrusion_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::extrusion::start(args)
}
fn juggler3d_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::juggler3d::start(args)
}
fn flurry_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::flurry::start(args)
}
fn atlantis_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::atlantis::start(args)
}
#[cfg(feature = "split")]
fn group_14_body(args: (u16, StartArgs)) -> Option<Started> {
    let (i, a) = args;
    match i {
        0 => Some(Started::Fb(xjack_body(a))),
        1 => Some(Started::Fb(xlyap_body(a))),
        2 => Some(Started::Fb(xmatrix_body(a))),
        3 => Some(Started::Fb(xrayswarm_body(a))),
        4 => Some(Started::Fb(xspirograph_body(a))),
        5 => Some(Started::Gl3d(dnalogo_body(a))),
        6 => Some(Started::Gl3d(extrusion_body(a))),
        7 => Some(Started::Gl3d(juggler3d_body(a))),
        8 => Some(Started::Gl3d(flurry_body(a))),
        9 => Some(Started::Gl3d(atlantis_body(a))),
        _ => None,
    }
}

#[cfg(feature = "split")]
static GROUP_14: wasm_split::LazyLoader<(u16, StartArgs), Option<Started>> = wasm_split::lazy_loader!(extern "group_14" fn group_14_body(props: (u16, StartArgs)) -> Option<Started>);
#[cfg(feature = "split")]
fn xjack_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_14.load().await {
            return None;
        }
        match GROUP_14.call((0, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn xjack_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(xjack_body(args)) })
}
#[cfg(feature = "split")]
fn xlyap_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_14.load().await {
            return None;
        }
        match GROUP_14.call((1, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn xlyap_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(xlyap_body(args)) })
}
#[cfg(feature = "split")]
fn xmatrix_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_14.load().await {
            return None;
        }
        match GROUP_14.call((2, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn xmatrix_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(xmatrix_body(args)) })
}
#[cfg(feature = "split")]
fn xrayswarm_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_14.load().await {
            return None;
        }
        match GROUP_14.call((3, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn xrayswarm_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(xrayswarm_body(args)) })
}
#[cfg(feature = "split")]
fn xspirograph_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async move {
        if !GROUP_14.load().await {
            return None;
        }
        match GROUP_14.call((4, args)).ok().flatten() {
            Some(Started::Fb(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn xspirograph_start(args: StartArgs) -> RunnerFuture {
    Box::pin(async { Some(xspirograph_body(args)) })
}
#[cfg(feature = "split")]
fn dnalogo_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async move {
        if !GROUP_14.load().await {
            return None;
        }
        match GROUP_14.call((5, args)).ok().flatten() {
            Some(Started::Gl3d(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn dnalogo_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(dnalogo_body(args)) })
}
#[cfg(feature = "split")]
fn extrusion_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async move {
        if !GROUP_14.load().await {
            return None;
        }
        match GROUP_14.call((6, args)).ok().flatten() {
            Some(Started::Gl3d(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn extrusion_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(extrusion_body(args)) })
}
#[cfg(feature = "split")]
fn juggler3d_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async move {
        if !GROUP_14.load().await {
            return None;
        }
        match GROUP_14.call((7, args)).ok().flatten() {
            Some(Started::Gl3d(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn juggler3d_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(juggler3d_body(args)) })
}
#[cfg(feature = "split")]
fn flurry_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async move {
        if !GROUP_14.load().await {
            return None;
        }
        match GROUP_14.call((8, args)).ok().flatten() {
            Some(Started::Gl3d(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn flurry_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(flurry_body(args)) })
}
#[cfg(feature = "split")]
fn atlantis_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async move {
        if !GROUP_14.load().await {
            return None;
        }
        match GROUP_14.call((9, args)).ok().flatten() {
            Some(Started::Gl3d(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn atlantis_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(atlantis_body(args)) })
}
fn companioncube_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::companioncube::start(args)
}
fn crackberg_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::crackberg::start(args)
}
fn cubicgrid_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::cubicgrid::start(args)
}
fn handsy_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::handsy::start(args)
}
fn headroom_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::headroom::start(args)
}
fn highvoltage_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::highvoltage::start(args)
}
fn mapscroller_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::mapscroller::start(args)
}
fn unicrud_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::unicrud::start(args)
}
fn winduprobot_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::winduprobot::start(args)
}
fn sproingies_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::sproingies::start(args)
}
#[cfg(feature = "split")]
fn group_15_body(args: (u16, StartArgs)) -> Option<Started> {
    let (i, a) = args;
    match i {
        0 => Some(Started::Gl3d(companioncube_body(a))),
        1 => Some(Started::Gl3d(crackberg_body(a))),
        2 => Some(Started::Gl3d(cubicgrid_body(a))),
        3 => Some(Started::Gl3d(handsy_body(a))),
        4 => Some(Started::Gl3d(headroom_body(a))),
        5 => Some(Started::Gl3d(highvoltage_body(a))),
        6 => Some(Started::Gl3d(mapscroller_body(a))),
        7 => Some(Started::Gl3d(unicrud_body(a))),
        8 => Some(Started::Gl3d(winduprobot_body(a))),
        9 => Some(Started::Gl3d(sproingies_body(a))),
        _ => None,
    }
}

#[cfg(feature = "split")]
static GROUP_15: wasm_split::LazyLoader<(u16, StartArgs), Option<Started>> = wasm_split::lazy_loader!(extern "group_15" fn group_15_body(props: (u16, StartArgs)) -> Option<Started>);
#[cfg(feature = "split")]
fn companioncube_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async move {
        if !GROUP_15.load().await {
            return None;
        }
        match GROUP_15.call((0, args)).ok().flatten() {
            Some(Started::Gl3d(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn companioncube_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(companioncube_body(args)) })
}
#[cfg(feature = "split")]
fn crackberg_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async move {
        if !GROUP_15.load().await {
            return None;
        }
        match GROUP_15.call((1, args)).ok().flatten() {
            Some(Started::Gl3d(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn crackberg_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(crackberg_body(args)) })
}
#[cfg(feature = "split")]
fn cubicgrid_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async move {
        if !GROUP_15.load().await {
            return None;
        }
        match GROUP_15.call((2, args)).ok().flatten() {
            Some(Started::Gl3d(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn cubicgrid_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(cubicgrid_body(args)) })
}
#[cfg(feature = "split")]
fn handsy_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async move {
        if !GROUP_15.load().await {
            return None;
        }
        match GROUP_15.call((3, args)).ok().flatten() {
            Some(Started::Gl3d(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn handsy_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(handsy_body(args)) })
}
#[cfg(feature = "split")]
fn headroom_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async move {
        if !GROUP_15.load().await {
            return None;
        }
        match GROUP_15.call((4, args)).ok().flatten() {
            Some(Started::Gl3d(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn headroom_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(headroom_body(args)) })
}
#[cfg(feature = "split")]
fn highvoltage_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async move {
        if !GROUP_15.load().await {
            return None;
        }
        match GROUP_15.call((5, args)).ok().flatten() {
            Some(Started::Gl3d(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn highvoltage_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(highvoltage_body(args)) })
}
#[cfg(feature = "split")]
fn mapscroller_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async move {
        if !GROUP_15.load().await {
            return None;
        }
        match GROUP_15.call((6, args)).ok().flatten() {
            Some(Started::Gl3d(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn mapscroller_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(mapscroller_body(args)) })
}
#[cfg(feature = "split")]
fn unicrud_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async move {
        if !GROUP_15.load().await {
            return None;
        }
        match GROUP_15.call((7, args)).ok().flatten() {
            Some(Started::Gl3d(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn unicrud_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(unicrud_body(args)) })
}
#[cfg(feature = "split")]
fn winduprobot_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async move {
        if !GROUP_15.load().await {
            return None;
        }
        match GROUP_15.call((8, args)).ok().flatten() {
            Some(Started::Gl3d(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn winduprobot_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(winduprobot_body(args)) })
}
#[cfg(feature = "split")]
fn sproingies_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async move {
        if !GROUP_15.load().await {
            return None;
        }
        match GROUP_15.call((9, args)).ok().flatten() {
            Some(Started::Gl3d(r)) => Some(r),
            _ => None,
        }
    })
}

#[cfg(not(feature = "split"))]
fn sproingies_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(sproingies_body(args)) })
}
fn carousel_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::carousel::start(args)
}
fn chompytower_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::chompytower::start(args)
}
fn skytentacles_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::skytentacles::start(args)
}
fn gltext_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::gltext::start(args)
}
fn glmatrix_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::glmatrix::start(args)
}
fn starwars_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::starwars::start(args)
}
fn fliptext_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::fliptext::start(args)
}
fn flipflop_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::flipflop::start(args)
}
fn flipscreen3d_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::flipscreen3d::start(args)
}
fn peepers_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::peepers::start(args)
}
// These stay resident in the main module: see the note above on the split limit.
fn carousel_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(carousel_body(args)) })
}
fn chompytower_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(chompytower_body(args)) })
}
fn skytentacles_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(skytentacles_body(args)) })
}
fn gltext_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(gltext_body(args)) })
}
fn glmatrix_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(glmatrix_body(args)) })
}
fn starwars_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(starwars_body(args)) })
}
fn fliptext_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(fliptext_body(args)) })
}
fn flipflop_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(flipflop_body(args)) })
}
fn flipscreen3d_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(flipscreen3d_body(args)) })
}
fn peepers_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(peepers_body(args)) })
}
fn photopile_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::photopile::start(args)
}
fn gflux_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::gflux::start(args)
}
fn hexstrut_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::hexstrut::start(args)
}
fn sballs_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::sballs::start(args)
}
fn sierpinski3d_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::sierpinski3d::start(args)
}
fn noof_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::noof::start(args)
}
fn moebius_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::moebius::start(args)
}
fn moebiusgears_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::moebiusgears::start(args)
}
fn mirrorblob_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::mirrorblob::start(args)
}
fn maze3d_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::maze3d::start(args)
}
// These stay resident in the main module: see the note above on the split limit.
fn photopile_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(photopile_body(args)) })
}
fn gflux_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(gflux_body(args)) })
}
fn hexstrut_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(hexstrut_body(args)) })
}
fn sballs_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(sballs_body(args)) })
}
fn sierpinski3d_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(sierpinski3d_body(args)) })
}
fn noof_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(noof_body(args)) })
}
fn moebius_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(moebius_body(args)) })
}
fn moebiusgears_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(moebiusgears_body(args)) })
}
fn mirrorblob_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(mirrorblob_body(args)) })
}
fn maze3d_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(maze3d_body(args)) })
}
fn nakagin_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::nakagin::start(args)
}
fn menger_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::menger::start(args)
}
fn hypnowheel_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::hypnowheel::start(args)
}
fn cubestack_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::cubestack::start(args)
}
fn cubestorm_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::cubestorm::start(args)
}
fn vigilance_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::vigilance::start(args)
}
fn voronoi_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::voronoi::start(args)
}
fn antinspect_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::antinspect::start(args)
}
fn antmaze_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::antmaze::start(args)
}
fn antspotlight_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::antspotlight::start(args)
}
// These stay resident in the main module: see the note above on the split limit.
fn nakagin_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(nakagin_body(args)) })
}
fn menger_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(menger_body(args)) })
}
fn hypnowheel_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(hypnowheel_body(args)) })
}
fn cubestack_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(cubestack_body(args)) })
}
fn cubestorm_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(cubestorm_body(args)) })
}
fn vigilance_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(vigilance_body(args)) })
}
fn voronoi_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(voronoi_body(args)) })
}
fn antinspect_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(antinspect_body(args)) })
}
fn antmaze_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(antmaze_body(args)) })
}
fn antspotlight_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(antspotlight_body(args)) })
}
fn atunnel_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::atunnel::start(args)
}
fn beats_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::beats::start(args)
}
fn covid19_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::covid19::start(args)
}
fn co_9_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::covid19::renamed::start(args)
}
fn crumbler_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::crumbler::start(args)
}
fn cube21_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::cube21::start(args)
}
fn cubetwist_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::cubetwist::start(args)
}
fn cubocteversion_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::cubocteversion::start(args)
}
fn cubenetic_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::cubenetic::start(args)
}
fn raverhoop_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::raverhoop::start(args)
}
// These stay resident in the main module: see the note above on the split limit.
fn atunnel_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(atunnel_body(args)) })
}
fn beats_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(beats_body(args)) })
}
fn covid19_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(covid19_body(args)) })
}
fn co_9_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(co_9_body(args)) })
}
fn crumbler_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(crumbler_body(args)) })
}
fn cube21_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(cube21_body(args)) })
}
fn cubetwist_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(cubetwist_body(args)) })
}
fn cubocteversion_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(cubocteversion_body(args)) })
}
fn cubenetic_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(cubenetic_body(args)) })
}
fn raverhoop_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(raverhoop_body(args)) })
}
fn romanboy_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::romanboy::start(args)
}
fn razzledazzle_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::razzledazzle::start(args)
}
fn rubik_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::rubik::start(args)
}
fn rubikblocks_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::rubikblocks::start(args)
}
fn discoball_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::discoball::start(args)
}
fn dumpsterfire_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::dumpsterfire::start(args)
}
fn dymaxionmap_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::dymaxionmap::start(args)
}
fn endgame_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::endgame::start(args)
}
fn energystream_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::energystream::start(args)
}
fn pinion_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::pinion::start(args)
}
// These stay resident in the main module: see the note above on the split limit.
fn romanboy_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(romanboy_body(args)) })
}
fn razzledazzle_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(razzledazzle_body(args)) })
}
fn rubik_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(rubik_body(args)) })
}
fn rubikblocks_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(rubikblocks_body(args)) })
}
fn discoball_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(discoball_body(args)) })
}
fn dumpsterfire_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(dumpsterfire_body(args)) })
}
fn dymaxionmap_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(dymaxionmap_body(args)) })
}
fn endgame_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(endgame_body(args)) })
}
fn energystream_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(energystream_body(args)) })
}
fn pinion_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(pinion_body(args)) })
}
fn pipes_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::pipes::start(args)
}
fn platonicfolding_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::platonicfolding::start(args)
}
fn polyhedra_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::polyhedra::start(args)
}
fn providence_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::providence::start(args)
}
fn pulsar_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::pulsar::start(args)
}
fn quasicrystal_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::quasicrystal::start(args)
}
fn kallisti_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::kallisti::start(args)
}
fn klein_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::klein::start(args)
}
fn klondike_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::klondike::start(args)
}
fn lament_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::lament::start(args)
}
// These stay resident in the main module: see the note above on the split limit.
fn pipes_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(pipes_body(args)) })
}
fn platonicfolding_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(platonicfolding_body(args)) })
}
fn polyhedra_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(polyhedra_body(args)) })
}
fn providence_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(providence_body(args)) })
}
fn pulsar_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(pulsar_body(args)) })
}
fn quasicrystal_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(quasicrystal_body(args)) })
}
fn kallisti_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(kallisti_body(args)) })
}
fn klein_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(klein_body(args)) })
}
fn klondike_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(klondike_body(args)) })
}
fn lament_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(lament_body(args)) })
}
fn lavalite_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::lavalite::start(args)
}
fn lockward_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::lockward::start(args)
}
fn glsnake_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::glsnake::start(args)
}
fn gravitywell_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::gravitywell::start(args)
}
fn hextrail_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::hextrail::start(args)
}
fn bouncingcow_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::bouncingcow::start(args)
}
fn boxed_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::boxed::start(args)
}
fn bubble3d_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::bubble3d::start(args)
}
fn cage_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::cage::start(args)
}
fn circuit_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::circuit::start(args)
}
// These stay resident in the main module: see the note above on the split limit.
fn lavalite_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(lavalite_body(args)) })
}
fn lockward_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(lockward_body(args)) })
}
fn glsnake_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(glsnake_body(args)) })
}
fn gravitywell_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(gravitywell_body(args)) })
}
fn hextrail_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(hextrail_body(args)) })
}
fn bouncingcow_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(bouncingcow_body(args)) })
}
fn boxed_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(boxed_body(args)) })
}
fn bubble3d_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(bubble3d_body(args)) })
}
fn cage_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(cage_body(args)) })
}
fn circuit_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(circuit_body(args)) })
}
fn cityflow_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::cityflow::start(args)
}
fn blocktube_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::blocktube::start(args)
}
fn boing_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::boing::start(args)
}
fn blinkbox_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::blinkbox::start(args)
}
fn surfaces_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::surfaces::start(args)
}
fn tronbit_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::tronbit::start(args)
}
fn morph3d_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::morph3d::start(args)
}
fn hopffibration_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::hopffibration::start(args)
}
fn hydrostat_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::hydrostat::start(args)
}
fn topblock_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::topblock::start(args)
}
// These stay resident in the main module: see the note above on the split limit.
fn cityflow_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(cityflow_body(args)) })
}
fn blocktube_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(blocktube_body(args)) })
}
fn boing_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(boing_body(args)) })
}
fn blinkbox_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(blinkbox_body(args)) })
}
fn surfaces_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(surfaces_body(args)) })
}
fn tronbit_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(tronbit_body(args)) })
}
fn morph3d_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(morph3d_body(args)) })
}
fn hopffibration_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(hopffibration_body(args)) })
}
fn hydrostat_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(hydrostat_body(args)) })
}
fn topblock_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(topblock_body(args)) })
}
fn skulloop_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::skulloop::start(args)
}
fn sphereeversion_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::sphereeversion::start(args)
}
fn spheremonics_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::spheremonics::start(args)
}
fn hypertorus_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::hypertorus::start(args)
}
fn tangram_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::tangram::start(args)
}
fn timetunnel_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::timetunnel::start(args)
}
fn papercube_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::papercube::start(args)
}
fn engine_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::engine::start(args)
}
fn esper_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::esper::start(args)
}
fn etruscanvenus_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::etruscanvenus::start(args)
}
// These stay resident in the main module: see the note above on the split limit.
fn skulloop_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(skulloop_body(args)) })
}
fn sphereeversion_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(sphereeversion_body(args)) })
}
fn spheremonics_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(spheremonics_body(args)) })
}
fn hypertorus_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(hypertorus_body(args)) })
}
fn tangram_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(tangram_body(args)) })
}
fn timetunnel_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(timetunnel_body(args)) })
}
fn papercube_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(papercube_body(args)) })
}
fn engine_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(engine_body(args)) })
}
fn esper_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(esper_body(args)) })
}
fn etruscanvenus_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(etruscanvenus_body(args)) })
}
fn molecule_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::molecule::start(args)
}
fn projectiveplane_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::projectiveplane::start(args)
}
fn polytopes_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::polytopes::start(args)
}
fn queens_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::queens::start(args)
}
fn geodesic_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::geodesic::start(args)
}
fn geodesicgears_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::geodesicgears::start(args)
}
fn glforestfire_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::glforestfire::start(args)
}
fn gleidescope_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::gleidescope::start(args)
}
fn glslideshow_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::glslideshow::start(args)
}
fn hilbert_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::hilbert::start(args)
}
// These stay resident in the main module: see the note above on the split limit.
fn molecule_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(molecule_body(args)) })
}
fn projectiveplane_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(projectiveplane_body(args)) })
}
fn polytopes_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(polytopes_body(args)) })
}
fn queens_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(queens_body(args)) })
}
fn geodesic_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(geodesic_body(args)) })
}
fn geodesicgears_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(geodesicgears_body(args)) })
}
fn glforestfire_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(glforestfire_body(args)) })
}
fn gleidescope_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(gleidescope_body(args)) })
}
fn glslideshow_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(glslideshow_body(args)) })
}
fn hilbert_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(hilbert_body(args)) })
}
fn jigsaw_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::jigsaw::start(args)
}
fn superquadrics_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::superquadrics::start(args)
}
fn unknownpleasures_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::unknownpleasures::start(args)
}
fn sonar_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::sonar::start(args)
}
fn squirtorus_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::squirtorus::start(args)
}
fn stairs_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::stairs::start(args)
}
fn stonerview_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::stonerview::start(args)
}
fn splitflap_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::splitflap::start(args)
}
fn splodesic_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::splodesic::start(args)
}
fn jigglypuff_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::jigglypuff::start(args)
}
// These stay resident in the main module: see the note above on the split limit.
fn jigsaw_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(jigsaw_body(args)) })
}
fn superquadrics_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(superquadrics_body(args)) })
}
fn unknownpleasures_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(unknownpleasures_body(args)) })
}
fn sonar_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(sonar_body(args)) })
}
fn squirtorus_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(squirtorus_body(args)) })
}
fn stairs_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(stairs_body(args)) })
}
fn stonerview_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(stonerview_body(args)) })
}
fn splitflap_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(splitflap_body(args)) })
}
fn splodesic_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(splodesic_body(args)) })
}
fn jigglypuff_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(jigglypuff_body(args)) })
}
fn kaleidocycle_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::kaleidocycle::start(args)
}
fn glplanet_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::glplanet::start(args)
}
fn glschool_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::glschool::start(args)
}
fn flyingtoasters_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::flyingtoasters::start(args)
}
fn gears_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::gears::start(args)
}
fn gibson_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::gibson::start(args)
}
fn glcells_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::glcells::start(args)
}
fn glblur_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::glblur::start(args)
}
fn glhanoi_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::glhanoi::start(args)
}
fn glknots_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::glknots::start(args)
}
// These stay resident in the main module: see the note above on the split limit.
fn kaleidocycle_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(kaleidocycle_body(args)) })
}
fn glplanet_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(glplanet_body(args)) })
}
fn glschool_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(glschool_body(args)) })
}
fn flyingtoasters_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(flyingtoasters_body(args)) })
}
fn gears_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(gears_body(args)) })
}
fn gibson_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(gibson_body(args)) })
}
fn glcells_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(glcells_body(args)) })
}
fn glblur_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(glblur_body(args)) })
}
fn glhanoi_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(glhanoi_body(args)) })
}
fn glknots_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(glknots_body(args)) })
}
fn dangerball_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::dangerball::start(args)
}
fn deepstars_body(args: StartArgs) -> Runner3d {
    xscreensaver::hacks3d::deepstars::start(args)
}
fn alienbeacon_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::alienbeacon::start(args)
}
fn batteredplanet_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::batteredplanet::start(args)
}
fn bestill_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::bestill::start(args)
}
fn bubblecolors_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::bubblecolors::start(args)
}
fn darktransit_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::darktransit::start(args)
}
fn downfall_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::downfall::start(args)
}
fn driftclouds_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::driftclouds::start(args)
}
fn elementalring_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::elementalring::start(args)
}
// These stay resident in the main module: see the note above on the split limit.
fn dangerball_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(dangerball_body(args)) })
}
fn deepstars_start(args: StartArgs) -> Runner3dFuture {
    Box::pin(async { Some(deepstars_body(args)) })
}
fn alienbeacon_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(alienbeacon_body(args)) })
}
fn batteredplanet_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(batteredplanet_body(args)) })
}
fn bestill_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(bestill_body(args)) })
}
fn bubblecolors_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(bubblecolors_body(args)) })
}
fn darktransit_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(darktransit_body(args)) })
}
fn downfall_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(downfall_body(args)) })
}
fn driftclouds_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(driftclouds_body(args)) })
}
fn elementalring_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(elementalring_body(args)) })
}
fn fluxcore_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::fluxcore::start(args)
}
fn gimbalharmonics_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::gimbalharmonics::start(args)
}
fn goldenapollian_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::goldenapollian::start(args)
}
fn hexplasma_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::hexplasma::start(args)
}
fn logarithmiccircles_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::logarithmiccircles::start(args)
}
fn neongravity_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::neongravity::start(args)
}
fn neontriangulator_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::neontriangulator::start(args)
}
fn noxfire_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::noxfire::start(args)
}
fn prococean_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::prococean::start(args)
}
fn protophore_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::protophore::start(args)
}
// These stay resident in the main module: see the note above on the split limit.
fn fluxcore_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(fluxcore_body(args)) })
}
fn gimbalharmonics_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(gimbalharmonics_body(args)) })
}
fn goldenapollian_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(goldenapollian_body(args)) })
}
fn hexplasma_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(hexplasma_body(args)) })
}
fn logarithmiccircles_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(logarithmiccircles_body(args)) })
}
fn neongravity_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(neongravity_body(args)) })
}
fn neontriangulator_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(neontriangulator_body(args)) })
}
fn noxfire_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(noxfire_body(args)) })
}
fn prococean_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(prococean_body(args)) })
}
fn protophore_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(protophore_body(args)) })
}
fn rigrekt_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::rigrekt::start(args)
}
fn selfreflect_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::selfreflect::start(args)
}
fn skyline_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::skyline::start(args)
}
fn stardome_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::stardome::start(args)
}
fn starnest_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::starnest::start(args)
}
fn stripeytorus_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::stripeytorus::start(args)
}
fn synthwavecity_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::synthwavecity::start(args)
}
fn topologica_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::topologica::start(args)
}
fn trainmandala_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::trainmandala::start(args)
}
fn trizm_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::trizm::start(args)
}
// These stay resident in the main module: see the note above on the split limit.
fn rigrekt_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(rigrekt_body(args)) })
}
fn selfreflect_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(selfreflect_body(args)) })
}
fn skyline_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(skyline_body(args)) })
}
fn stardome_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(stardome_body(args)) })
}
fn starnest_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(starnest_body(args)) })
}
fn stripeytorus_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(stripeytorus_body(args)) })
}
fn synthwavecity_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(synthwavecity_body(args)) })
}
fn topologica_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(topologica_body(args)) })
}
fn trainmandala_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(trainmandala_body(args)) })
}
fn trizm_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(trizm_body(args)) })
}
fn truchetzoom_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::truchetzoom::start(args)
}
fn universeball_body(args: StartArgs) -> Shadertoy {
    xscreensaver::shadertoy::universeball::start(args)
}
// These stay resident in the main module: see the note above on the split limit.
fn truchetzoom_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(truchetzoom_body(args)) })
}
fn universeball_start(args: StartArgs) -> ShadertoyFuture {
    Box::pin(async { Some(universeball_body(args)) })
}

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
        slug: "companioncube",
        label: "Companion Cube",
        start: Start::Gl3d(companioncube_start),
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
        slug: "extrusion",
        label: "Extrusion",
        start: Start::Gl3d(extrusion_start),
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
        slug: "klondike",
        label: "Klondike",
        start: Start::Gl3d(klondike_start),
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
        slug: "mapscroller",
        label: "Map Scroller",
        start: Start::Gl3d(mapscroller_start),
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
        slug: "unicrud",
        label: "Unicrud",
        start: Start::Gl3d(unicrud_start),
    },
    Entry {
        slug: "vidwhacker",
        label: "Vid Whacker",
        start: Start::Fb(vidwhacker_start),
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
        slug: "webcollage",
        label: "Web Collage",
        start: Start::Fb(webcollage_start),
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
        slug: "co____9",
        label: "Co____9",
        start: Start::Gl3d(co_9_start),
    },
    Entry {
        slug: "covid19",
        label: "COVID19",
        start: Start::Gl3d(covid19_start),
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
        slug: "cubocteversion",
        label: "Cuboctahedron Eversion",
        start: Start::Gl3d(cubocteversion_start),
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
        slug: "glcells",
        label: "GL Cells",
        start: Start::Gl3d(glcells_start),
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
        slug: "glplanet",
        label: "GL Planet",
        start: Start::Gl3d(glplanet_start),
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
        slug: "mismunch",
        label: "Mismunch",
        start: Start::Fb(mismunch_start),
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
        slug: "pipes",
        label: "Pipes",
        start: Start::Gl3d(pipes_start),
    },
    Entry {
        slug: "projectiveplane",
        label: "Projective Plane",
        start: Start::Gl3d(projectiveplane_start),
    },
    Entry {
        slug: "platonicfolding",
        label: "Platonic Folding",
        start: Start::Gl3d(platonicfolding_start),
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
        slug: "sonar",
        label: "Sonar",
        start: Start::Gl3d(sonar_start),
    },
    Entry {
        slug: "squirtorus",
        label: "Squirtorus",
        start: Start::Gl3d(squirtorus_start),
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
        slug: "sphereeversion",
        label: "Sphere Eversion",
        start: Start::Gl3d(sphereeversion_start),
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
        slug: "hopffibration",
        label: "Hopf Fibration",
        start: Start::Gl3d(hopffibration_start),
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
        slug: "timetunnel",
        label: "Time Tunnel",
        start: Start::Gl3d(timetunnel_start),
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
        slug: "dymaxionmap",
        label: "Dymaxion Map",
        start: Start::Gl3d(dymaxionmap_start),
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
