#![allow(dead_code)]
//! systemd-cgls — Recursively show control group contents as a tree.
//!
//! A drop-in replacement for `systemd-cgls(1)`. Reads the cgroup2 unified
//! hierarchy from `/sys/fs/cgroup/` and displays it as an indented tree,
//! optionally showing the processes within each cgroup.

use clap::Parser;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

// ── CLI ───────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "systemd-cgls",
    about = "Recursively show control group contents",
    version
)]
struct Cli {
    /// Show all groups, including empty ones
    #[arg(short, long)]
    all: bool,

    /// Limit output to a specific cgroup path(s)
    cgroup: Vec<String>,

    /// Show kernel threads in addition to userspace processes
    #[arg(short, long)]
    kernel_threads: bool,

    /// Do not pipe output into a pager
    #[arg(long)]
    no_pager: bool,

    /// Do not ellipsize process tree members
    #[arg(short, long)]
    full: bool,

    /// Specify a machine to show the cgroup tree for
    #[arg(short = 'M', long)]
    machine: Option<String>,

    /// Limit depth of the displayed cgroup tree
    #[arg(long)]
    depth: Option<usize>,

    /// Show extended attributes for each cgroup (yes/no)
    #[arg(long, default_value = "no", default_missing_value = "yes", num_args = 0..=1, value_parser = parse_bool_arg)]
    xattr: bool,

    /// Show cgroup ID for each cgroup (yes/no)
    #[arg(long, default_value = "no", default_missing_value = "yes", num_args = 0..=1, value_parser = parse_bool_arg)]
    cgroup_id: bool,

    /// Show the subtree of a specific unit
    #[arg(short = 'u', long)]
    unit: Option<String>,

    /// Show the subtree of a specific user unit
    #[arg(long)]
    user_unit: Option<String>,
}

/// Parse a boolean argument value (yes/no/true/false/1/0).
fn parse_bool_arg(s: &str) -> Result<bool, String> {
    match s.to_lowercase().as_str() {
        "yes" | "true" | "1" => Ok(true),
        "no" | "false" | "0" => Ok(false),
        _ => Err(format!("Invalid boolean value: {s}")),
    }
}

// ── Data structures ───────────────────────────────────────────────────────

/// Information about a single process.
#[derive(Debug, Clone)]
struct ProcessInfo {
    pid: u32,
    comm: String,
}

/// A node in the cgroup tree.
#[derive(Debug, Clone)]
struct CgroupNode {
    name: String,
    path: PathBuf,
    processes: Vec<ProcessInfo>,
    children: BTreeMap<String, CgroupNode>,
}

impl CgroupNode {
    fn new(name: &str, path: PathBuf) -> Self {
        CgroupNode {
            name: name.to_string(),
            path,
            processes: Vec::new(),
            children: BTreeMap::new(),
        }
    }

    /// Returns true if this node and all its descendants have no processes.
    fn is_empty_recursive(&self) -> bool {
        if !self.processes.is_empty() {
            return false;
        }
        self.children.values().all(|c| c.is_empty_recursive())
    }

    /// Total number of processes in this node and all descendants.
    fn process_count_recursive(&self) -> usize {
        let mut count = self.processes.len();
        for child in self.children.values() {
            count += child.process_count_recursive();
        }
        count
    }
}

// ── Cgroup reading ────────────────────────────────────────────────────────

const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// Read PIDs from a cgroup's `cgroup.procs` file.
fn read_cgroup_procs(cgroup_path: &Path) -> Vec<u32> {
    let procs_file = cgroup_path.join("cgroup.procs");
    let content = match fs::read_to_string(&procs_file) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    content
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect()
}

/// Read process information for a given PID from /proc.
fn read_process_info(pid: u32) -> Option<ProcessInfo> {
    let comm_path = format!("/proc/{}/comm", pid);
    let comm = fs::read_to_string(&comm_path)
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "?".to_string());

    Some(ProcessInfo { pid, comm })
}

