//! Step types for workflow execution.
//!
//! A step is a single unit of execution within a workflow. Steps implement the
//! [`Step`] trait, which provides the step's name, command(s), and kind.
//!
//! There are two kinds of steps:
//! - **System** steps are injected by the CI runner (e.g. the clone step).
//! - **User** steps are defined by the user in the workflow YAML.
//!
//! Matches the upstream Go `Step` interface, `StepKind` enum, and `CloneStep` struct.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The kind of a step: system-injected or user-defined.
///
/// Serialized as an integer to match the upstream Go `iota` representation
/// (System = 0, User = 1) for wire compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum StepKind {
    /// Steps injected by the CI runner (e.g. clone).
    System = 0,
    /// Steps defined by the user in the pipeline workflow YAML.
    User = 1,
}

impl Serialize for StepKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u8(*self as u8)
    }
}

impl<'de> Deserialize<'de> for StepKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u8::deserialize(deserializer)?;
        match value {
            0 => Ok(StepKind::System),
            1 => Ok(StepKind::User),
            other => Err(serde::de::Error::custom(format!(
                "invalid StepKind value: {other} (expected 0 or 1)"
            ))),
        }
    }
}

impl fmt::Display for StepKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StepKind::System => f.write_str("system"),
            StepKind::User => f.write_str("user"),
        }
    }
}

/// A single executable step within a workflow.
///
/// This trait mirrors the upstream Go `Step` interface:
/// ```go
/// type Step interface {
///     Name() string
///     Command() string
///     Kind() StepKind
/// }
/// ```
///
/// Implementors must also be serializable so that workflows can be persisted
/// and transmitted over the wire.
pub trait Step: fmt::Debug + Send + Sync {
    /// Human-readable name of this step (e.g. `"Clone repository into workspace"`).
    fn name(&self) -> &str;

    /// The shell command(s) to execute for this step.
    ///
    /// For multi-line commands (like [`CloneStep`]), individual commands are
    /// joined with newlines, matching the upstream Go `CloneStep.Command()`.
    fn command(&self) -> String;

    /// Whether this step was injected by the system or defined by the user.
    fn kind(&self) -> StepKind;

    /// Clone this step into a boxed trait object.
    fn clone_boxed(&self) -> Box<dyn Step>;
}

impl Clone for Box<dyn Step> {
    fn clone(&self) -> Self {
        self.clone_boxed()
    }
}

// Implement Serialize for Box<dyn Step> by serializing a generic representation.
impl Serialize for Box<dyn Step> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("Step", 3)?;
        s.serialize_field("name", self.name())?;
        s.serialize_field("command", &self.command())?;
        s.serialize_field("kind", &self.kind())?;
        s.end()
    }
}

// Deserialize Box<dyn Step> as a UserStep (the most general representation).
impl<'de> Deserialize<'de> for Box<dyn Step> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct StepData {
            name: String,
            command: String,
            kind: StepKind,
        }
        let data = StepData::deserialize(deserializer)?;
        match data.kind {
            StepKind::System => Ok(Box::new(CloneStep {
                name: data.name,
                commands: data.command.lines().map(|l| l.to_owned()).collect(),
            })),
            StepKind::User => Ok(Box::new(UserStep {
                name: data.name,
                command: data.command,
            })),
        }
    }
}

/// A user-defined step from the workflow YAML.
///
/// This represents a single `steps:` entry in a `.tangled/workflows/*.yml` file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserStep {
    /// Human-readable step name.
    pub name: String,
    /// Shell command to execute (passed to `bash -euo pipefail -c`).
    pub command: String,
}

impl UserStep {
    /// Create a new user step.
    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
        }
    }
}

impl Step for UserStep {
    fn name(&self) -> &str {
        &self.name
    }

    fn command(&self) -> String {
        self.command.clone()
    }

    fn kind(&self) -> StepKind {
        StepKind::User
    }

    fn clone_boxed(&self) -> Box<dyn Step> {
        Box::new(self.clone())
    }
}

