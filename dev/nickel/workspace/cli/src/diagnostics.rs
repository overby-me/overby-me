#![allow(dead_code)]
//! Structured diagnostics for nix-workspace.
//!
//! Implements the NW diagnostic format from SPEC.md:
//! - NW0xx — Contract violations (type/value errors)
//! - NW1xx — Discovery errors (missing files, bad directory structure)
//! - NW2xx — Namespace conflicts (duplicate names, invalid derivation names)
//! - NW3xx — Module errors (missing dependencies, circular imports)
//! - NW4xx — System errors (unsupported system, missing input)

use owo_colors::OwoColorize;
use serde::{Deserialize, Serialize};
use std::fmt;

// ── Severity ──────────────────────────────────────────────────────

/// Diagnostic severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
    Hint,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Error => write!(f, "error"),
            Self::Warning => write!(f, "warning"),
            Self::Info => write!(f, "info"),
            Self::Hint => write!(f, "hint"),
        }
    }
}

impl Severity {
    /// Format the severity with terminal colors.
    pub fn colored(&self) -> String {
        match self {
            Self::Error => "error".red().bold().to_string(),
            Self::Warning => "warning".yellow().bold().to_string(),
            Self::Info => "info".blue().bold().to_string(),
            Self::Hint => "hint".cyan().bold().to_string(),
        }
    }
}

// ── Diagnostic context ────────────────────────────────────────────

/// Additional context about where the diagnostic originated.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiagnosticContext {
    /// Workspace name (if known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,

    /// Flake output path (e.g., `nixosConfigurations.gravitas`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

// ── Single diagnostic ─────────────────────────────────────────────

/// A single structured diagnostic following the NW diagnostic schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    /// NW diagnostic code (e.g., `"NW001"`).
    pub code: String,

    /// Severity level.
    pub severity: Severity,

    /// Source file (relative to workspace root).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,

    /// Line number (1-based).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,

    /// Column number (1-based).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,

    /// Field path that failed validation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,

    /// Human-readable diagnostic message.
    pub message: String,

    /// Optional hint for how to fix the issue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,

    /// Contract name that was violated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,

    /// Additional context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<DiagnosticContext>,
}

impl Diagnostic {
    /// Create a new diagnostic with the given code, severity, and message.
    pub fn new(code: impl Into<String>, severity: Severity, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity,
            file: None,
            line: None,
            column: None,
            field: None,
            message: message.into(),
            hint: None,
            contract: None,
            context: None,
        }
    }

    /// Create an error diagnostic.
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Error, message)
    }

    /// Create a warning diagnostic.
    pub fn warning(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Warning, message)
    }

    /// Create an info diagnostic.
    pub fn info(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(code, Severity::Info, message)
    }

    pub fn with_file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    pub fn with_line(mut self, line: u32) -> Self {
        self.line = Some(line);
        self
    }

    pub fn with_column(mut self, column: u32) -> Self {
        self.column = Some(column);
        self
    }

    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_contract(mut self, contract: impl Into<String>) -> Self {
        self.contract = Some(contract.into());
        self
    }

    pub fn with_context(mut self, context: DiagnosticContext) -> Self {
        self.context = Some(context);
        self
    }

    /// Format the diagnostic for human-readable terminal output.
    ///
    /// Example:
    /// ```text
    /// error[NW001]: Expected System, got "x86-linux"
    ///   --> machines/gravitas.ncl:3:13
    ///   = field: system
    ///   = contract: MachineConfig.system
    ///   = hint: did you mean "x86_64-linux"?
    /// ```
    pub fn format_human(&self) -> String {
        let mut out = String::new();

        // Header: severity[CODE]: message
        out.push_str(&format!(
            "{}[{}]: {}\n",
            self.severity.colored(),
            self.code.bold(),
            self.message
        ));

        // Location
        if let Some(ref file) = self.file {
            out.push_str(&format!("  {} {}", "-->".blue().bold(), file.bold()));
            if let Some(line) = self.line {
                out.push_str(&format!(":{line}"));
                if let Some(col) = self.column {
                    out.push_str(&format!(":{col}"));
                }
            }
            out.push('\n');
        }

        // Field
        if let Some(ref field) = self.field {
            out.push_str(&format!(
                "  {} {}: {field}\n",
                "=".blue().bold(),
                "field".dimmed()
            ));
        }

        // Contract
        if let Some(ref contract) = self.contract {
            out.push_str(&format!(
                "  {} {}: {contract}\n",
                "=".blue().bold(),
                "contract".dimmed()
            ));
        }

        // Context
        if let Some(ref ctx) = self.context {
            if let Some(ref ws) = ctx.workspace {
                out.push_str(&format!(
                    "  {} {}: {ws}\n",
                    "=".blue().bold(),
                    "workspace".dimmed()
                ));
            }
            if let Some(ref output) = ctx.output {
                out.push_str(&format!(
                    "  {} {}: {output}\n",
                    "=".blue().bold(),
                    "output".dimmed()
                ));
            }
        }

        // Hint (last, so it stands out)
        if let Some(ref hint) = self.hint {
            out.push_str(&format!(
                "  {} {}: {hint}\n",
                "=".blue().bold(),
                "hint".green()
            ));
        }

        out
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format_human())
    }
}