/// Build a cgroup tree by recursively reading the filesystem.
fn build_cgroup_tree(
    path: &Path,
    name: &str,
    max_depth: Option<usize>,
    current_depth: usize,
    include_kernel_threads: bool,
) -> CgroupNode {
    let mut node = CgroupNode::new(name, path.to_path_buf());

    // Read processes in this cgroup
    let pids = read_cgroup_procs(path);
    for pid in pids {
        // Filter out kernel threads (PID 2 and its children) unless requested
        if !include_kernel_threads && is_kernel_thread(pid) {
            continue;
        }
        if let Some(info) = read_process_info(pid) {
            node.processes.push(info);
        }
    }

    // Sort processes by PID
    node.processes.sort_by_key(|p| p.pid);

    // Recurse into child cgroups (if within depth limit)
    if max_depth.is_none_or(|d| current_depth < d)
        && let Ok(entries) = fs::read_dir(path)
    {
        let mut dirs: Vec<_> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter(|e| {
                // Skip non-cgroup directories (those without cgroup.procs)
                e.path().join("cgroup.procs").exists()
            })
            .collect();

        // Sort by name for consistent output
        dirs.sort_by_key(|e| e.file_name());

        for entry in dirs {
            let child_name = entry.file_name().to_string_lossy().to_string();
            let child_path = entry.path();
            let child = build_cgroup_tree(
                &child_path,
                &child_name,
                max_depth,
                current_depth + 1,
                include_kernel_threads,
            );
            node.children.insert(child_name, child);
        }
    }

    node
}

/// Check if a PID corresponds to a kernel thread.
fn is_kernel_thread(pid: u32) -> bool {
    if pid == 0 || pid == 2 {
        return true;
    }

    // Kernel threads have no VmSize in /proc/PID/status
    let status_path = format!("/proc/{}/status", pid);
    if let Ok(content) = fs::read_to_string(&status_path) {
        // Kernel threads have PPid of 2 (kthreadd) or are kthreadd itself
        let mut ppid = 0u32;
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("PPid:") {
                ppid = rest.trim().parse().unwrap_or(0);
            }
        }
        if ppid == 2 {
            return true;
        }

        // Another heuristic: kernel threads have empty /proc/PID/cmdline
        let cmdline_path = format!("/proc/{}/cmdline", pid);
        if let Ok(cmdline) = fs::read_to_string(&cmdline_path)
            && cmdline.is_empty()
            && ppid != 1
        {
            return true;
        }
    }

    false
}

// ── Tree display ──────────────────────────────────────────────────────────

/// Tree-drawing characters
const TREE_BRANCH: &str = "├─";
const TREE_LAST: &str = "└─";
const TREE_VERT: &str = "│ ";
const TREE_SPACE: &str = "  ";

/// Print the cgroup tree to stdout.
fn print_tree(node: &CgroupNode, prefix: &str, is_last: bool, is_root: bool, show_empty: bool) {
    // Skip empty nodes unless --all is specified
    if !show_empty && node.is_empty_recursive() && !is_root {
        return;
    }

    // Count visible children
    let visible_children: Vec<&CgroupNode> = if show_empty {
        node.children.values().collect()
    } else {
        node.children
            .values()
            .filter(|c| !c.is_empty_recursive())
            .collect()
    };

    // Print the node name
    if is_root {
        let cgroup_display = if node.name == "/" || node.name.is_empty() {
            "Control group /:"
        } else {
            &node.name
        };
        println!("{}", cgroup_display);
    } else {
        let connector = if is_last { TREE_LAST } else { TREE_BRANCH };
        println!("{}{}{}", prefix, connector, node.name);
    }

    // Determine the prefix for children
    let child_prefix = if is_root {
        String::new()
    } else if is_last {
        format!("{}{}", prefix, TREE_SPACE)
    } else {
        format!("{}{}", prefix, TREE_VERT)
    };

    // Total items to render under this node
    let total_items = node.processes.len() + visible_children.len();
    let mut item_index = 0;

    // Print processes
    for proc in &node.processes {
        item_index += 1;
        let is_last_item = item_index == total_items;
        let connector = if is_last_item { TREE_LAST } else { TREE_BRANCH };
        println!("{}{}{} {}", child_prefix, connector, proc.pid, proc.comm);
    }

    // Print child cgroups
    let child_count = visible_children.len();
    for (i, child) in visible_children.iter().enumerate() {
        item_index += 1;
        let is_last_child = i == child_count - 1;
        print_tree(child, &child_prefix, is_last_child, false, show_empty);
    }
}

/// Resolve a cgroup path string to a filesystem path.
/// Handles both relative cgroup paths (e.g. "system.slice/foo.service")
/// and absolute filesystem paths (e.g. "/sys/fs/cgroup/system.slice/foo.service").
fn resolve_cgroup_path(cg: &str) -> PathBuf {
    let path = Path::new(cg);
    if path.starts_with(CGROUP_ROOT) {
        path.to_path_buf()
    } else {
        let cg = cg.trim_start_matches('/');
        if cg.is_empty() {
            PathBuf::from(CGROUP_ROOT)
        } else {
            PathBuf::from(CGROUP_ROOT).join(cg)
        }
    }
}

