use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let prog = Path::new(&args[0])
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    match prog.as_str() {
        "cmp" => cmp_main(&args[1..]),
        "sdiff" => sdiff_main(&args[1..]),
        "diff3" => diff3_main(&args[1..]),
        _ => diff_main(&args[1..]),
    }
}

// ---------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------

fn diff_main(args: &[String]) {
    let mut opts = DiffOpts::default();
    let mut files = Vec::new();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--version" => {
                println!("diff (rust-diffutils) {}", env!("CARGO_PKG_VERSION"));
                process::exit(0);
            }
            "--help" => {
                println!("Usage: diff [OPTION]... FILE1 FILE2");
                println!("Compare FILE1 and FILE2 line by line.");
                println!("  -u, -U NUM, --unified[=NUM]  unified diff (default 3 context lines)");
                println!("  -c, -C NUM, --context[=NUM]  context diff");
                println!("  -e, --ed                     output ed script");
                println!("  -n, --rcs                    output RCS format");
                println!("  -y, --side-by-side           side-by-side output");
                println!("  -q, --brief                  report only whether files differ");
                println!("  -s, --report-identical-files  report when files are identical");
                println!("  -r, --recursive              recursively compare directories");
                println!("  -N, --new-file               treat absent files as empty");
                println!("  -i, --ignore-case            ignore case differences");
                println!("  -w, --ignore-all-space        ignore all white space");
                println!("  -b, --ignore-space-change    ignore changes in amount of white space");
                println!("  -B, --ignore-blank-lines     ignore blank line changes");
                println!("  -a, --text                   treat all files as text");
                println!("  --label LABEL                use LABEL for file name");
                println!("  --color[=WHEN]               colorize output");
                println!("  --no-dereference             don't follow symlinks");
                process::exit(0);
            }
            "-u" | "--unified" => opts.format = DiffFormat::Unified(3),
            "-c" | "--context" => opts.format = DiffFormat::Context(3),
            "-e" | "--ed" => opts.format = DiffFormat::Ed,
            "-n" | "--rcs" => opts.format = DiffFormat::Rcs,
            "-y" | "--side-by-side" => opts.format = DiffFormat::SideBySide,
            "-q" | "--brief" => opts.brief = true,
            "-s" | "--report-identical-files" => opts.report_identical = true,
            "-r" | "--recursive" => opts.recursive = true,
            "-N" | "--new-file" => opts.new_file = true,
            "-i" | "--ignore-case" => opts.ignore_case = true,
            "-w" | "--ignore-all-space" => opts.ignore_all_space = true,
            "-b" | "--ignore-space-change" => opts.ignore_space_change = true,
            "-B" | "--ignore-blank-lines" => opts.ignore_blank_lines = true,
            "-a" | "--text" => opts.text = true,
            "--label" => {
                i += 1;
                if i < args.len() {
                    opts.labels.push(args[i].clone());
                }
            }
            "--normal" => opts.format = DiffFormat::Normal,
            "--color" | "--color=always" | "--color=auto" => {}
            "--color=never" => {}
            "--no-dereference" => {}
            "-U" => {
                i += 1;
                if i < args.len() {
                    let n: usize = args[i].parse().unwrap_or(3);
                    opts.format = DiffFormat::Unified(n);
                }
            }
            "-C" => {
                i += 1;
                if i < args.len() {
                    let n: usize = args[i].parse().unwrap_or(3);
                    opts.format = DiffFormat::Context(n);
                }
            }
            arg if arg.starts_with("-U") => {
                let n: usize = arg[2..].parse().unwrap_or(3);
                opts.format = DiffFormat::Unified(n);
            }
            arg if arg.starts_with("-C") => {
                let n: usize = arg[2..].parse().unwrap_or(3);
                opts.format = DiffFormat::Context(n);
            }
            arg if arg.starts_with("--unified=") => {
                let n: usize = arg.strip_prefix("--unified=").unwrap().parse().unwrap_or(3);
                opts.format = DiffFormat::Unified(n);
            }
            arg if arg.starts_with("--context=") => {
                let n: usize = arg.strip_prefix("--context=").unwrap().parse().unwrap_or(3);
                opts.format = DiffFormat::Context(n);
            }
            arg if arg.starts_with("--label=") => {
                opts.labels.push(arg.strip_prefix("--label=").unwrap().to_string());
            }
            arg if arg.starts_with('-') && arg.len() > 1 => {
                // Ignore unknown options silently for compatibility
            }
            _ => files.push(args[i].clone()),
        }
        i += 1;
    }

    if files.len() != 2 {
        eprintln!("diff: missing operand");
        process::exit(2);
    }

    let exit_code = diff_files(&files[0], &files[1], &opts);
    process::exit(exit_code);
}

