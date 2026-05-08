// Config file search-path resolution. Mirrors `src/pipewire/conf.c`'s
// search order (PIPEWIRE_CONFIG_DIR > $HOME/.config/pipewire > /etc >
// /usr/share/pipewire), and the level numbering used in
// `override.<level>.<index>.config.path` keys.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ConfigPath {
    pub level: i32,
    pub index: i32,
    pub path: PathBuf,
    pub is_override: bool,
}

#[derive(Debug, Clone, Copy)]
enum Source {
    /// `$PIPEWIRE_CONFIG_DIR` env var. Reported as level 1 in overrides.
    Env,
    /// Compile-time data dir, fallback when env is unset. Level 1.
    Confdata,
    /// Compile-time config dir. Level 2.
    Configdir,
    /// User home config. Level 3.
    Homeconf,
}

impl Source {
    fn override_level(self) -> i32 {
        match self {
            Source::Env | Source::Confdata => 1,
            Source::Configdir => 2,
            Source::Homeconf => 3,
        }
    }
}

/// Locate every config file matching `name` (e.g. "pipewire.conf"), in
/// the order the C library would load them.
pub fn discover(name: &str, prefix: Option<&str>) -> Vec<ConfigPath> {
    let mut out = Vec::new();
    let drop_in_name = format!("{name}.d");

    // First the main config — first match wins. Search order from
    // get_config_path() in conf.c:
    //   1. abs path / env, 2. home, 3. configdir, 4. confdata
    let main_chain: Vec<(Source, Option<PathBuf>)> = vec![
        (Source::Env, env_dir()),
        (Source::Homeconf, home_dir()),
        (Source::Configdir, Some(PathBuf::from("/etc/pipewire"))),
        (Source::Confdata, Some(PathBuf::from("/usr/share/pipewire"))),
    ];
    let mut main_source: Option<Source> = None;
    for (src, dir) in &main_chain {
        if let Some(d) = dir {
            let p = build_path(d, prefix, name);
            if p.is_file() {
                out.push(ConfigPath {
                    level: -1,
                    index: -1,
                    path: p,
                    is_override: false,
                });
                main_source = Some(*src);
                break;
            }
        }
    }
    if main_source.is_none() {
        return out;
    }

    // Then walk drop-in directories. The C tool walks them in the same
    // order get_config_dir() iterates, with `level` incrementing as each
    // path source is exhausted.
    let drop_in_chain: Vec<(Source, Option<PathBuf>)> = if env_dir().is_some() {
        // env is set → only env dir provides drop-ins (the level counter
        // gates further iteration).
        vec![(Source::Env, env_dir())]
    } else {
        vec![
            (Source::Confdata, Some(PathBuf::from("/usr/share/pipewire"))),
            (Source::Configdir, Some(PathBuf::from("/etc/pipewire"))),
            (Source::Homeconf, home_dir()),
        ]
    };

    for (src, dir) in drop_in_chain {
        let dir = match dir {
            Some(d) => d,
            None => continue,
        };
        let drop_in_dir = build_path(&dir, prefix, &drop_in_name);
        let entries = match fs::read_dir(&drop_in_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().and_then(|s| s.to_str()) == Some("conf"))
            .collect();
        files.sort();
        for (idx, p) in files.into_iter().enumerate() {
            out.push(ConfigPath {
                level: src.override_level(),
                index: idx as i32,
                path: p,
                is_override: true,
            });
        }
    }

    out
}

fn env_dir() -> Option<PathBuf> {
    env::var("PIPEWIRE_CONFIG_DIR").ok().map(PathBuf::from)
}

fn home_dir() -> Option<PathBuf> {
    if let Ok(d) = env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(d).join("pipewire"));
    }
    if let Ok(home) = env::var("HOME") {
        return Some(PathBuf::from(home).join(".config").join("pipewire"));
    }
    None
}

fn build_path(dir: &Path, prefix: Option<&str>, name: &str) -> PathBuf {
    let mut p = PathBuf::from(dir);
    if let Some(pre) = prefix
        && !pre.is_empty()
    {
        p.push(pre);
    }
    p.push(name);
    p
}