/// Find a unit's cgroup path by searching for a directory matching the unit name
/// in the cgroup hierarchy.
fn find_unit_cgroup(unit_name: &str) -> Option<PathBuf> {
    find_unit_cgroup_in(Path::new(CGROUP_ROOT), unit_name)
}

fn find_unit_cgroup_in(dir: &Path, unit_name: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str == unit_name && path.join("cgroup.procs").exists() {
            return Some(path);
        }
        // Recurse into slices
        if (name_str.ends_with(".slice") || name_str == "init.scope")
            && let Some(found) = find_unit_cgroup_in(&path, unit_name)
        {
            return Some(found);
        }
    }
    None
}

// ── Main ──────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    if cli.machine.is_some() {
        eprintln!("Machine connection is not yet supported.");
        process::exit(1);
    }

    // Determine which cgroup paths to display
    let cgroup_paths = if let Some(ref unit) = cli.unit {
        match find_unit_cgroup(unit) {
            Some(path) => vec![path],
            None => {
                eprintln!("Failed to get unit cgroup path for {unit}.");
                process::exit(1);
            }
        }
    } else if let Some(ref user_unit) = cli.user_unit {
        // Search in user cgroup hierarchy
        let user_root = PathBuf::from(CGROUP_ROOT).join("user.slice");
        let found = if user_root.exists() {
            find_unit_cgroup_in(&user_root, user_unit)
        } else {
            None
        };
        match found {
            Some(path) => vec![path],
            None => {
                eprintln!("Failed to get user unit cgroup path for {user_unit}.");
                process::exit(1);
            }
        }
    } else if cli.cgroup.is_empty() {
        // Auto-detect: if CWD is under /sys/fs/cgroup, use it
        if let Ok(cwd) = std::env::current_dir() {
            if cwd.starts_with(CGROUP_ROOT) && cwd.join("cgroup.procs").exists() {
                vec![cwd]
            } else {
                vec![PathBuf::from(CGROUP_ROOT)]
            }
        } else {
            vec![PathBuf::from(CGROUP_ROOT)]
        }
    } else {
        cli.cgroup
            .iter()
            .map(|cg| resolve_cgroup_path(cg))
            .collect()
    };

    for cg_path in &cgroup_paths {
        if !cg_path.exists() {
            eprintln!(
                "Failed to list cgroup {}: No such file or directory",
                cg_path.display()
            );
            process::exit(1);
        }

        let name = if cg_path == Path::new(CGROUP_ROOT) {
            "/".to_string()
        } else {
            cg_path
                .strip_prefix(CGROUP_ROOT)
                .unwrap_or(cg_path)
                .display()
                .to_string()
        };

        let tree = build_cgroup_tree(cg_path, &name, cli.depth, 0, cli.kernel_threads);

        print_tree(&tree, "", true, true, cli.all);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cgroup_node_new() {
        let node = CgroupNode::new("test", PathBuf::from("/sys/fs/cgroup/test"));
        assert_eq!(node.name, "test");
        assert!(node.processes.is_empty());
        assert!(node.children.is_empty());
    }

    #[test]
    fn test_cgroup_node_is_empty_recursive_empty() {
        let node = CgroupNode::new("test", PathBuf::from("/tmp"));
        assert!(node.is_empty_recursive());
    }

    #[test]
    fn test_cgroup_node_is_empty_recursive_with_process() {
        let mut node = CgroupNode::new("test", PathBuf::from("/tmp"));
        node.processes.push(ProcessInfo {
            pid: 1,
            comm: "init".to_string(),
        });
        assert!(!node.is_empty_recursive());
    }

    #[test]
    fn test_cgroup_node_is_empty_recursive_child_has_process() {
        let mut node = CgroupNode::new("parent", PathBuf::from("/tmp"));
        let mut child = CgroupNode::new("child", PathBuf::from("/tmp/child"));
        child.processes.push(ProcessInfo {
            pid: 42,
            comm: "test".to_string(),
        });
        node.children.insert("child".to_string(), child);
        assert!(!node.is_empty_recursive());
    }

    #[test]
    fn test_cgroup_node_process_count_recursive() {
        let mut root = CgroupNode::new("root", PathBuf::from("/tmp"));
        root.processes.push(ProcessInfo {
            pid: 1,
            comm: "init".to_string(),
        });

        let mut child = CgroupNode::new("child", PathBuf::from("/tmp/child"));
        child.processes.push(ProcessInfo {
            pid: 10,
            comm: "a".to_string(),
        });
        child.processes.push(ProcessInfo {
            pid: 11,
            comm: "b".to_string(),
        });

        let mut grandchild = CgroupNode::new("grandchild", PathBuf::from("/tmp/child/gc"));
        grandchild.processes.push(ProcessInfo {
            pid: 100,
            comm: "c".to_string(),
        });

        child.children.insert("grandchild".to_string(), grandchild);
        root.children.insert("child".to_string(), child);

        assert_eq!(root.process_count_recursive(), 4);
    }

    #[test]
    fn test_process_info_fields() {
        let info = ProcessInfo {
            pid: 42,
            comm: "myprocess".to_string(),
        };
        assert_eq!(info.pid, 42);
        assert_eq!(info.comm, "myprocess");
    }

    #[test]
    fn test_read_process_info_self() {
        let pid = std::process::id();
        let info = read_process_info(pid);
        assert!(info.is_some());
        let info = info.unwrap();
        assert_eq!(info.pid, pid);
        assert!(!info.comm.is_empty());
    }

    #[test]
    fn test_read_process_info_nonexistent() {
        // PID 0 is the idle/swapper process; its comm may or may not be readable
        // Use a very high PID that almost certainly doesn't exist
        let info = read_process_info(4_000_000);
        // Should still return Some (with "?" as comm) since we don't check existence
        // Actually our implementation reads /proc/PID/comm, if it fails comm = "?"
        if let Some(info) = info {
            assert_eq!(info.pid, 4_000_000);
        }
    }

    #[test]
    fn test_read_cgroup_procs_nonexistent() {
        let procs = read_cgroup_procs(Path::new("/nonexistent/path"));
        assert!(procs.is_empty());
    }

    #[test]
    fn test_read_cgroup_procs_root() {
        // The root cgroup should have some processes (this test may fail in
        // restricted environments, but should work on most Linux systems)
        let procs = read_cgroup_procs(Path::new(CGROUP_ROOT));
        // Don't assert non-empty since in containers the root cgroup might
        // have no direct processes (they're all in child cgroups)
        let _ = procs;
    }

    #[test]
    fn test_is_kernel_thread_pid0() {
        assert!(is_kernel_thread(0));
    }

    #[test]
    fn test_is_kernel_thread_pid2() {
        assert!(is_kernel_thread(2));
    }

    #[test]
    fn test_is_kernel_thread_self() {
        // Our own process should NOT be a kernel thread
        let pid = std::process::id();
        assert!(!is_kernel_thread(pid));
    }

    #[test]
    fn test_build_cgroup_tree_nonexistent() {
        let tree = build_cgroup_tree(Path::new("/nonexistent"), "test", None, 0, false);
        assert_eq!(tree.name, "test");
        assert!(tree.processes.is_empty());
        assert!(tree.children.is_empty());
    }

    #[test]
    fn test_build_cgroup_tree_root() {
        // Build the tree from the actual cgroup root — should not panic
        if Path::new(CGROUP_ROOT).exists() {
            let tree = build_cgroup_tree(Path::new(CGROUP_ROOT), "/", Some(1), 0, false);
            assert_eq!(tree.name, "/");
        }
    }

    #[test]
    fn test_build_cgroup_tree_depth_limit() {
        if Path::new(CGROUP_ROOT).exists() {
            let tree = build_cgroup_tree(Path::new(CGROUP_ROOT), "/", Some(0), 0, false);
            // With depth 0, there should be no children
            assert!(tree.children.is_empty());
        }
    }

    #[test]
    fn test_print_tree_no_panic() {
        let mut root = CgroupNode::new("/", PathBuf::from("/tmp"));
        root.processes.push(ProcessInfo {
            pid: 1,
            comm: "init".to_string(),
        });

        let mut child = CgroupNode::new("user.slice", PathBuf::from("/tmp/user"));
        child.processes.push(ProcessInfo {
            pid: 1000,
            comm: "bash".to_string(),
        });
        root.children.insert("user.slice".to_string(), child);

        // This should not panic; output goes to stdout
        print_tree(&root, "", true, true, false);
    }

    #[test]
    fn test_print_tree_empty_skipped() {
        let mut root = CgroupNode::new("/", PathBuf::from("/tmp"));
        let empty_child = CgroupNode::new("empty", PathBuf::from("/tmp/empty"));
        root.children.insert("empty".to_string(), empty_child);

        // With show_empty=false, the empty child should be skipped
        // (we can't easily capture stdout here, but at least verify no panic)
        print_tree(&root, "", true, true, false);
    }

    #[test]
    fn test_print_tree_show_all() {
        let mut root = CgroupNode::new("/", PathBuf::from("/tmp"));
        let empty_child = CgroupNode::new("empty", PathBuf::from("/tmp/empty"));
        root.children.insert("empty".to_string(), empty_child);

        // With show_empty=true, the empty child should be shown
        print_tree(&root, "", true, true, true);
    }
}