#[derive(Default)]
struct DiffOpts {
    format: DiffFormat,
    brief: bool,
    report_identical: bool,
    recursive: bool,
    new_file: bool,
    ignore_case: bool,
    ignore_all_space: bool,
    ignore_space_change: bool,
    ignore_blank_lines: bool,
    text: bool,
    labels: Vec<String>,
}

#[derive(Default, Clone)]
enum DiffFormat {
    #[default]
    Normal,
    Unified(usize),
    Context(usize),
    Ed,
    Rcs,
    SideBySide,
}

fn read_lines(path: &str) -> io::Result<Vec<String>> {
    if path == "-" {
        let stdin = io::stdin();
        Ok(stdin.lock().lines().collect::<io::Result<Vec<_>>>()?)
    } else {
        let content = fs::read_to_string(path)?;
        Ok(content.lines().map(|s| s.to_string()).collect())
    }
}

fn normalize_line(line: &str, opts: &DiffOpts) -> String {
    let mut s = line.to_string();
    if opts.ignore_case {
        s = s.to_lowercase();
    }
    if opts.ignore_all_space {
        s.retain(|c| !c.is_whitespace());
    } else if opts.ignore_space_change {
        let mut result = String::new();
        let mut in_space = false;
        for c in s.chars() {
            if c.is_whitespace() {
                if !in_space {
                    result.push(' ');
                    in_space = true;
                }
            } else {
                result.push(c);
                in_space = false;
            }
        }
        s = result;
    }
    s
}

fn diff_files(path1: &str, path2: &str, opts: &DiffOpts) -> i32 {
    // Handle directories
    if Path::new(path1).is_dir() && Path::new(path2).is_dir() {
        if opts.recursive {
            return diff_dirs(path1, path2, opts);
        }
        eprintln!("diff: {path1}: Is a directory");
        return 2;
    }

    let lines1 = match read_lines(path1) {
        Ok(l) => l,
        Err(e) => {
            if opts.new_file {
                Vec::new()
            } else {
                eprintln!("diff: {path1}: {e}");
                return 2;
            }
        }
    };
    let lines2 = match read_lines(path2) {
        Ok(l) => l,
        Err(e) => {
            if opts.new_file {
                Vec::new()
            } else {
                eprintln!("diff: {path2}: {e}");
                return 2;
            }
        }
    };

    let edits = compute_diff(&lines1, &lines2, opts);

    if edits.is_empty() {
        if opts.report_identical {
            println!("Files {path1} and {path2} are identical");
        }
        return 0;
    }

    if opts.brief {
        println!("Files {path1} and {path2} differ");
        return 1;
    }

    let stdout = io::stdout();
    let mut out = stdout.lock();
    match &opts.format {
        DiffFormat::Normal => print_normal(&edits, &lines1, &lines2, &mut out),
        DiffFormat::Unified(ctx) => {
            let label1 = opts.labels.first().map(|s| s.as_str()).unwrap_or(path1);
            let label2 = opts.labels.get(1).map(|s| s.as_str()).unwrap_or(path2);
            print_unified(&edits, &lines1, &lines2, *ctx, label1, label2, &mut out);
        }
        DiffFormat::Context(ctx) => {
            let label1 = opts.labels.first().map(|s| s.as_str()).unwrap_or(path1);
            let label2 = opts.labels.get(1).map(|s| s.as_str()).unwrap_or(path2);
            print_context(&edits, &lines1, &lines2, *ctx, label1, label2, &mut out);
        }
        DiffFormat::Ed => print_ed(&edits, &lines2, &mut out),
        DiffFormat::Rcs => print_rcs(&edits, &lines1, &lines2, &mut out),
        DiffFormat::SideBySide => print_side_by_side(&edits, &lines1, &lines2, &mut out),
    }

    1
}