/// A system-injected step for cloning the repository into the workspace.
///
/// Matches the upstream Go `CloneStep` struct. The step generates a sequence
/// of git commands (`git init`, `git remote add`, `git fetch`, `git checkout`)
/// that are joined with newlines when accessed via [`Step::command()`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloneStep {
    /// Human-readable step name (e.g. `"Clone repository into workspace"`).
    pub name: String,
    /// Individual git commands to execute.
    pub commands: Vec<String>,
}

/// Options for the clone step, parsed from the workflow YAML `clone:` field.
///
/// Matches the upstream Go `Pipeline_CloneOpts` / `CloneOpts` types.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CloneOpts {
    /// Fetch depth (0 means use default of 1 for shallow clone).
    #[serde(default)]
    pub depth: u32,
    /// Whether to skip cloning entirely.
    #[serde(default)]
    pub skip: bool,
    /// Whether to recurse into submodules.
    #[serde(default)]
    pub submodules: bool,
}

impl CloneStep {
    /// Build a clone step from trigger metadata and clone options.
    ///
    /// Generates git commands matching the upstream Go `BuildCloneStep`:
    /// 1. `git init`
    /// 2. `git remote add origin <url>`
    /// 3. `git fetch --depth=<d> [--recurse-submodules=yes] origin [<sha>]`
    /// 4. `git checkout FETCH_HEAD`
    ///
    /// If `clone_opts.skip` is true, returns an empty (no-op) clone step.
    pub fn build(repo_url: &str, commit_sha: Option<&str>, clone_opts: Option<&CloneOpts>) -> Self {
        let default_opts = CloneOpts::default();
        let opts = clone_opts.unwrap_or(&default_opts);

        if opts.skip {
            return CloneStep {
                name: String::new(),
                commands: Vec::new(),
            };
        }

        let depth = if opts.depth == 0 { 1 } else { opts.depth };

        let mut fetch_args = vec![format!("--depth={depth}")];
        if opts.submodules {
            fetch_args.push("--recurse-submodules=yes".into());
        }
        fetch_args.push("origin".into());
        if let Some(sha) = commit_sha
            && !sha.is_empty()
        {
            fetch_args.push(sha.to_owned());
        }

        CloneStep {
            name: "Clone repository into workspace".into(),
            commands: vec![
                "git init".into(),
                format!("git remote add origin {repo_url}"),
                format!("git fetch {}", fetch_args.join(" ")),
                "git checkout FETCH_HEAD".into(),
            ],
        }
    }

    /// Returns `true` if this is an empty/skipped clone step.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

impl Step for CloneStep {
    fn name(&self) -> &str {
        &self.name
    }

    /// Returns all commands joined with newlines, matching upstream Go
    /// `CloneStep.Command()` which does `strings.Join(s.commands, "\n")`.
    fn command(&self) -> String {
        self.commands.join("\n")
    }

    fn kind(&self) -> StepKind {
        StepKind::System
    }

    fn clone_boxed(&self) -> Box<dyn Step> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_kind_display() {
        assert_eq!(StepKind::System.to_string(), "system");
        assert_eq!(StepKind::User.to_string(), "user");
    }

    #[test]
    fn step_kind_repr() {
        assert_eq!(StepKind::System as u8, 0);
        assert_eq!(StepKind::User as u8, 1);
    }

    #[test]
    fn user_step_basic() {
        let step = UserStep::new("Run tests", "cargo test");
        assert_eq!(step.name(), "Run tests");
        assert_eq!(step.command(), "cargo test");
        assert_eq!(step.kind(), StepKind::User);
    }

    #[test]
    fn user_step_serialization_roundtrip() {
        let step = UserStep::new("Build", "cargo build --release");
        let json = serde_json::to_string(&step).unwrap();
        let deserialized: UserStep = serde_json::from_str(&json).unwrap();
        assert_eq!(step, deserialized);
    }

