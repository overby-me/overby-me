//! Strings: `sh.tangled.string`.
//!
//! Tangled's paste: a named blob of text stored as a record on your own PDS,
//! with no repo and no knot involved. Everything here is plain atproto record
//! traffic.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::client::TangledClient;

pub const STRING: &str = "sh.tangled.string";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TangledString {
    pub filename: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub contents: String,
    #[serde(rename = "createdAt", default)]
    pub created_at: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StringRecord {
    pub rkey: String,
    pub value: TangledString,
}

impl TangledString {
    /// A one-line preview, for a listing that should not print a whole file.
    pub fn preview(&self) -> String {
        let first = self.contents.lines().next().unwrap_or("").trim();
        let short: String = first.chars().take(60).collect();
        if short.len() < first.len() {
            format!("{short}…")
        } else {
            short
        }
    }

    pub fn line_count(&self) -> usize {
        if self.contents.is_empty() {
            0
        } else {
            self.contents.lines().count()
        }
    }
}

impl TangledClient {
    pub async fn list_strings(&self, did: &str, bearer: Option<&str>) -> Result<Vec<StringRecord>> {
        #[derive(Deserialize)]
        struct Item {
            uri: String,
            value: TangledString,
        }
        #[derive(Deserialize)]
        struct Res {
            #[serde(default)]
            records: Vec<Item>,
        }
        let params = vec![
            ("repo", did.to_string()),
            ("collection", STRING.to_string()),
            ("limit", "100".to_string()),
        ];
        let res: Res = self
            .get_json("com.atproto.repo.listRecords", &params, bearer)
            .await?;
        Ok(res
            .records
            .into_iter()
            .map(|i| StringRecord {
                rkey: Self::uri_rkey(&i.uri).unwrap_or_default(),
                value: i.value,
            })
            .collect())
    }

    pub async fn get_string(
        &self,
        did: &str,
        rkey: &str,
        bearer: Option<&str>,
    ) -> Result<TangledString> {
        #[derive(Deserialize)]
        struct Res {
            value: TangledString,
        }
        let params = [
            ("repo", did.to_string()),
            ("collection", STRING.to_string()),
            ("rkey", rkey.to_string()),
        ];
        let res: Res = self
            .get_json("com.atproto.repo.getRecord", &params, bearer)
            .await?;
        Ok(res.value)
    }

    pub async fn create_string(
        &self,
        did: &str,
        filename: &str,
        description: &str,
        contents: &str,
        pds_base: &str,
        access_jwt: &str,
    ) -> Result<String> {
        #[derive(Serialize)]
        struct Rec<'a> {
            #[serde(rename = "$type")]
            lexicon_type: &'a str,
            filename: &'a str,
            description: &'a str,
            contents: &'a str,
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
            collection: STRING,
            validate: false,
            record: Rec {
                lexicon_type: STRING,
                filename,
                description,
                contents,
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        };
        let res: Res = self
            .derive(pds_base)
            .post_json_pub("com.atproto.repo.createRecord", &req, Some(access_jwt))
            .await?;
        Self::uri_rkey(&res.uri).ok_or_else(|| anyhow!("missing rkey in string uri"))
    }

    pub async fn delete_string(
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
            collection: STRING,
            rkey,
        };
        self.derive(pds_base)
            .post("com.atproto.repo.deleteRecord", &req, Some(access_jwt))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(contents: &str) -> TangledString {
        TangledString {
            filename: "f".into(),
            description: String::new(),
            contents: contents.into(),
            created_at: None,
        }
    }

    #[test]
    fn previews_only_the_first_line() {
        assert_eq!(s("first\nsecond\nthird").preview(), "first");
        assert_eq!(s("").preview(), "");
    }

    #[test]
    fn marks_a_truncated_preview() {
        let long = "x".repeat(100);
        let p = s(&long).preview();
        assert!(p.ends_with('…'));
        assert_eq!(p.chars().count(), 61);
    }

    #[test]
    fn counts_lines() {
        assert_eq!(s("").line_count(), 0);
        assert_eq!(s("one").line_count(), 1);
        assert_eq!(s("one\ntwo\n").line_count(), 2);
    }
}