fn diff_dirs(dir1: &str, dir2: &str, opts: &DiffOpts) -> i32 {
    let mut entries1: Vec<String> = fs::read_dir(dir1)
        .unwrap_or_else(|_| { eprintln!("diff: {dir1}: No such file or directory"); process::exit(2); })
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
        .collect();
    let mut entries2: Vec<String> = fs::read_dir(dir2)
        .unwrap_or_else(|_| { eprintln!("diff: {dir2}: No such file or directory"); process::exit(2); })
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().to_string()))
        .collect();
    entries1.sort();
    entries2.sort();

    let mut all: Vec<String> = entries1.clone();
    for e in &entries2 {
        if !all.contains(e) {
            all.push(e.clone());
        }
    }
    all.sort();

    let mut result = 0;
    for name in &all {
        let p1 = format!("{dir1}/{name}");
        let p2 = format!("{dir2}/{name}");
        let in1 = entries1.contains(name);
        let in2 = entries2.contains(name);

        if in1 && in2 {
            let r = diff_files(&p1, &p2, opts);
            if r > result {
                result = r;
            }
        } else if in1 && !in2 {
            if opts.new_file {
                let r = diff_files(&p1, &p2, opts);
                if r > result { result = r; }
            } else {
                println!("Only in {dir1}: {name}");
                if result < 1 { result = 1; }
            }
        } else {
            if opts.new_file {
                let r = diff_files(&p1, &p2, opts);
                if r > result { result = r; }
            } else {
                println!("Only in {dir2}: {name}");
                if result < 1 { result = 1; }
            }
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Myers diff algorithm
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Edit {
    Equal(usize, usize),   // (line_idx_in_a, line_idx_in_b)
    Delete(usize),          // line_idx_in_a
    Insert(usize),          // line_idx_in_b
}

fn compute_diff(a: &[String], b: &[String], opts: &DiffOpts) -> Vec<Edit> {
    let na: Vec<String> = a.iter().map(|l| normalize_line(l, opts)).collect();
    let nb: Vec<String> = b.iter().map(|l| normalize_line(l, opts)).collect();

    let n = na.len();
    let m = nb.len();

    if n == 0 && m == 0 {
        return Vec::new();
    }
    if n == 0 {
        return (0..m).map(Edit::Insert).collect();
    }
    if m == 0 {
        return (0..n).map(Edit::Delete).collect();
    }

    // Myers algorithm for shortest edit script
    let max = n + m;
    let sz = 2 * max + 1;
    let mut v = vec![0i64; sz];
    let mut trace: Vec<Vec<i64>> = Vec::new();

    let idx = |k: i64| -> usize { (k + max as i64) as usize };

    'outer: for d in 0..=(max as i64) {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let mut x = if k == -d || (k != d && v[idx(k - 1)] < v[idx(k + 1)]) {
                v[idx(k + 1)]
            } else {
                v[idx(k - 1)] + 1
            };
            let mut y = x - k;
            while (x as usize) < n && (y as usize) < m && na[x as usize] == nb[y as usize] {
                x += 1;
                y += 1;
            }
            v[idx(k)] = x;
            if x as usize >= n && y as usize >= m {
                break 'outer;
            }
            k += 2;
        }
    }

    // Backtrack to build edit script
    let mut edits = Vec::new();
    let mut x = n as i64;
    let mut y = m as i64;

    for d in (0..trace.len()).rev() {
        let v = &trace[d];
        let d = d as i64;
        let k = x - y;

        let prev_k = if k == -d || (k != d && v[idx(k - 1)] < v[idx(k + 1)]) {
            k + 1
        } else {
            k - 1
        };

        let prev_x = v[idx(prev_k)];
        let prev_y = prev_x - prev_k;

        // Diagonal (equal lines)
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            edits.push(Edit::Equal(x as usize, y as usize));
        }

        if d > 0 {
            if x == prev_x {
                // Insert
                y -= 1;
                edits.push(Edit::Insert(y as usize));
            } else {
                // Delete
                x -= 1;
                edits.push(Edit::Delete(x as usize));
            }
        }
    }

    edits.reverse();

    // Filter blank line changes if requested
    if opts.ignore_blank_lines {
        edits.retain(|e| match e {
            Edit::Delete(i) => !a[*i].trim().is_empty(),
            Edit::Insert(i) => !b[*i].trim().is_empty(),
            Edit::Equal(_, _) => true,
        });
    }

    // Check if there are actual changes
    if edits.iter().all(|e| matches!(e, Edit::Equal(_, _))) {
        return Vec::new();
    }

    edits
}

