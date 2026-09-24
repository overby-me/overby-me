//! Who a DID is: where its PDS is, which key it signs with, and, for a space's
//! authority, which key and host answer for its spaces (proposal 0016, "Space
//! authority").

use crate::Error;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub did: String,
    /// The `#atproto` key, as a multikey.
    pub signing_key: String,
    /// Service endpoints by fragment (`atproto_pds`, and whatever else).
    pub services: BTreeMap<String, String>,
    /// The `#atproto_space` key, where the authority publishes one.
    space_key: Option<String>,
}

impl Identity {
    pub fn pds(&self) -> Option<&str> {
        self.services.get("atproto_pds").map(String::as_str)
    }

    /// Where the authority's spaces are answered for: its own host for them,
    /// else its PDS.
    pub fn space_host(&self) -> Option<&str> {
        self.services
            .get("atproto_space_host")
            .map(String::as_str)
            .or(self.pds())
    }

    /// The key a space credential of this authority is signed with.
    pub fn space_key(&self) -> &str {
        self.space_key.as_deref().unwrap_or(&self.signing_key)
    }

    /// Read a DID document. A dedicated space key or host that is there and
    /// malformed is an error and no reason to fall back: whoever published it
    /// meant the fallback NOT to be used.
    pub fn from_document(doc: &Value) -> Result<Identity, Error> {
        let bad = |what: &str| Error::Malformed(format!("DID document: {what}"));
        let did = doc["id"].as_str().ok_or_else(|| bad("no id"))?.to_string();
        let mut keys = BTreeMap::new();
        for method in doc["verificationMethod"].as_array().into_iter().flatten() {
            let fragment = method["id"].as_str().and_then(|id| id.rsplit_once('#'));
            if let Some((_, fragment)) = fragment {
                keys.insert(fragment.to_string(), method["publicKeyMultibase"].as_str());
            }
        }
        let mut services = BTreeMap::new();
        for service in doc["service"].as_array().into_iter().flatten() {
            let fragment = service["id"].as_str().and_then(|id| id.rsplit_once('#'));
            let Some((_, fragment)) = fragment else {
                continue;
            };
            match service["serviceEndpoint"].as_str() {
                Some(endpoint) => {
                    services.insert(
                        fragment.to_string(),
                        endpoint.trim_end_matches('/').to_string(),
                    );
                }
                None if fragment == "atproto_space_host" => {
                    return Err(bad("a space host with no endpoint"));
                }
                None => {}
            }
        }
        Ok(Identity {
            did,
            signing_key: keys
                .get("atproto")
                .copied()
                .flatten()
                .ok_or_else(|| bad("no #atproto key"))?
                .to_string(),
            space_key: match keys.get("atproto_space") {
                None => None,
                Some(Some(key)) => Some(key.to_string()),
                Some(None) => return Err(bad("a space key that is no multikey")),
            },
            services,
        })
    }
}

/// Resolves a DID to its document. `did:plc` by a directory, `did:web` by the
/// domain it names.
#[derive(Clone)]
pub struct Directory {
    http: reqwest::Client,
    plc: String,
}

impl Directory {
    pub fn new(http: reqwest::Client, plc_url: &str) -> Self {
        Directory {
            http,
            plc: plc_url.trim_end_matches('/').to_string(),
        }
    }

    pub async fn resolve(&self, did: &str) -> Result<Identity, Error> {
        let url = match did
            .split_once(':')
            .and_then(|(_, rest)| rest.split_once(':'))
        {
            Some(("plc", _)) => format!("{}/{did}", self.plc),
            // A port is percent-encoded in a did:web, and a path would follow
            // further colons: neither is a host a wiki's member lives on.
            Some(("web", host)) if !host.contains([':', '%', '/']) => {
                format!("https://{host}/.well-known/did.json")
            }
            _ => {
                return Err(Error::Malformed(format!(
                    "a DID this cannot resolve: {did}"
                )));
            }
        };
        let answer = self.http.get(url).send().await?;
        if !answer.status().is_success() {
            return Err(Error::Malformed(format!(
                "{did} did not resolve ({})",
                answer.status()
            )));
        }
        let identity = Identity::from_document(&answer.json().await?)?;
        match identity.did == did {
            true => Ok(identity),
            false => Err(Error::Unverified(format!(
                "{did} resolved to another DID's document"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document() -> Value {
        json!({
            "id": "did:plc:org",
            "verificationMethod": [
                {"id": "did:plc:org#atproto", "type": "Multikey", "publicKeyMultibase": "zKey"},
            ],
            "service": [
                {"id": "#atproto_pds", "type": "AtprotoPersonalDataServer",
                 "serviceEndpoint": "https://pds.example/"},
            ],
        })
    }

    #[test]
    fn an_authority_falls_back_to_its_account_unless_it_says_otherwise() {
        let plain = Identity::from_document(&document()).expect("an identity");
        assert_eq!(plain.space_host(), Some("https://pds.example"));
        assert_eq!(plain.space_key(), "zKey");

        let mut own = document();
        own["verificationMethod"]
            .as_array_mut()
            .expect("methods")
            .push(json!({"id": "did:plc:org#atproto_space", "publicKeyMultibase": "zSpaceKey"}));
        own["service"].as_array_mut().expect("services").push(
            json!({"id": "#atproto_space_host", "serviceEndpoint": "https://spaces.example"}),
        );
        let own = Identity::from_document(&own).expect("an identity");
        assert_eq!(own.space_host(), Some("https://spaces.example"));
        assert_eq!(own.space_key(), "zSpaceKey");

        let mut broken = document();
        broken["verificationMethod"]
            .as_array_mut()
            .expect("methods")
            .push(json!({"id": "did:plc:org#atproto_space"}));
        assert!(
            Identity::from_document(&broken).is_err(),
            "published and malformed is not absent"
        );
    }
}
