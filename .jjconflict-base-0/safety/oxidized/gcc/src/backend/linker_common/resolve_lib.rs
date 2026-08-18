//! Library name resolution helper.
//!
//! Resolves `-l` library names to filesystem paths by searching library
//! directories, handling both exact (`:filename`) and prefix (`libfoo.so/.a`)
//! forms.

use std::path::Path;

/// Resolve a library name to a path by searching directories.
///
/// Handles both `-l:filename` (exact match) and `-lfoo` (lib prefix search).
/// When `prefer_static` is true, searches for `.a` before `.so`.
pub fn resolve_lib(name: &str, paths: &[String], prefer_static: bool) -> Option<String> {
    if let Some(exact) = name.strip_prefix(':') {
        for dir in paths {
            let p = format!("{}/{}", dir, exact);
            if Path::new(&p).exists() {
                return Some(p);
            }
        }
        return None;
    }
    if prefer_static {
        for dir in paths {
            let a = format!("{}/lib{}.a", dir, name);
            if Path::new(&a).exists() {
                return Some(a);
            }
            let so = format!("{}/lib{}.so", dir, name);
            if Path::new(&so).exists() {
                return Some(so);
            }
        }
    } else {
        for dir in paths {
            let so = format!("{}/lib{}.so", dir, name);
            if Path::new(&so).exists() {
                return Some(so);
            }
            let a = format!("{}/lib{}.a", dir, name);
            if Path::new(&a).exists() {
                return Some(a);
            }
        }
    }
    None
}