// ---------------------------------------------------------------------------
// Output formats
// ---------------------------------------------------------------------------

fn print_normal(edits: &[Edit], a: &[String], b: &[String], out: &mut impl Write) {
    // Group consecutive edits into hunks
    let mut i = 0;
    while i < edits.len() {
        match &edits[i] {
            Edit::Equal(_, _) => { i += 1; }
            Edit::Delete(ai) => {
                let start = *ai;
                let mut end = start;
                let mut j = i + 1;
                while j < edits.len() {
                    if let Edit::Delete(aj) = &edits[j] {
                        if *aj == end + 1 { end = *aj; j += 1; } else { break; }
                    } else { break; }
                }
                // Check for following inserts (change)
                let mut ins_start = None;
                let mut ins_end = 0;
                let mut k = j;
                while k < edits.len() {
                    if let Edit::Insert(bi) = &edits[k] {
                        if ins_start.is_none() { ins_start = Some(*bi); }
                        ins_end = *bi;
                        k += 1;
                    } else { break; }
                }
                if let Some(is) = ins_start {
                    // Change
                    let _ = writeln!(out, "{}{}c{}{}",
                        start + 1, if end > start { format!(",{}", end + 1) } else { String::new() },
                        is + 1, if ins_end > is { format!(",{}", ins_end + 1) } else { String::new() });
                    for idx in start..=end { let _ = writeln!(out, "< {}", a[idx]); }
                    let _ = writeln!(out, "---");
                    for idx in is..=ins_end { let _ = writeln!(out, "> {}", b[idx]); }
                    i = k;
                } else {
                    // Delete
                    let _ = writeln!(out, "{}{}d{}",
                        start + 1, if end > start { format!(",{}", end + 1) } else { String::new() },
                        start); // approximate
                    for idx in start..=end { let _ = writeln!(out, "< {}", a[idx]); }
                    i = j;
                }
            }
            Edit::Insert(bi) => {
                let start = *bi;
                let mut end = start;
                let mut j = i + 1;
                while j < edits.len() {
                    if let Edit::Insert(bj) = &edits[j] {
                        if *bj == end + 1 { end = *bj; j += 1; } else { break; }
                    } else { break; }
                }
                // Find the position in file a
                let a_pos = edits[..i].iter().rev()
                    .find_map(|e| match e { Edit::Equal(ai, _) | Edit::Delete(ai) => Some(*ai + 1), _ => None })
                    .unwrap_or(0);
                let _ = writeln!(out, "{}a{}{}",
                    a_pos,
                    start + 1, if end > start { format!(",{}", end + 1) } else { String::new() });
                for idx in start..=end { let _ = writeln!(out, "> {}", b[idx]); }
                i = j;
            }
        }
    }
}

fn print_unified(edits: &[Edit], a: &[String], b: &[String], ctx: usize,
                 label1: &str, label2: &str, out: &mut impl Write) {
    let _ = writeln!(out, "--- {}", label1);
    let _ = writeln!(out, "+++ {}", label2);

    // Build hunks with context
    let hunks = build_hunks(edits, ctx);
    for hunk in hunks {
        let (a_start, a_count, b_start, b_count) = hunk_range(&hunk, a.len(), b.len());
        let _ = writeln!(out, "@@ -{},{} +{},{} @@", a_start + 1, a_count, b_start + 1, b_count);
        for edit in &hunk {
            match edit {
                Edit::Equal(ai, _) => { let _ = writeln!(out, " {}", a[*ai]); }
                Edit::Delete(ai) => { let _ = writeln!(out, "-{}", a[*ai]); }
                Edit::Insert(bi) => { let _ = writeln!(out, "+{}", b[*bi]); }
            }
        }
    }
}

