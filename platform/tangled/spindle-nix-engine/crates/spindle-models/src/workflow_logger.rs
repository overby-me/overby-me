//! Workflow logger abstraction for writing log lines to persistent storage.
//!
//! Matches the upstream Go `WorkflowLogger` interface, `FileWorkflowLogger`,
//! and `NullLogger` types from `logger.go`.
//!
//! The logger writes [`LogLine`] entries as newline-delimited JSON (NDJSON)
//! to per-workflow log files. Secret values are masked before writing using
//! [`SecretMask`].
//!
//! # Architecture
//!
//! The logger provides two kinds of writers:
//! - **Data writers** for step stdout/stderr output.
//! - **Control writers** for step lifecycle events (start/end).
//!
//! Each writer implements [`std::io::Write`], so it can be used directly with
//! `std::io::BufReader::lines()` or similar line-oriented I/O. Internally,
//! each `write()` call produces one [`LogLine`] JSON entry in the log file.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::log_line::{LogLine, StepStatus};
use crate::secret_mask::SecretMask;
use crate::step::Step;
use crate::workflow::WorkflowId;

/// Trait for writing workflow log entries.
///
/// Matches the upstream Go `WorkflowLogger` interface:
/// ```go
/// type WorkflowLogger interface {
///     Close() error
///     DataWriter(idx int, stream string) io.Writer
///     ControlWriter(idx int, step Step, stepStatus StepStatus) io.Writer
/// }
/// ```
///
/// Implementations must be `Send + Sync` so they can be shared across async
/// tasks (e.g. stdout and stderr readers running concurrently).
pub trait WorkflowLogger: Send + Sync {
    /// Close the logger, flushing any buffered data.
    fn close(&self) -> io::Result<()>;

    /// Create a writer for step output data (stdout/stderr).
    ///
    /// Each `write()` call on the returned writer produces a [`LogLine`] with
    /// [`LogKind::Data`](crate::log_line::LogKind::Data).
    ///
    /// # Arguments
    /// * `step_id` — Zero-based index of the step.
    /// * `stream` — The stream name (e.g. `"stdout"`, `"stderr"`).
    fn data_writer(&self, step_id: usize, stream: String) -> Box<dyn Write + Send>;

    /// Create a writer for step lifecycle control events.
    ///
    /// A single `write()` call on the returned writer (with any payload — the
    /// content is ignored) produces a [`LogLine`] with
    /// [`LogKind::Control`](crate::log_line::LogKind::Control).
    ///
    /// # Arguments
    /// * `step_id` — Zero-based index of the step.
    /// * `step` — The step whose lifecycle is being reported.
    /// * `step_status` — Whether the step is starting or ending.
    fn control_writer(
        &self,
        step_id: usize,
        step: &dyn Step,
        step_status: StepStatus,
    ) -> Box<dyn Write + Send>;
}

/// A no-op logger that discards all output.
///
/// Matches the upstream Go `NullLogger` type. Useful for testing or when
/// log output is not needed.
#[derive(Debug, Clone, Copy)]
pub struct NullLogger;

impl WorkflowLogger for NullLogger {
    fn close(&self) -> io::Result<()> {
        Ok(())
    }

    fn data_writer(&self, _step_id: usize, _stream: String) -> Box<dyn Write + Send> {
        Box::new(io::sink())
    }

    fn control_writer(
        &self,
        _step_id: usize,
        _step: &dyn Step,
        _step_status: StepStatus,
    ) -> Box<dyn Write + Send> {
        Box::new(io::sink())
    }
}

/// Compute the log file path for a workflow.
///
/// Format: `{base_dir}/{workflow_id}.log`
///
/// Matches the upstream Go `LogFilePath` function.
pub fn log_file_path(base_dir: &Path, workflow_id: &WorkflowId) -> PathBuf {
    base_dir.join(format!("{}.log", workflow_id))
}

/// Shared inner state for [`FileWorkflowLogger`].
///
/// Protected by a mutex so multiple writers (stdout + stderr) can safely
/// write interleaved log lines to the same file.
struct FileLoggerInner {
    file: std::fs::File,
    mask: Option<SecretMask>,
}

/// A logger that writes NDJSON [`LogLine`] entries to a file.
///
/// Matches the upstream Go `FileWorkflowLogger` type. Each line in the file
/// is a JSON-serialized [`LogLine`], terminated by a newline.
///
/// Secret values are masked using [`SecretMask`] before writing data log lines.
///
/// # Thread Safety
///
/// The inner file handle is protected by a `Mutex` wrapped in an `Arc`, so
/// the logger can be shared across async tasks. Each `DataWriter` and
/// `ControlWriter` holds a clone of the `Arc`.
#[derive(Clone)]
pub struct FileWorkflowLogger {
    inner: Arc<Mutex<FileLoggerInner>>,
}

