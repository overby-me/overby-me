//! Pipeline environment variable builder.
//!
//! Extracts environment variables from pipeline trigger metadata and injects
//! them into workflow steps. Matches the upstream Go `PipelineEnvVars` function
//! and associated types from `pipeline_env.go`.
//!
//! These are framework-provided variables (prefixed with `TANGLED_`) that give
//! workflow steps information about the trigger event, repository, ref, etc.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::pipeline::PipelineId;

/// The kind of event that triggered the pipeline.
///
/// Matches the upstream Go `TriggerKind` type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    Push,
    PullRequest,
    Manual,
}

impl TriggerKind {
    /// Returns the string representation matching the upstream Go format.
    pub fn as_str(self) -> &'static str {
        match self {
            TriggerKind::Push => "push",
            TriggerKind::PullRequest => "pull_request",
            TriggerKind::Manual => "manual",
        }
    }
}

impl std::fmt::Display for TriggerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Repository information from the trigger event.
///
/// Matches the upstream Go `Pipeline_TriggerRepo` struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerRepo {
    /// The knot server hostname (e.g. `"example.com"`).
    pub knot: String,
    /// The DID of the repository owner (e.g. `"did:plc:user123"`).
    pub did: String,
    /// The repository name (e.g. `"my-repo"`).
    pub repo: String,
    /// The default branch of the repository (e.g. `"main"`).
    #[serde(default)]
    pub default_branch: String,
}

/// Push trigger data.
///
/// Matches the upstream Go `Pipeline_PushTriggerData` struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushTriggerData {
    /// The new commit SHA after the push.
    pub new_sha: String,
    /// The old commit SHA before the push.
    pub old_sha: String,
    /// The full ref name (e.g. `"refs/heads/main"` or `"refs/tags/v1.0.0"`).
    #[serde(rename = "ref")]
    pub ref_name: String,
}

/// Pull request trigger data.
///
/// Matches the upstream Go `Pipeline_PullRequestTriggerData` struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequestTriggerData {
    /// The source branch of the pull request.
    pub source_branch: String,
    /// The target branch of the pull request.
    pub target_branch: String,
    /// The commit SHA of the source branch head.
    pub source_sha: String,
    /// The action that triggered the event (e.g. `"opened"`, `"synchronize"`).
    pub action: String,
}

/// Manual trigger data.
///
/// Matches the upstream Go `Pipeline_ManualTriggerData` struct.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ManualTriggerData {
    /// Key-value input pairs provided by the user.
    #[serde(default)]
    pub inputs: Vec<KeyValuePair>,
}

/// A simple key-value pair.
///
/// Matches the upstream Go `Pipeline_Pair` struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyValuePair {
    pub key: String,
    pub value: String,
}

/// Full trigger metadata for a pipeline event.
///
/// Matches the upstream Go `Pipeline_TriggerMetadata` struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerMetadata {
    /// The kind of trigger event.
    pub kind: String,
    /// Repository information (always present).
    #[serde(default)]
    pub repo: Option<TriggerRepo>,
    /// Push-specific data (present when `kind == "push"`).
    #[serde(default)]
    pub push: Option<PushTriggerData>,
    /// Pull request-specific data (present when `kind == "pull_request"`).
    #[serde(default)]
    pub pull_request: Option<PullRequestTriggerData>,
    /// Manual trigger data (present when `kind == "manual"`).
    #[serde(default)]
    pub manual: Option<ManualTriggerData>,
}

/// Build the repository URL from trigger repo metadata.
///
/// Matches the upstream Go `BuildRepoURL` function:
/// - In dev mode, uses `http://` and replaces `localhost` with `host.docker.internal`
///   (for Docker networking compatibility with the upstream Go spindle).
/// - In production mode, uses `https://`.
///
/// Format: `{scheme}{host}/{did}/{repo}`
pub fn build_repo_url(repo: &TriggerRepo, dev_mode: bool) -> String {
    let scheme = if dev_mode { "http://" } else { "https://" };

    let host = if dev_mode {
        repo.knot.replace("localhost", "host.docker.internal")
    } else {
        repo.knot.clone()
    };

    format!("{scheme}{host}/{}/{}", repo.did, repo.repo)
}

/// Extract the short ref name from a full git reference.
///
/// - `refs/heads/main` → `main`
/// - `refs/tags/v1.0.0` → `v1.0.0`
/// - `refs/heads/feature/foo` → `feature/foo`
fn ref_short_name(full_ref: &str) -> &str {
    if let Some(name) = full_ref.strip_prefix("refs/heads/") {
        name
    } else if let Some(name) = full_ref.strip_prefix("refs/tags/") {
        name
    } else if let Some(name) = full_ref.strip_prefix("refs/") {
        name
    } else {
        full_ref
    }
}

