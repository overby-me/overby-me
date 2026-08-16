pub fn setup_logging(conf: &crate::config::LoggingConfig) -> Result<(), String> {
    let mut logger = fern::Dispatch::new()
        .format(|out, message, record| {
            let level = record.level();
            let colored_level = match level {
                log::Level::Error => format!("\x1b[31m{}\x1b[0m", level),
                log::Level::Warn => format!("\x1b[33m{}\x1b[0m", level),
                log::Level::Info => format!("\x1b[32m{}\x1b[0m", level),
                log::Level::Debug => format!("\x1b[34m{}\x1b[0m", level),
                log::Level::Trace => format!("\x1b[36m{}\x1b[0m", level),
            };
            out.finish(format_args!(
                "{}[{}][{}] {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                colored_level,
                message
            ));
        })
        .level(log::LevelFilter::Info);

    if conf.log_to_stdout {
        logger = logger.chain(std::io::stdout());
    }

    if conf.log_to_disk {
        // Disk logging is not implemented (pipe stdout to a logging solution
        // instead). Do NOT abort the manager on a config that requests it --
        // fall back to stdout so logs are not lost.
        eprintln!(
            "rust-systemd: logging to disk is not supported; logging to stdout instead"
        );
        if !conf.log_to_stdout {
            logger = logger.chain(std::io::stdout());
        }
    }

    logger
        .apply()
        .map_err(|e| format!("Error while setting up logger: {e}"))
}