impl FileWorkflowLogger {
    /// Create a new file workflow logger.
    ///
    /// Creates (or appends to) the log file at `{base_dir}/{workflow_id}.log`.
    /// The `secret_values` are used to construct a [`SecretMask`] for redacting
    /// secret content from data log lines.
    ///
    /// The parent directory is created if it doesn't exist.
    pub fn new(
        base_dir: &Path,
        workflow_id: &WorkflowId,
        secret_values: &[impl AsRef<str>],
    ) -> io::Result<Self> {
        let path = log_file_path(base_dir, workflow_id);

        // Ensure the parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = OpenOptions::new().create(true).append(true).open(&path)?;

        let mask = SecretMask::new(secret_values);

        Ok(Self {
            inner: Arc::new(Mutex::new(FileLoggerInner { file, mask })),
        })
    }

    /// Write a single [`LogLine`] to the log file as JSON + newline.
    fn write_log_line(&self, line: &LogLine) -> io::Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|e| io::Error::other(e.to_string()))?;

        serde_json::to_writer(&mut inner.file, line).map_err(io::Error::other)?;
        inner.file.write_all(b"\n")?;
        inner.file.flush()?;
        Ok(())
    }
}

impl WorkflowLogger for FileWorkflowLogger {
    fn close(&self) -> io::Result<()> {
        let inner = self
            .inner
            .lock()
            .map_err(|e| io::Error::other(e.to_string()))?;
        inner.file.sync_all()
    }

    fn data_writer(&self, step_id: usize, stream: String) -> Box<dyn Write + Send> {
        Box::new(DataWriter {
            logger: self.clone(),
            step_id,
            stream,
        })
    }

    fn control_writer(
        &self,
        step_id: usize,
        step: &dyn Step,
        step_status: StepStatus,
    ) -> Box<dyn Write + Send> {
        Box::new(ControlWriter {
            logger: self.clone(),
            step_id,
            step_name: step.name().to_owned(),
            step_kind: step.kind(),
            step_command: step.command(),
            step_status,
        })
    }
}

/// Writer that produces data log lines from step output.
///
/// Each `write()` call trims trailing `\r\n`, applies secret masking,
/// and writes a [`LogLine`] with [`LogKind::Data`](crate::log_line::LogKind::Data).
///
/// Matches the upstream Go `dataWriter` struct.
struct DataWriter {
    logger: FileWorkflowLogger,
    step_id: usize,
    stream: String,
}

impl Write for DataWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let text = String::from_utf8_lossy(buf);
        let trimmed = text.trim_end_matches(['\r', '\n']);

        // Apply secret masking
        let content = {
            let inner = self
                .logger
                .inner
                .lock()
                .map_err(|e| io::Error::other(e.to_string()))?;
            SecretMask::mask_optional(inner.mask.as_ref(), trimmed)
        };

        let line = LogLine::data(self.step_id, content, &self.stream);
        self.logger.write_log_line(&line)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Writer that produces control log lines for step lifecycle events.
///
/// A single `write()` call (content is ignored) produces a [`LogLine`] with
/// [`LogKind::Control`](crate::log_line::LogKind::Control).
///
/// Matches the upstream Go `controlWriter` struct.
struct ControlWriter {
    logger: FileWorkflowLogger,
    step_id: usize,
    step_name: String,
    step_kind: crate::step::StepKind,
    step_command: String,
    step_status: StepStatus,
}