/// Determine the ref type from a full git reference.
///
/// Returns `"tag"` for `refs/tags/*`, `"branch"` for everything else.
fn ref_type(full_ref: &str) -> &'static str {
    if full_ref.starts_with("refs/tags/") {
        "tag"
    } else {
        "branch"
    }
}

/// Builder for pipeline environment variables.
///
/// Extracts environment variables from pipeline trigger metadata and returns
/// them as a `HashMap<String, String>`. These variables are injected into every
/// workflow step's environment.
///
/// Matches the upstream Go `PipelineEnvVars` function from `pipeline_env.go`.
///
/// # Variables produced
///
/// Always set:
/// - `CI=true`
/// - `TANGLED_PIPELINE_ID` — AT Protocol URI of the pipeline record
///
/// From repo metadata (when present):
/// - `TANGLED_REPO_KNOT` — knot server hostname
/// - `TANGLED_REPO_DID` — repo owner DID
/// - `TANGLED_REPO_NAME` — repo name
/// - `TANGLED_REPO_DEFAULT_BRANCH` — default branch
/// - `TANGLED_REPO_URL` — full clone URL
///
/// For push triggers:
/// - `TANGLED_REF` — full ref (e.g. `refs/heads/main`)
/// - `TANGLED_REF_NAME` — short ref name (e.g. `main`)
/// - `TANGLED_REF_TYPE` — `"branch"` or `"tag"`
/// - `TANGLED_SHA` / `TANGLED_COMMIT_SHA` — the new commit SHA
///
/// For pull request triggers:
/// - `TANGLED_REF` — `refs/heads/{source_branch}`
/// - `TANGLED_REF_NAME` — source branch name
/// - `TANGLED_REF_TYPE` — `"branch"`
/// - `TANGLED_SHA` / `TANGLED_COMMIT_SHA` — source branch SHA
/// - `TANGLED_PR_SOURCE_BRANCH` / `TANGLED_PR_TARGET_BRANCH`
/// - `TANGLED_PR_SOURCE_SHA` / `TANGLED_PR_ACTION`
///
/// For manual triggers:
/// - `TANGLED_INPUT_{KEY}` — one entry per input, key uppercased
pub struct PipelineEnvVars;