// ── Diagnostic report ─────────────────────────────────────────────

/// A collection of diagnostics, serializable as the SPEC.md JSON format.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiagnosticReport {
    pub diagnostics: Vec<Diagnostic>,
}

impl DiagnosticReport {
    pub fn new() -> Self {
        Self {
            diagnostics: Vec::new(),
        }
    }

    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub fn is_empty(&self) -> bool {
        self.diagnostics.is_empty()
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .count()
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Warning)
            .count()
    }

    /// Format the entire report as JSON.
    pub fn format_json(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Format the entire report for human-readable terminal output.
    pub fn format_human(&self) -> String {
        let mut out = String::new();

        for diag in &self.diagnostics {
            out.push_str(&diag.format_human());
            out.push('\n');
        }

        // Summary line
        let errors = self.error_count();
        let warnings = self.warning_count();
        if errors > 0 || warnings > 0 {
            let parts: Vec<String> = [
                (errors > 0).then(|| {
                    format!(
                        "{} {}",
                        errors,
                        if errors == 1 { "error" } else { "errors" }
                    )
                    .red()
                    .bold()
                    .to_string()
                }),
                (warnings > 0).then(|| {
                    format!(
                        "{} {}",
                        warnings,
                        if warnings == 1 { "warning" } else { "warnings" }
                    )
                    .yellow()
                    .bold()
                    .to_string()
                }),
            ]
            .into_iter()
            .flatten()
            .collect();

            out.push_str(&format!("{} generated\n", parts.join(", ")));
        }

        out
    }
}

// ── Output format ─────────────────────────────────────────────────

/// The output format for diagnostics (set via `--format`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// Human-readable colored terminal output.
    #[default]
    Human,
    /// JSON following the SPEC.md structured diagnostics schema.
    Json,
}

// ── Well-known diagnostic codes ───────────────────────────────────

/// Well-known NW diagnostic codes.
///
/// These correspond to the code ranges in SPEC.md:
/// - NW0xx — Contract violations
/// - NW1xx — Discovery errors
/// - NW2xx — Namespace conflicts
/// - NW3xx — Module errors
/// - NW4xx — System/plugin errors
pub mod codes {
    // Contract violations (NW0xx)
    pub const CONTRACT_VIOLATION: &str = "NW001";
    pub const INVALID_TYPE: &str = "NW002";
    pub const INVALID_VALUE: &str = "NW003";

    // Discovery errors (NW1xx)
    pub const MISSING_WORKSPACE_NCL: &str = "NW100";
    pub const MISSING_DIRECTORY: &str = "NW101";
    pub const INVALID_NCL_FILE: &str = "NW102";
    pub const DISCOVERY_ERROR: &str = "NW103";

    // Namespace conflicts (NW2xx)
    pub const NAMESPACE_CONFLICT: &str = "NW200";
    pub const INVALID_NAME: &str = "NW201";

    // Module errors (NW3xx)
    pub const MISSING_DEPENDENCY: &str = "NW300";
    pub const CIRCULAR_IMPORT: &str = "NW301";

    // System/plugin errors (NW4xx)
    pub const UNSUPPORTED_SYSTEM: &str = "NW400";
    pub const MISSING_INPUT: &str = "NW401";
    pub const PLUGIN_ERROR: &str = "NW402";

    // CLI-specific (NW5xx)
    pub const MISSING_TOOL: &str = "NW500";
    pub const TOOL_FAILED: &str = "NW501";
    pub const FLAKE_GENERATION_FAILED: &str = "NW502";
}