impl Write for ControlWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let line = LogLine {
            kind: crate::log_line::LogKind::Control,
            content: self.step_name.clone(),
            time: chrono::Utc::now(),
            step_id: self.step_id,
            stream: None,
            step_status: Some(self.step_status),
            step_kind: Some(self.step_kind),
            step_command: Some(self.step_command.clone()),
        };
        self.logger.write_log_line(&line)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_line::{LogKind, StepStatus};
    use crate::step::{StepKind, UserStep};
    use std::io::Write;

    #[test]
    fn log_file_path_format() {
        use crate::pipeline::PipelineId;

        let wid = WorkflowId::new(
            PipelineId {
                knot: "example.com".into(),
                rkey: "abc123".into(),
            },
            "test",
        );
        let path = log_file_path(Path::new("/var/log/spindle"), &wid);
        assert_eq!(
            path,
            PathBuf::from("/var/log/spindle/example.com-abc123-test.log")
        );
    }

    #[test]
    fn null_logger_close() {
        let logger = NullLogger;
        assert!(logger.close().is_ok());
    }

    #[test]
    fn null_logger_data_writer_discards() {
        let logger = NullLogger;
        let mut writer = logger.data_writer(0, "stdout".into());
        let result = writer.write(b"hello world\n");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 12);
    }

    #[test]
    fn null_logger_control_writer_discards() {
        let logger = NullLogger;
        let step = UserStep::new("Build", "cargo build");
        let mut writer = logger.control_writer(0, &step, StepStatus::Start);
        let result = writer.write(b"trigger");
        assert!(result.is_ok());
    }

    #[test]
    fn file_logger_writes_ndjson() {
        use crate::pipeline::PipelineId;

        let dir = tempdir();
        let wid = WorkflowId::new(
            PipelineId {
                knot: "example.com".into(),
                rkey: "abc".into(),
            },
            "test",
        );

        let logger = FileWorkflowLogger::new(dir.path(), &wid, &Vec::<String>::new()).unwrap();

        // Write a data line
        {
            let mut writer = logger.data_writer(0, "stdout".into());
            writer.write_all(b"hello world\n").unwrap();
        }

        // Write a control line
        {
            let step = UserStep::new("Build", "cargo build");
            let mut writer = logger.control_writer(0, &step, StepStatus::Start);
            writer.write_all(b"trigger").unwrap();
        }

        logger.close().unwrap();

        // Read the log file and verify
        let path = log_file_path(dir.path(), &wid);
        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.trim().split('\n').collect();
        assert_eq!(lines.len(), 2);

        let data_line: LogLine = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(data_line.kind, LogKind::Data);
        assert_eq!(data_line.content, "hello world");
        assert_eq!(data_line.step_id, 0);
        assert_eq!(data_line.stream.as_deref(), Some("stdout"));

        let control_line: LogLine = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(control_line.kind, LogKind::Control);
        assert_eq!(control_line.content, "Build");
        assert_eq!(control_line.step_id, 0);
        assert_eq!(control_line.step_status, Some(StepStatus::Start));
        assert_eq!(control_line.step_kind, Some(StepKind::User));
        assert_eq!(control_line.step_command.as_deref(), Some("cargo build"));
    }

    #[test]
    fn file_logger_masks_secrets() {
        use crate::pipeline::PipelineId;

        let dir = tempdir();
        let wid = WorkflowId::new(
            PipelineId {
                knot: "example.com".into(),
                rkey: "abc".into(),
            },
            "masked",
        );

        let secrets = vec!["my-api-key".to_string(), "other-secret".to_string()];
        let logger = FileWorkflowLogger::new(dir.path(), &wid, &secrets).unwrap();

        {
            let mut writer = logger.data_writer(0, "stdout".into());
            writer
                .write_all(b"Token: my-api-key and other-secret\n")
                .unwrap();
        }

        logger.close().unwrap();

        let path = log_file_path(dir.path(), &wid);
        let content = fs::read_to_string(&path).unwrap();
        let line: LogLine = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(line.content, "Token: *** and ***");
    }

    #[test]
    fn file_logger_trims_trailing_newlines() {
        use crate::pipeline::PipelineId;

        let dir = tempdir();
        let wid = WorkflowId::new(
            PipelineId {
                knot: "example.com".into(),
                rkey: "abc".into(),
            },
            "trim",
        );

        let logger = FileWorkflowLogger::new(dir.path(), &wid, &Vec::<String>::new()).unwrap();

        {
            let mut writer = logger.data_writer(0, "stdout".into());
            writer.write_all(b"line with trailing\r\n").unwrap();
        }

        logger.close().unwrap();

        let path = log_file_path(dir.path(), &wid);
        let content = fs::read_to_string(&path).unwrap();
        let line: LogLine = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(line.content, "line with trailing");
    }

    #[test]
    fn file_logger_creates_parent_dirs() {
        use crate::pipeline::PipelineId;

        let dir = tempdir();
        let nested = dir.path().join("deep").join("nested").join("dir");
        let wid = WorkflowId::new(
            PipelineId {
                knot: "example.com".into(),
                rkey: "abc".into(),
            },
            "nested",
        );

        let logger = FileWorkflowLogger::new(&nested, &wid, &Vec::<String>::new()).unwrap();
        logger.close().unwrap();

        let path = log_file_path(&nested, &wid);
        assert!(path.exists());
    }

    /// Create a temporary directory for tests.
    /// Returns a `TempDir`-like wrapper that cleans up on drop.
    fn tempdir() -> TempDir {
        let path = std::env::temp_dir().join(format!("spindle-test-{}", uuid_v4()));
        fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    /// Simple UUID v4 generation for unique temp dir names.
    fn uuid_v4() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let pid = std::process::id();
        format!("{t:x}-{pid:x}")
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
}