impl PipelineEnvVars {
    /// Build pipeline environment variables from trigger metadata.
    ///
    /// Returns `None` if `trigger` is `None`, matching the upstream Go behavior.
    pub fn build(
        trigger: Option<&TriggerMetadata>,
        pipeline_id: &PipelineId,
        dev_mode: bool,
    ) -> Option<HashMap<String, String>> {
        let tr = trigger?;

        let mut env = HashMap::new();

        // Standard CI environment variable
        env.insert("CI".into(), "true".into());

        // Pipeline identifier
        env.insert("TANGLED_PIPELINE_ID".into(), pipeline_id.at_uri());

        // Repo info
        if let Some(repo) = &tr.repo {
            env.insert("TANGLED_REPO_KNOT".into(), repo.knot.clone());
            env.insert("TANGLED_REPO_DID".into(), repo.did.clone());
            env.insert("TANGLED_REPO_NAME".into(), repo.repo.clone());
            env.insert(
                "TANGLED_REPO_DEFAULT_BRANCH".into(),
                repo.default_branch.clone(),
            );
            env.insert("TANGLED_REPO_URL".into(), build_repo_url(repo, dev_mode));
        }

        // Trigger-kind-specific variables
        match tr.kind.as_str() {
            "push" => {
                if let Some(push) = &tr.push {
                    let short = ref_short_name(&push.ref_name);
                    let rtype = ref_type(&push.ref_name);

                    env.insert("TANGLED_REF".into(), push.ref_name.clone());
                    env.insert("TANGLED_REF_NAME".into(), short.to_owned());
                    env.insert("TANGLED_REF_TYPE".into(), rtype.to_owned());
                    env.insert("TANGLED_SHA".into(), push.new_sha.clone());
                    env.insert("TANGLED_COMMIT_SHA".into(), push.new_sha.clone());
                }
            }
            "pull_request" => {
                if let Some(pr) = &tr.pull_request {
                    env.insert(
                        "TANGLED_REF".into(),
                        format!("refs/heads/{}", pr.source_branch),
                    );
                    env.insert("TANGLED_REF_NAME".into(), pr.source_branch.clone());
                    env.insert("TANGLED_REF_TYPE".into(), "branch".into());
                    env.insert("TANGLED_SHA".into(), pr.source_sha.clone());
                    env.insert("TANGLED_COMMIT_SHA".into(), pr.source_sha.clone());

                    // PR-specific variables
                    env.insert("TANGLED_PR_SOURCE_BRANCH".into(), pr.source_branch.clone());
                    env.insert("TANGLED_PR_TARGET_BRANCH".into(), pr.target_branch.clone());
                    env.insert("TANGLED_PR_SOURCE_SHA".into(), pr.source_sha.clone());
                    env.insert("TANGLED_PR_ACTION".into(), pr.action.clone());
                }
            }
            "manual" => {
                if let Some(manual) = &tr.manual {
                    for pair in &manual.inputs {
                        let key = format!("TANGLED_INPUT_{}", pair.key.to_uppercase());
                        env.insert(key, pair.value.clone());
                    }
                }
            }
            _ => {
                // Unknown trigger kind — no additional variables
            }
        }

        Some(env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::PipelineId;

    fn test_pipeline_id() -> PipelineId {
        PipelineId {
            knot: "example.com".into(),
            rkey: "123123".into(),
        }
    }

    #[test]
    fn push_branch() {
        let tr = TriggerMetadata {
            kind: "push".into(),
            push: Some(PushTriggerData {
                new_sha: "abc123def456".into(),
                old_sha: "000000000000".into(),
                ref_name: "refs/heads/main".into(),
            }),
            repo: Some(TriggerRepo {
                knot: "example.com".into(),
                did: "did:plc:user123".into(),
                repo: "my-repo".into(),
                default_branch: "main".into(),
            }),
            pull_request: None,
            manual: None,
        };
        let id = test_pipeline_id();
        let env = PipelineEnvVars::build(Some(&tr), &id, false).unwrap();

        assert_eq!(env["CI"], "true");
        assert_eq!(
            env["TANGLED_PIPELINE_ID"],
            "at://did:web:example.com/sh.tangled.pipeline/123123"
        );
        assert_eq!(env["TANGLED_REF"], "refs/heads/main");
        assert_eq!(env["TANGLED_REF_NAME"], "main");
        assert_eq!(env["TANGLED_REF_TYPE"], "branch");
        assert_eq!(env["TANGLED_SHA"], "abc123def456");
        assert_eq!(env["TANGLED_COMMIT_SHA"], "abc123def456");
        assert_eq!(env["TANGLED_REPO_KNOT"], "example.com");
        assert_eq!(env["TANGLED_REPO_DID"], "did:plc:user123");
        assert_eq!(env["TANGLED_REPO_NAME"], "my-repo");
        assert_eq!(env["TANGLED_REPO_DEFAULT_BRANCH"], "main");
        assert_eq!(
            env["TANGLED_REPO_URL"],
            "https://example.com/did:plc:user123/my-repo"
        );
    }

    #[test]
    fn push_tag() {
        let tr = TriggerMetadata {
            kind: "push".into(),
            push: Some(PushTriggerData {
                new_sha: "abc123def456".into(),
                old_sha: "000000000000".into(),
                ref_name: "refs/tags/v1.2.3".into(),
            }),
            repo: Some(TriggerRepo {
                knot: "example.com".into(),
                did: "did:plc:user123".into(),
                repo: "my-repo".into(),
                default_branch: String::new(),
            }),
            pull_request: None,
            manual: None,
        };
        let id = test_pipeline_id();
        let env = PipelineEnvVars::build(Some(&tr), &id, false).unwrap();

        assert_eq!(env["TANGLED_REF"], "refs/tags/v1.2.3");
        assert_eq!(env["TANGLED_REF_NAME"], "v1.2.3");
        assert_eq!(env["TANGLED_REF_TYPE"], "tag");
    }

    #[test]
    fn pull_request() {
        let tr = TriggerMetadata {
            kind: "pull_request".into(),
            pull_request: Some(PullRequestTriggerData {
                source_branch: "feature-branch".into(),
                target_branch: "main".into(),
                source_sha: "pr-sha-789".into(),
                action: "opened".into(),
            }),
            repo: Some(TriggerRepo {
                knot: "example.com".into(),
                did: "did:plc:user123".into(),
                repo: "my-repo".into(),
                default_branch: String::new(),
            }),
            push: None,
            manual: None,
        };
        let id = test_pipeline_id();
        let env = PipelineEnvVars::build(Some(&tr), &id, false).unwrap();

        assert_eq!(env["TANGLED_REF"], "refs/heads/feature-branch");
        assert_eq!(env["TANGLED_REF_NAME"], "feature-branch");
        assert_eq!(env["TANGLED_REF_TYPE"], "branch");
        assert_eq!(env["TANGLED_SHA"], "pr-sha-789");
        assert_eq!(env["TANGLED_COMMIT_SHA"], "pr-sha-789");
        assert_eq!(env["TANGLED_PR_SOURCE_BRANCH"], "feature-branch");
        assert_eq!(env["TANGLED_PR_TARGET_BRANCH"], "main");
        assert_eq!(env["TANGLED_PR_SOURCE_SHA"], "pr-sha-789");
        assert_eq!(env["TANGLED_PR_ACTION"], "opened");
    }

    #[test]
    fn manual_with_inputs() {
        let tr = TriggerMetadata {
            kind: "manual".into(),
            manual: Some(ManualTriggerData {
                inputs: vec![
                    KeyValuePair {
                        key: "version".into(),
                        value: "1.0.0".into(),
                    },
                    KeyValuePair {
                        key: "environment".into(),
                        value: "production".into(),
                    },
                ],
            }),
            repo: Some(TriggerRepo {
                knot: "example.com".into(),
                did: "did:plc:user123".into(),
                repo: "my-repo".into(),
                default_branch: String::new(),
            }),
            push: None,
            pull_request: None,
        };
        let id = test_pipeline_id();
        let env = PipelineEnvVars::build(Some(&tr), &id, false).unwrap();

        assert_eq!(env["TANGLED_INPUT_VERSION"], "1.0.0");
        assert_eq!(env["TANGLED_INPUT_ENVIRONMENT"], "production");

        // Manual triggers shouldn't have ref/sha variables
        assert!(!env.contains_key("TANGLED_REF"));
        assert!(!env.contains_key("TANGLED_SHA"));
    }

    #[test]
    fn dev_mode_repo_url() {
        let tr = TriggerMetadata {
            kind: "push".into(),
            push: Some(PushTriggerData {
                new_sha: "abc123".into(),
                old_sha: String::new(),
                ref_name: "refs/heads/main".into(),
            }),
            repo: Some(TriggerRepo {
                knot: "localhost:3000".into(),
                did: "did:plc:user123".into(),
                repo: "my-repo".into(),
                default_branch: String::new(),
            }),
            pull_request: None,
            manual: None,
        };
        let id = test_pipeline_id();
        let env = PipelineEnvVars::build(Some(&tr), &id, true).unwrap();

        assert_eq!(
            env["TANGLED_REPO_URL"],
            "http://host.docker.internal:3000/did:plc:user123/my-repo"
        );
    }

    #[test]
    fn nil_trigger_returns_none() {
        let id = test_pipeline_id();
        let env = PipelineEnvVars::build(None, &id, false);
        assert!(env.is_none());
    }

    #[test]
    fn nil_push_data() {
        let tr = TriggerMetadata {
            kind: "push".into(),
            push: None,
            repo: Some(TriggerRepo {
                knot: "example.com".into(),
                did: "did:plc:user123".into(),
                repo: "my-repo".into(),
                default_branch: String::new(),
            }),
            pull_request: None,
            manual: None,
        };
        let id = test_pipeline_id();
        let env = PipelineEnvVars::build(Some(&tr), &id, false).unwrap();

        // Should still have repo variables
        assert_eq!(env["TANGLED_REPO_KNOT"], "example.com");

        // Should not have ref/sha variables when push data is nil
        assert!(!env.contains_key("TANGLED_REF"));
        assert!(!env.contains_key("TANGLED_SHA"));
    }

    #[test]
    fn build_repo_url_production() {
        let repo = TriggerRepo {
            knot: "example.com".into(),
            did: "did:plc:user123".into(),
            repo: "my-repo".into(),
            default_branch: String::new(),
        };
        assert_eq!(
            build_repo_url(&repo, false),
            "https://example.com/did:plc:user123/my-repo"
        );
    }

    #[test]
    fn build_repo_url_dev_mode_localhost() {
        let repo = TriggerRepo {
            knot: "localhost:3000".into(),
            did: "did:plc:user123".into(),
            repo: "my-repo".into(),
            default_branch: String::new(),
        };
        assert_eq!(
            build_repo_url(&repo, true),
            "http://host.docker.internal:3000/did:plc:user123/my-repo"
        );
    }

    #[test]
    fn build_repo_url_dev_mode_no_localhost() {
        let repo = TriggerRepo {
            knot: "example.com".into(),
            did: "did:plc:user123".into(),
            repo: "my-repo".into(),
            default_branch: String::new(),
        };
        // Dev mode but no localhost — should use http:// but not replace hostname
        assert_eq!(
            build_repo_url(&repo, true),
            "http://example.com/did:plc:user123/my-repo"
        );
    }

    #[test]
    fn ref_short_name_branch() {
        assert_eq!(ref_short_name("refs/heads/main"), "main");
        assert_eq!(
            ref_short_name("refs/heads/feature/my-feature"),
            "feature/my-feature"
        );
    }

    #[test]
    fn ref_short_name_tag() {
        assert_eq!(ref_short_name("refs/tags/v1.0.0"), "v1.0.0");
        assert_eq!(ref_short_name("refs/tags/release/2024"), "release/2024");
    }

    #[test]
    fn ref_short_name_other() {
        assert_eq!(
            ref_short_name("refs/remotes/origin/main"),
            "remotes/origin/main"
        );
        assert_eq!(ref_short_name("main"), "main");
    }

    #[test]
    fn ref_type_detection() {
        assert_eq!(ref_type("refs/heads/main"), "branch");
        assert_eq!(ref_type("refs/tags/v1.0.0"), "tag");
        assert_eq!(ref_type("refs/other/thing"), "branch");
    }

    #[test]
    fn trigger_kind_display() {
        assert_eq!(TriggerKind::Push.to_string(), "push");
        assert_eq!(TriggerKind::PullRequest.to_string(), "pull_request");
        assert_eq!(TriggerKind::Manual.to_string(), "manual");
    }

    #[test]
    fn trigger_kind_serialization_roundtrip() {
        for kind in [
            TriggerKind::Push,
            TriggerKind::PullRequest,
            TriggerKind::Manual,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            let deserialized: TriggerKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, deserialized);
        }
    }

    #[test]
    fn trigger_metadata_serialization_roundtrip() {
        let tr = TriggerMetadata {
            kind: "push".into(),
            push: Some(PushTriggerData {
                new_sha: "abc123".into(),
                old_sha: "000000".into(),
                ref_name: "refs/heads/main".into(),
            }),
            repo: Some(TriggerRepo {
                knot: "example.com".into(),
                did: "did:plc:user123".into(),
                repo: "my-repo".into(),
                default_branch: "main".into(),
            }),
            pull_request: None,
            manual: None,
        };
        let json = serde_json::to_string(&tr).unwrap();
        let deserialized: TriggerMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(tr, deserialized);
    }

    #[test]
    fn always_has_ci_true() {
        let tr = TriggerMetadata {
            kind: "push".into(),
            push: None,
            repo: None,
            pull_request: None,
            manual: None,
        };
        let id = test_pipeline_id();
        let env = PipelineEnvVars::build(Some(&tr), &id, false).unwrap();
        assert_eq!(env["CI"], "true");
    }

    #[test]
    fn always_has_pipeline_id() {
        let tr = TriggerMetadata {
            kind: "push".into(),
            push: None,
            repo: None,
            pull_request: None,
            manual: None,
        };
        let id = test_pipeline_id();
        let env = PipelineEnvVars::build(Some(&tr), &id, false).unwrap();
        assert_eq!(
            env["TANGLED_PIPELINE_ID"],
            "at://did:web:example.com/sh.tangled.pipeline/123123"
        );
    }

    #[test]
    fn unknown_trigger_kind_no_extra_vars() {
        let tr = TriggerMetadata {
            kind: "unknown".into(),
            push: None,
            repo: Some(TriggerRepo {
                knot: "example.com".into(),
                did: "did:plc:user123".into(),
                repo: "my-repo".into(),
                default_branch: String::new(),
            }),
            pull_request: None,
            manual: None,
        };
        let id = test_pipeline_id();
        let env = PipelineEnvVars::build(Some(&tr), &id, false).unwrap();

        // Should have CI and repo vars but no ref/sha/pr/manual vars
        assert_eq!(env["CI"], "true");
        assert_eq!(env["TANGLED_REPO_KNOT"], "example.com");
        assert!(!env.contains_key("TANGLED_REF"));
        assert!(!env.contains_key("TANGLED_SHA"));
        assert!(!env.contains_key("TANGLED_PR_SOURCE_BRANCH"));
    }
}