    #[test]
    fn clone_step_build_basic() {
        let step = CloneStep::build(
            "https://example.com/did:plc:user123/my-repo",
            Some("abc123def456"),
            None,
        );

        assert_eq!(step.name(), "Clone repository into workspace");
        assert_eq!(step.kind(), StepKind::System);
        assert_eq!(step.commands.len(), 4);
        assert_eq!(step.commands[0], "git init");
        assert_eq!(
            step.commands[1],
            "git remote add origin https://example.com/did:plc:user123/my-repo"
        );
        assert_eq!(step.commands[2], "git fetch --depth=1 origin abc123def456");
        assert_eq!(step.commands[3], "git checkout FETCH_HEAD");
    }

    #[test]
    fn clone_step_build_with_options() {
        let opts = CloneOpts {
            depth: 10,
            skip: false,
            submodules: true,
        };
        let step = CloneStep::build(
            "https://example.com/did:plc:user123/my-repo",
            Some("abc123"),
            Some(&opts),
        );

        assert_eq!(step.commands.len(), 4);
        assert_eq!(
            step.commands[2],
            "git fetch --depth=10 --recurse-submodules=yes origin abc123"
        );
    }

    #[test]
    fn clone_step_build_skip() {
        let opts = CloneOpts {
            skip: true,
            ..Default::default()
        };
        let step = CloneStep::build("https://example.com/repo", Some("abc"), Some(&opts));

        assert!(step.is_empty());
        assert_eq!(step.name(), "");
        assert_eq!(step.command(), "");
    }

    #[test]
    fn clone_step_build_no_sha() {
        let step = CloneStep::build("https://example.com/repo", None, None);

        assert_eq!(step.commands[2], "git fetch --depth=1 origin");
    }

    #[test]
    fn clone_step_build_empty_sha() {
        let step = CloneStep::build("https://example.com/repo", Some(""), None);

        assert_eq!(step.commands[2], "git fetch --depth=1 origin");
    }

    #[test]
    fn clone_step_command_joins_with_newlines() {
        let step = CloneStep::build("https://example.com/repo", Some("sha1"), None);

        let command = step.command();
        let lines: Vec<&str> = command.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[0], "git init");
        assert_eq!(lines[1], "git remote add origin https://example.com/repo");
        assert_eq!(lines[2], "git fetch --depth=1 origin sha1");
        assert_eq!(lines[3], "git checkout FETCH_HEAD");
    }

    #[test]
    fn boxed_step_clone() {
        let step: Box<dyn Step> = Box::new(UserStep::new("test", "echo hi"));
        let cloned = step.clone();
        assert_eq!(cloned.name(), "test");
        assert_eq!(cloned.command(), "echo hi");
        assert_eq!(cloned.kind(), StepKind::User);
    }

    #[test]
    fn boxed_step_serialization_roundtrip_user() {
        let step: Box<dyn Step> = Box::new(UserStep::new("test", "cargo test"));
        let json = serde_json::to_string(&step).unwrap();
        let deserialized: Box<dyn Step> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name(), "test");
        assert_eq!(deserialized.command(), "cargo test");
        assert_eq!(deserialized.kind(), StepKind::User);
    }

    #[test]
    fn boxed_step_serialization_roundtrip_system() {
        let step: Box<dyn Step> = Box::new(CloneStep::build(
            "https://example.com/repo",
            Some("abc123"),
            None,
        ));
        let json = serde_json::to_string(&step).unwrap();
        let deserialized: Box<dyn Step> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name(), "Clone repository into workspace");
        assert_eq!(deserialized.kind(), StepKind::System);
        // The command is the joined form; individual commands are reconstructed from lines
        assert!(deserialized.command().contains("git init"));
        assert!(deserialized.command().contains("git checkout FETCH_HEAD"));
    }

    #[test]
    fn clone_opts_default() {
        let opts = CloneOpts::default();
        assert_eq!(opts.depth, 0);
        assert!(!opts.skip);
        assert!(!opts.submodules);
    }

    #[test]
    fn clone_step_default_depth_is_1() {
        // When depth is 0 (default), the effective depth should be 1 (shallow clone)
        let opts = CloneOpts {
            depth: 0,
            ..Default::default()
        };
        let step = CloneStep::build("https://example.com/repo", Some("sha"), Some(&opts));
        assert!(step.commands[2].contains("--depth=1"));
    }
}
