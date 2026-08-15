//! SSH keys: `sh.tangled.publicKey`.
//!
//! A knot authorises an SSH push by matching the offered key against the keys
//! published by accounts allowed to write to that repo, so registering a key
//! is a prerequisite for pushing at all — including for any bot account used
//! by CI.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::client::TangledClient;

pub const PUBLIC_KEY: &str = "sh.tangled.publicKey";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicKey {
    /// Public key contents, in authorized_keys form.
    pub key: String,
    /// Human-readable name for this key.
    #[serde(default)]
    pub name: String,
    #[serde(rename = "createdAt", default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct PublicKeyRecord {
    pub rkey: String,
    pub key: PublicKey,
}

impl PublicKey {
    /// The key's type and comment, without the base64 body, for display.
    pub fn summary(&self) -> String {
        let mut parts = self.key.split_whitespace();
        let algo = parts.next().unwrap_or("?");
        let body = parts.next().unwrap_or("");
        let comment = parts.next().unwrap_or("");
        let tail: String = body
            .chars()
            .rev()
            .take(12)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{algo} ...{tail} {comment}").trim_end().to_string()
    }
}

impl TangledClient {
    /// Keys published by an account. Records are public, so no token is needed
    /// to read someone else's.
    pub async fn list_public_keys(
        &self,
        did: &str,
        bearer: Option<&str>,
    ) -> Result<Vec<PublicKeyRecord>> {
        #[derive(Deserialize)]
        struct Item {
            uri: String,
            value: PublicKey,
        }
        #[derive(Deserialize)]
        struct Res {
            #[serde(default)]
            records: Vec<Item>,
        }
        let params = vec![
            ("repo", did.to_string()),
            ("collection", PUBLIC_KEY.to_string()),
            ("limit", "100".to_string()),
        ];
        let res: Res = self
            .get_json("com.atproto.repo.listRecords", &params, bearer)
            .await?;
        Ok(res
            .records
            .into_iter()
            .map(|item| PublicKeyRecord {
                rkey: Self::uri_rkey(&item.uri).unwrap_or_default(),
                key: item.value,
            })
            .collect())
    }

    /// Publish a key. Returns its record key.
    pub async fn add_public_key(
        &self,
        did: &str,
        name: &str,
        key: &str,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<String> {
        #[derive(Serialize)]
        struct Rec<'a> {
            #[serde(rename = "$type")]
            lexicon_type: &'a str,
            key: &'a str,
            name: &'a str,
            #[serde(rename = "createdAt")]
            created_at: String,
        }
        #[derive(Serialize)]
        struct Req<'a> {
            repo: &'a str,
            collection: &'a str,
            validate: bool,
            record: Rec<'a>,
        }
        #[derive(Deserialize)]
        struct Res {
            uri: String,
        }
        let req = Req {
            repo: did,
            collection: PUBLIC_KEY,
            validate: false,
            record: Rec {
                lexicon_type: PUBLIC_KEY,
                key,
                name,
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        };
        let res: Res = self
            .derive(pds_base)
            .post_json_pub("com.atproto.repo.createRecord", &req, Some(access_jwt))
            .await?;
        Self::uri_rkey(&res.uri).ok_or_else(|| anyhow!("missing rkey in public key uri"))
    }

    pub async fn delete_public_key(
        &self,
        did: &str,
        rkey: &str,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Req<'a> {
            repo: &'a str,
            collection: &'a str,
            rkey: &'a str,
        }
        let req = Req {
            repo: did,
            collection: PUBLIC_KEY,
            rkey,
        };
        let _: serde_json::Value = self
            .derive(pds_base)
            .post_json_pub("com.atproto.repo.deleteRecord", &req, Some(access_jwt))
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: &str) -> PublicKey {
        PublicKey {
            key: k.to_string(),
            name: "n".into(),
            created_at: None,
        }
    }

    #[test]
    fn summarises_without_leaking_the_whole_body() {
        let s = key("ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFLbpBEpRe7F4Eb tangled-publish bot")
            .summary();
        assert!(s.starts_with("ssh-ed25519 ..."));
        assert!(s.ends_with("tangled-publish"));
        assert!(!s.contains("AAAAC3NzaC1lZDI1NTE5"));
    }

    #[test]
    fn survives_a_key_with_no_comment() {
        let s = key("ssh-rsa AAAAB3NzaC1yc2EAAAA").summary();
        assert!(s.starts_with("ssh-rsa ..."));
    }
}