// ── Nickel error parsing ──────────────────────────────────────────

/// Attempt to parse a Nickel error message into a structured diagnostic.
///
/// Nickel errors have a recognizable format with contract names and locations.
/// This function does a best-effort parse; if parsing fails, it wraps the
/// raw error text into a generic NW001 diagnostic.
pub fn parse_nickel_error(stderr: &str) -> DiagnosticReport {
    let mut report = DiagnosticReport::new();

    if stderr.trim().is_empty() {
        return report;
    }

    // Try to extract structured information from Nickel's error output.
    //
    // Nickel errors typically look like:
    //   error: contract broken by a value
    //     ┌─ <source>:LINE:COL
    //     │
    //   N │   some code
    //     │   ^^^^^^^^^ this value
    //     │
    //     = expected: ...
    //     = ...
    //
    // We do a line-by-line best-effort parse.

    let mut current_message: Option<String> = None;
    let mut current_file: Option<String> = None;
    let mut current_line: Option<u32> = None;
    let mut current_column: Option<u32> = None;
    let mut current_hint: Option<String> = None;
    let mut current_contract: Option<String> = None;
    let mut details: Vec<String> = Vec::new();

    for line in stderr.lines() {
        let trimmed = line.trim();

        // "error: ..." — start of a new error block
        if let Some(msg) = trimmed.strip_prefix("error:") {
            // Flush previous diagnostic if any
            if let Some(ref msg_text) = current_message {
                let full_message = if details.is_empty() {
                    msg_text.clone()
                } else {
                    format!("{}\n{}", msg_text, details.join("\n"))
                };
                let mut diag = Diagnostic::error(codes::CONTRACT_VIOLATION, full_message);
                if let Some(ref f) = current_file {
                    diag = diag.with_file(f);
                }
                if let Some(l) = current_line {
                    diag = diag.with_line(l);
                }
                if let Some(c) = current_column {
                    diag = diag.with_column(c);
                }
                if let Some(ref h) = current_hint {
                    diag = diag.with_hint(h);
                }
                if let Some(ref ct) = current_contract {
                    diag = diag.with_contract(ct);
                }
                report.push(diag);
            }

            current_message = Some(msg.trim().to_string());
            current_file = None;
            current_line = None;
            current_column = None;
            current_hint = None;
            current_contract = None;
            details.clear();
            continue;
        }

        // "┌─ <file>:LINE:COL" — location line
        if trimmed.contains("┌─") || trimmed.contains("-->") {
            if let Some(loc_part) = trimmed.split("┌─").nth(1).or(trimmed.split("-->").nth(1)) {
                let loc = loc_part.trim();
                let parts: Vec<&str> = loc.rsplitn(3, ':').collect();
                match parts.len() {
                    3 => {
                        current_column = parts[0].parse().ok();
                        current_line = parts[1].parse().ok();
                        current_file = Some(parts[2].to_string());
                    }
                    2 => {
                        current_line = parts[0].parse().ok();
                        current_file = Some(parts[1].to_string());
                    }
                    _ => {
                        current_file = Some(loc.to_string());
                    }
                }
            }
            continue;
        }

        // "= expected: ..." or "= <key>: ..."
        if let Some(rest) = trimmed.strip_prefix("= ") {
            if let Some(expected) = rest.strip_prefix("expected:") {
                details.push(format!("expected: {}", expected.trim()));
            } else if let Some(hint_text) = rest.strip_prefix("hint:") {
                current_hint = Some(hint_text.trim().to_string());
            } else if let Some(contract_text) = rest.strip_prefix("contract:") {
                current_contract = Some(contract_text.trim().to_string());
            } else {
                details.push(rest.to_string());
            }
            continue;
        }
    }

    // Flush the last diagnostic
    if let Some(ref msg_text) = current_message {
        let full_message = if details.is_empty() {
            msg_text.clone()
        } else {
            format!("{}\n{}", msg_text, details.join("\n"))
        };
        let mut diag = Diagnostic::error(codes::CONTRACT_VIOLATION, full_message);
        if let Some(ref f) = current_file {
            diag = diag.with_file(f);
        }
        if let Some(l) = current_line {
            diag = diag.with_line(l);
        }
        if let Some(c) = current_column {
            diag = diag.with_column(c);
        }
        if let Some(ref h) = current_hint {
            diag = diag.with_hint(h);
        }
        if let Some(ref ct) = current_contract {
            diag = diag.with_contract(ct);
        }
        report.push(diag);
    }

    // If we couldn't parse anything meaningful, wrap the whole stderr as a generic error
    if report.is_empty() {
        report.push(Diagnostic::error(
            codes::CONTRACT_VIOLATION,
            stderr.trim().to_string(),
        ));
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_severity_display() {
        assert_eq!(Severity::Error.to_string(), "error");
        assert_eq!(Severity::Warning.to_string(), "warning");
        assert_eq!(Severity::Info.to_string(), "info");
        assert_eq!(Severity::Hint.to_string(), "hint");
    }

    #[test]
    fn test_diagnostic_builder() {
        let diag = Diagnostic::error("NW001", "something broke")
            .with_file("workspace.ncl")
            .with_line(3)
            .with_column(13)
            .with_field("system")
            .with_hint("did you mean x86_64-linux?")
            .with_contract("MachineConfig.system")
            .with_context(DiagnosticContext {
                workspace: Some("my-project".into()),
                output: Some("nixosConfigurations.gravitas".into()),
            });

        assert_eq!(diag.code, "NW001");
        assert_eq!(diag.severity, Severity::Error);
        assert_eq!(diag.file.as_deref(), Some("workspace.ncl"));
        assert_eq!(diag.line, Some(3));
        assert_eq!(diag.column, Some(13));
        assert_eq!(diag.field.as_deref(), Some("system"));
        assert_eq!(diag.hint.as_deref(), Some("did you mean x86_64-linux?"));
        assert_eq!(diag.contract.as_deref(), Some("MachineConfig.system"));
        let ctx = diag.context.as_ref().unwrap();
        assert_eq!(ctx.workspace.as_deref(), Some("my-project"));
        assert_eq!(ctx.output.as_deref(), Some("nixosConfigurations.gravitas"));
    }

    #[test]
    fn test_diagnostic_report_counts() {
        let mut report = DiagnosticReport::new();
        assert!(report.is_empty());
        assert!(!report.has_errors());
        assert_eq!(report.error_count(), 0);
        assert_eq!(report.warning_count(), 0);

        report.push(Diagnostic::error("NW001", "bad"));
        report.push(Diagnostic::warning("NW100", "iffy"));
        report.push(Diagnostic::error("NW002", "also bad"));
        report.push(Diagnostic::info("NW200", "fyi"));

        assert!(!report.is_empty());
        assert!(report.has_errors());
        assert_eq!(report.error_count(), 2);
        assert_eq!(report.warning_count(), 1);
    }

    #[test]
    fn test_diagnostic_report_json_roundtrip() {
        let mut report = DiagnosticReport::new();
        report.push(
            Diagnostic::error("NW001", "bad value")
                .with_file("workspace.ncl")
                .with_line(5),
        );

        let json = report.format_json().unwrap();
        let parsed: DiagnosticReport = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.diagnostics.len(), 1);
        assert_eq!(parsed.diagnostics[0].code, "NW001");
        assert_eq!(parsed.diagnostics[0].file.as_deref(), Some("workspace.ncl"));
        assert_eq!(parsed.diagnostics[0].line, Some(5));
    }

    #[test]
    fn test_parse_nickel_error_empty() {
        let report = parse_nickel_error("");
        assert!(report.is_empty());
    }

    #[test]
    fn test_parse_nickel_error_raw_text() {
        let report = parse_nickel_error("something unexpected happened");
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].code, codes::CONTRACT_VIOLATION);
        assert!(
            report.diagnostics[0]
                .message
                .contains("something unexpected happened")
        );
    }

    #[test]
    fn test_parse_nickel_error_structured() {
        let stderr = r#"error: contract broken by a value
  ┌─ workspace.ncl:3:13
  │
3 │   system = "x86-linux",
  │             ^^^^^^^^^^^ this value
  │
  = expected: System
  = hint: did you mean "x86_64-linux"?
"#;
        let report = parse_nickel_error(stderr);
        assert_eq!(report.diagnostics.len(), 1);
        let diag = &report.diagnostics[0];
        assert_eq!(diag.severity, Severity::Error);
        assert_eq!(diag.file.as_deref(), Some("workspace.ncl"));
        assert_eq!(diag.line, Some(3));
        assert_eq!(diag.column, Some(13));
        assert_eq!(diag.hint.as_deref(), Some("did you mean \"x86_64-linux\"?"));
    }
}