fn print_context(edits: &[Edit], a: &[String], b: &[String], ctx: usize,
                 label1: &str, label2: &str, out: &mut impl Write) {
    let _ = writeln!(out, "*** {}", label1);
    let _ = writeln!(out, "--- {}", label2);

    let hunks = build_hunks(edits, ctx);
    for hunk in hunks {
        let (a_start, a_count, b_start, b_count) = hunk_range(&hunk, a.len(), b.len());
        let _ = writeln!(out, "***************");
        let _ = writeln!(out, "*** {},{} ****", a_start + 1, a_start + a_count);
        for edit in &hunk {
            match edit {
                Edit::Equal(ai, _) => { let _ = writeln!(out, "  {}", a[*ai]); }
                Edit::Delete(ai) => { let _ = writeln!(out, "- {}", a[*ai]); }
                Edit::Insert(_) => {}
            }
        }
        let _ = writeln!(out, "--- {},{} ----", b_start + 1, b_start + b_count);
        for edit in &hunk {
            match edit {
                Edit::Equal(_, bi) => { let _ = writeln!(out, "  {}", b[*bi]); }
                Edit::Delete(_) => {}
                Edit::Insert(bi) => { let _ = writeln!(out, "+ {}", b[*bi]); }
            }
        }
    }
}

fn print_ed(edits: &[Edit], b: &[String], out: &mut impl Write) {
    // ed format: output changes in reverse order
    let mut i = edits.len();
    while i > 0 {
        i -= 1;
        match &edits[i] {
            Edit::Equal(_, _) => {}
            Edit::Delete(ai) => { let _ = writeln!(out, "{}d", ai + 1); }
            Edit::Insert(bi) => {
                let a_pos = edits[..=i].iter().rev()
                    .find_map(|e| match e { Edit::Equal(ai, _) | Edit::Delete(ai) => Some(*ai + 1), _ => None })
                    .unwrap_or(0);
                let _ = writeln!(out, "{}a", a_pos);
                let _ = writeln!(out, "{}", b[*bi]);
                let _ = writeln!(out, ".");
            }
        }
    }
}

fn print_rcs(edits: &[Edit], a: &[String], b: &[String], out: &mut impl Write) {
    for edit in edits {
        match edit {
            Edit::Equal(_, _) => {}
            Edit::Delete(ai) => { let _ = writeln!(out, "d{} 1", ai + 1); }
            Edit::Insert(bi) => {
                let a_pos = edits.iter()
                    .take_while(|e| !std::ptr::eq(*e, edit))
                    .filter_map(|e| match e { Edit::Equal(ai, _) | Edit::Delete(ai) => Some(*ai + 1), _ => None })
                    .last()
                    .unwrap_or(0);
                let _ = writeln!(out, "a{} 1", a_pos);
                let _ = writeln!(out, "{}", b[*bi]);
            }
        }
    }
    let _ = a; // suppress warning
}

fn print_side_by_side(edits: &[Edit], a: &[String], b: &[String], out: &mut impl Write) {
    let width = 80;
    let col = (width - 3) / 2;
    for edit in edits {
        match edit {
            Edit::Equal(ai, _) => {
                let _ = writeln!(out, "{:<col$}   {}", truncate(&a[*ai], col), truncate(&a[*ai], col));
            }
            Edit::Delete(ai) => {
                let _ = writeln!(out, "{:<col$} <", truncate(&a[*ai], col));
            }
            Edit::Insert(bi) => {
                let _ = writeln!(out, "{:<col$} > {}", "", truncate(&b[*bi], col));
            }
        }
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}...", &s[..max.saturating_sub(3)]) }
}

fn build_hunks(edits: &[Edit], ctx: usize) -> Vec<Vec<Edit>> {
    let mut hunks: Vec<Vec<Edit>> = Vec::new();
    let mut current: Vec<Edit> = Vec::new();
    let mut last_change = None;

    for (i, edit) in edits.iter().enumerate() {
        let is_change = !matches!(edit, Edit::Equal(_, _));
        if is_change {
            // Add context before
            let ctx_start = if i >= ctx { i - ctx } else { 0 };
            if current.is_empty() {
                for j in ctx_start..i {
                    if matches!(edits[j], Edit::Equal(_, _)) {
                        current.push(edits[j].clone());
                    }
                }
            } else if let Some(lc) = last_change {
                // Fill gap between last change and this one
                for j in (lc + 1)..i {
                    current.push(edits[j].clone());
                }
            }
            current.push(edit.clone());
            last_change = Some(i);
        } else if let Some(lc) = last_change {
            if i - lc <= ctx * 2 {
                current.push(edit.clone());
            } else if i - lc == ctx * 2 + 1 {
                // Add remaining context and start new hunk
                current.push(edit.clone());
            } else if i - lc == ctx + 1 {
                // Just finished context after
                current.push(edit.clone());
            } else if !current.is_empty() && i == lc + ctx + 1 {
                // One past context
            } else if !current.is_empty() {
                hunks.push(current);
                current = Vec::new();
                last_change = None;
            }
        }
    }

    // Add trailing context
    if let Some(lc) = last_change {
        for j in (lc + 1)..edits.len().min(lc + ctx + 1) {
            if matches!(edits[j], Edit::Equal(_, _)) {
                current.push(edits[j].clone());
            }
        }
    }

    if !current.is_empty() {
        hunks.push(current);
    }

    hunks
}

