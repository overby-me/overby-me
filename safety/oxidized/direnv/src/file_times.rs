//! File modification time tracking for direnv watches.

use crate::gzenv;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// FileTime represents a single recorded file status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTime {
    pub path: String,
    pub modtime: i64,
    pub exists: bool,
}

impl FileTime {
    /// Check verifies that the file hasn't changed since it was recorded.
    pub fn check(&self) -> Result<(), String> {
        match get_latest_stat(&self.path) {
            Err(_) => {
                if self.exists {
                    Err(format!("File {:?} is missing", self.path))
                } else {
                    Ok(())
                }
            }
            Ok(stat) => {
                if !self.exists {
                    return Err(format!("File {:?} newly created", self.path));
                }
                let modtime = stat
                    .modified()
                    .unwrap_or(SystemTime::UNIX_EPOCH)
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;

                if modtime != self.modtime {
                    Err(format!("File {:?} has changed", self.path))
                } else {
                    Ok(())
                }
            }
        }
    }

    /// Return a human-friendly formatted string.
    pub fn formatted(&self, rel_dir: &Path) -> String {
        let path = PathBuf::from(&self.path);
        let display = path
            .strip_prefix(rel_dir)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| self.path.clone());

        let time_str = if self.modtime > 0 {
            chrono_format(self.modtime)
        } else {
            "<<???>>".to_string()
        };

        format!("\"{display}\" - {time_str}")
    }
}

fn chrono_format(unix_secs: i64) -> String {
    // Simple ISO-ish format without pulling in chrono
    // Convert to a basic UTC timestamp
    let secs = unix_secs;
    let days = secs / 86400;
    let rem = secs % 86400;
    let hours = rem / 3600;
    let mins = (rem % 3600) / 60;
    let s = rem % 60;

    // Days since epoch to Y-M-D (simplified Gregorian)
    let mut y = 1970i64;
    let mut d = days;
    loop {
        let days_in_year = if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
            366
        } else {
            365
        };
        if d < days_in_year {
            break;
        }
        d -= days_in_year;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let mdays = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut m = 0usize;
    while m < 12 && d >= mdays[m] {
        d -= mdays[m];
        m += 1;
    }
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m + 1,
        d + 1,
        hours,
        mins,
        s
    )
}

/// FileTimes represent a record of all known files and times.
#[derive(Debug, Clone)]
pub struct FileTimes {
    pub list: Vec<FileTime>,
}

impl FileTimes {
    pub fn new() -> Self {
        Self { list: Vec::new() }
    }

    /// Update gets the latest stats on the path and updates the record.
    pub fn update(&mut self, path: &str) -> Result<(), String> {
        let abs_path = fs::canonicalize(path)
            .or_else(|_| {
                // If file doesn't exist, just use the absolute path
                let p = PathBuf::from(path);
                if p.is_absolute() {
                    Ok(p)
                } else {
                    std::env::current_dir()
                        .map(|cwd| cwd.join(p))
                        .map_err(|e| e.to_string())
                }
            })
            .map_err(|e| format!("abs path: {e}"))?;

        let abs_str = abs_path.to_string_lossy().to_string();

        match get_latest_stat(&abs_str) {
            Ok(stat) => {
                let modtime = stat
                    .modified()
                    .unwrap_or(SystemTime::UNIX_EPOCH)
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                self.new_time(&abs_str, modtime, true);
            }
            Err(_) => {
                self.new_time(&abs_str, 0, false);
            }
        }
        Ok(())
    }

    /// Add or update a file time entry.
    pub fn new_time(&mut self, path: &str, modtime: i64, exists: bool) {
        if let Some(entry) = self.list.iter_mut().find(|t| t.path == path) {
            entry.modtime = modtime;
            entry.exists = exists;
        } else {
            self.list.push(FileTime {
                path: path.to_string(),
                modtime,
                exists,
            });
        }
    }

    /// Check validates all recorded file times.
    pub fn check(&self) -> Result<(), String> {
        if self.list.is_empty() {
            return Err("Times list is empty".to_string());
        }
        for ft in &self.list {
            ft.check()?;
        }
        Ok(())
    }

    /// Marshal dumps the times into gzenv format.
    pub fn marshal(&self) -> String {
        gzenv::marshal(&self.list)
    }

    /// Unmarshal loads the watches from gzenv format.
    pub fn unmarshal(from: &str) -> Result<Self, String> {
        let list: Vec<FileTime> = gzenv::unmarshal(from)?;
        Ok(Self { list })
    }
}

fn get_latest_stat(path: &str) -> Result<fs::Metadata, std::io::Error> {
    let lstat = fs::symlink_metadata(path)?;
    let stat = fs::metadata(path)?;

    let lmod = lstat
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let smod = stat
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if lmod > smod { Ok(lstat) } else { Ok(stat) }
}
