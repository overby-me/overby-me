//! `wclip -i` — take ownership of a selection and serve it to pasting clients.

use std::fs::File;
use std::io::{self, Read, Write};

use crate::args::{Args, Selection};
use crate::proto::{self, State};
use crate::wire::Connection;

/// MIME types offered for text when no explicit `-target` is given. The plain
/// `TEXT`/`STRING`/`UTF8_STRING` aliases let XWayland clients paste too.
const DEFAULT_TEXT_MIMES: &[&str] = &[
    "text/plain;charset=utf-8",
    "text/plain",
    "TEXT",
    "STRING",
    "UTF8_STRING",
];

pub fn run(args: &Args) -> io::Result<()> {
    let mut data = read_input(args)?;
    if args.rmlastnl && data.last() == Some(&b'\n') {
        data.pop();
    }

    if args.filter {
        let mut out = io::stdout();
        out.write_all(&data)?;
        out.flush()?;
    }

    let mimes: Vec<String> = match &args.target {
        Some(t) => vec![t.clone()],
        None => DEFAULT_TEXT_MIMES.iter().map(|s| s.to_string()).collect(),
    };

    let mut conn = Connection::connect(args.display.as_deref())?;
    let mut st = State::new();
    proto::setup(&mut conn, &mut st)?;

    let primary = args.selection == Selection::Primary;
    if primary && !st.primary_supported() {
        return Err(io::Error::other(
            "the compositor does not support the primary selection",
        ));
    }

    proto::set_selection(&mut conn, &mut st, &mimes, primary)?;
    if st.cancelled {
        // Another client immediately replaced us; nothing left to serve.
        return Ok(());
    }

    st.data = data;
    st.loops = args.loops;

    // Detach into the background so the shell prompt returns while we keep
    // serving the selection — unless the user asked to stay in the foreground.
    if !args.foreground {
        daemonize()?;
    }

    proto::serve(&mut conn, &mut st)
}

/// Read the data to copy: concatenated file contents, or stdin when no files
/// are given (a `-` argument also means stdin).
fn read_input(args: &Args) -> io::Result<Vec<u8>> {
    if args.files.is_empty() {
        let mut data = Vec::new();
        io::stdin().lock().read_to_end(&mut data)?;
        return Ok(data);
    }

    let mut data = Vec::new();
    for file in &args.files {
        if file == "-" {
            io::stdin().lock().read_to_end(&mut data)?;
        } else {
            // fs::read would replace `data` rather than append to it, and it
            // cannot name the file in the error the way this does.
            #[allow(clippy::verbose_file_reads)]
            {
                let mut fh = File::open(file)
                    .map_err(|e| io::Error::new(e.kind(), format!("{file}: {e}")))?;
                fh.read_to_end(&mut data)?;
            }
        }
    }
    Ok(data)
}

/// Fork into a detached background process that continues serving the
/// selection. The original process exits, returning control to the shell; the
/// child gets a new session and redirects its standard streams to `/dev/null`.
fn daemonize() -> io::Result<()> {
    io::stdout().flush().ok();
    io::stderr().flush().ok();

    // SAFETY: a plain fork; the child only performs async-signal-safe calls.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        // Fall back to foreground serving rather than failing the copy.
        return Ok(());
    }
    if pid > 0 {
        // Parent: leave the child running and return success immediately.
        unsafe { libc::_exit(0) };
    }

    // Child: detach from the controlling terminal and silence std streams.
    unsafe {
        libc::setsid();
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if null >= 0 {
            libc::dup2(null, 0);
            libc::dup2(null, 1);
            libc::dup2(null, 2);
            if null > 2 {
                libc::close(null);
            }
        }
    }
    Ok(())
}