fn hunk_range(hunk: &[Edit], _a_len: usize, _b_len: usize) -> (usize, usize, usize, usize) {
    let mut a_start = usize::MAX;
    let mut a_end = 0;
    let mut b_start = usize::MAX;
    let mut b_end = 0;

    for edit in hunk {
        match edit {
            Edit::Equal(ai, bi) => {
                a_start = a_start.min(*ai);
                a_end = a_end.max(*ai + 1);
                b_start = b_start.min(*bi);
                b_end = b_end.max(*bi + 1);
            }
            Edit::Delete(ai) => {
                a_start = a_start.min(*ai);
                a_end = a_end.max(*ai + 1);
            }
            Edit::Insert(bi) => {
                b_start = b_start.min(*bi);
                b_end = b_end.max(*bi + 1);
            }
        }
    }

    if a_start == usize::MAX { a_start = 0; }
    if b_start == usize::MAX { b_start = 0; }

    (a_start, a_end - a_start, b_start, b_end - b_start)
}

// ---------------------------------------------------------------------------
// cmp
// ---------------------------------------------------------------------------

fn cmp_main(args: &[String]) {
    let mut silent = false;
    let mut files = Vec::new();

    for arg in args {
        match arg.as_str() {
            "--version" => {
                println!("cmp (rust-diffutils) {}", env!("CARGO_PKG_VERSION"));
                process::exit(0);
            }
            "-s" | "--silent" | "--quiet" => silent = true,
            "-l" | "--verbose" => {} // TODO
            arg if arg.starts_with('-') => {}
            _ => files.push(arg.clone()),
        }
    }

    if files.len() < 2 {
        eprintln!("cmp: missing operand");
        process::exit(2);
    }

    let data1 = match fs::read(&files[0]) {
        Ok(d) => d,
        Err(e) => { eprintln!("cmp: {}: {e}", files[0]); process::exit(2); }
    };
    let data2 = match fs::read(&files[1]) {
        Ok(d) => d,
        Err(e) => { eprintln!("cmp: {}: {e}", files[1]); process::exit(2); }
    };

    let mut byte_pos = 0usize;
    let mut line = 1usize;

    for (b1, b2) in data1.iter().zip(data2.iter()) {
        byte_pos += 1;
        if *b1 == b'\n' { line += 1; }
        if b1 != b2 {
            if !silent {
                println!("{} {} differ: byte {byte_pos}, line {line}", files[0], files[1]);
            }
            process::exit(1);
        }
    }

    if data1.len() != data2.len() {
        if !silent {
            let shorter = if data1.len() < data2.len() { &files[0] } else { &files[1] };
            eprintln!("cmp: EOF on {shorter}");
        }
        process::exit(1);
    }

    process::exit(0);
}

// ---------------------------------------------------------------------------
// sdiff (stub)
// ---------------------------------------------------------------------------

fn sdiff_main(args: &[String]) {
    for arg in args {
        if arg == "--version" {
            println!("sdiff (rust-diffutils) {}", env!("CARGO_PKG_VERSION"));
            process::exit(0);
        }
    }
    // sdiff is side-by-side diff; delegate to diff -y
    let mut new_args = vec!["-y".to_string()];
    new_args.extend_from_slice(args);
    diff_main(&new_args);
}

// ---------------------------------------------------------------------------
// diff3 (stub)
// ---------------------------------------------------------------------------

fn diff3_main(args: &[String]) {
    for arg in args {
        if arg == "--version" {
            println!("diff3 (rust-diffutils) {}", env!("CARGO_PKG_VERSION"));
            process::exit(0);
        }
    }
    eprintln!("diff3: not yet implemented");
    process::exit(2);
}
