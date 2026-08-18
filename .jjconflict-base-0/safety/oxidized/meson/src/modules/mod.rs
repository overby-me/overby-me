pub mod cmake;
pub mod cuda;
pub mod dlang;
pub mod external_project;
/// Meson modules: fs, pkgconfig, python, gnome, cmake, rust, windows,
/// i18n, qt, sourceset, keyval, wayland, cuda, hotdoc, java, dlang,
/// external_project, icestorm.
pub mod fs;
pub mod gnome;
pub mod hotdoc;
pub mod i18n;
pub mod icestorm;
pub mod modtest;

pub mod java;
pub mod keyval;
pub mod pkgconfig;
pub mod python;
pub mod qt;
pub mod rust_mod;
pub mod simd;
pub mod sourceset;
pub mod wayland;
pub mod windows;

use crate::vm::VM;

pub fn register_methods(vm: &mut VM) {
    fs::register(vm);
    pkgconfig::register(vm);
    python::register(vm);
    gnome::register(vm);
    cmake::register(vm);
    rust_mod::register(vm);
    windows::register(vm);
    i18n::register(vm);
    qt::register(vm);
    sourceset::register(vm);
    keyval::register(vm);
    wayland::register(vm);
    cuda::register(vm);
    hotdoc::register(vm);
    java::register(vm);
    dlang::register(vm);
    external_project::register(vm);
    icestorm::register(vm);
    modtest::register(vm);
    simd::register(vm);
}
