//! The `sh.tangled.ci.*` spindle API.
//!
//! Replaces the older `sh.tangled.spindle.listRuns`, which no longer exists:
//! a spindle answers it with a bare 404. Pipelines are now the unit, each
//! carrying the workflows it ran, and log delivery is a WebSocket
//! subscription rather than a query.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::client::TangledClient;

pub const QUERY_PIPELINES: &str = "sh.tangled.ci.queryPipelines";
pub const GET_PIPELINE: &str = "sh.tangled.ci.getPipeline";
pub const TRIGGER_PIPELINE: &str = "sh.tangled.ci.triggerPipeline";
pub const CANCEL_PIPELINE: &str = "sh.tangled.ci.cancelPipeline";
pub const SUBSCRIBE_PIPELINE_LOGS: &str = "sh.tangled.ci.subscribePipelineLogs";

/// One workflow executed by a pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(rename = "startedAt", default)]
    pub started_at: Option<String>,
    #[serde(rename = "finishedAt", default)]
    pub finished_at: Option<String>,
}

/// A CI pipeline as a spindle reports it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pipeline {
    pub id: String,
    #[serde(default)]
    pub commit: String,
    #[serde(rename = "createdAt", default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(rename = "sourceRepo", default)]
    pub source_repo: Option<String>,
    /// Open union (`#push`, `#manual`, ...); kept raw so an unknown trigger
    /// kind does not fail the whole response.
    #[serde(default)]
    pub trigger: serde_json::Value,
    #[serde(default)]
    pub workflows: Vec<Workflow>,
}

impl Pipeline {
    /// The branch or tag ref this pipeline ran for, if the trigger names one.
    pub fn trigger_ref(&self) -> Option<&str> {
        self.trigger.get("ref").and_then(|r| r.as_str())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct QueryPipelines {
    #[serde(default)]
    pub pipelines: Vec<Pipeline>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub total: i64,
}

impl TangledClient {
    /// Pipelines for a repo, newest first. `repo` is the repo's own DID, not
    /// its at-uri. Reading is public, so no token is needed.
    pub async fn query_pipelines(
        &self,
        repo_did: &str,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<QueryPipelines> {
        let mut params = vec![("repo", repo_did.to_string()), ("limit", limit.to_string())];
        if let Some(c) = cursor {
            params.push(("cursor", c.to_string()));
        }
        self.get_json(QUERY_PIPELINES, &params, None).await
    }

    pub async fn get_pipeline(&self, pipeline_id: &str) -> Result<Pipeline> {
        let params = [("pipeline", pipeline_id.to_string())];
        self.get_json(GET_PIPELINE, &params, None).await
    }

    /// Start a pipeline by hand. Returns the new pipeline's id.
    pub async fn trigger_pipeline(
        &self,
        pds_base: &str,
        access_jwt: &str,
        repo_did: &str,
        sha: &str,
        git_ref: Option<&str>,
        workflows: &[String],
    ) -> Result<String> {
        #[derive(Serialize)]
        struct Trigger<'a> {
            #[serde(rename = "$type")]
            lexicon_type: &'a str,
            sha: &'a str,
            #[serde(rename = "ref", skip_serializing_if = "Option::is_none")]
            git_ref: Option<&'a str>,
        }
        #[derive(Serialize)]
        struct Req<'a> {
            repo: &'a str,
            trigger: Trigger<'a>,
            #[serde(skip_serializing_if = "<[String]>::is_empty")]
            workflows: &'a [String],
        }
        #[derive(Deserialize)]
        struct Res {
            pipeline: String,
        }
        let sa = self
            .spindle_auth(pds_base, access_jwt, TRIGGER_PIPELINE)
            .await?;
        let req = Req {
            repo: repo_did,
            trigger: Trigger {
                lexicon_type: "sh.tangled.ci.trigger#manual",
                sha,
                git_ref,
            },
            workflows,
        };
        let res: Res = self
            .post_json_pub(TRIGGER_PIPELINE, &req, Some(&sa))
            .await?;
        Ok(res.pipeline)
    }

    /// Cancel a running pipeline, or just some of its workflows.
    pub async fn cancel_pipeline(
        &self,
        pds_base: &str,
        access_jwt: &str,
        pipeline_id: &str,
        repo_did: &str,
        workflows: &[String],
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Req<'a> {
            pipeline: &'a str,
            repo: &'a str,
            #[serde(skip_serializing_if = "<[String]>::is_empty")]
            workflows: &'a [String],
        }
        let sa = self
            .spindle_auth(pds_base, access_jwt, CANCEL_PIPELINE)
            .await?;
        let req = Req {
            pipeline: pipeline_id,
            repo: repo_did,
            workflows,
        };
        let _: serde_json::Value = self.post_json_pub(CANCEL_PIPELINE, &req, Some(&sa)).await?;
        Ok(())
    }

    /// The `wss://` URL for a pipeline's log subscription.
    pub fn pipeline_logs_url(&self, pipeline_id: &str, workflows: &[String]) -> Result<String> {
        let mut url = url::Url::parse(&self.base_url_with_scheme())?;
        match url.scheme() {
            "https" => url
                .set_scheme("wss")
                .map_err(|_| anyhow!("cannot use wss for this host"))?,
            "http" => url
                .set_scheme("ws")
                .map_err(|_| anyhow!("cannot use ws for this host"))?,
            other => return Err(anyhow!("spindle host must be http(s), got {other}")),
        }
        url.set_path(&format!("/xrpc/{SUBSCRIBE_PIPELINE_LOGS}"));
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("pipeline", pipeline_id);
            for w in workflows {
                q.append_pair("workflows", w);
            }
        }
        Ok(url.to_string())
    }
}
