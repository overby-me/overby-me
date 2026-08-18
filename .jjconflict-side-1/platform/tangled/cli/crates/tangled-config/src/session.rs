use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::keychain::Keychain;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub access_jwt: String,
    pub refresh_jwt: String,
    pub did: String,
    pub handle: String,
    #[serde(default)]
    pub pds: Option<String>,
    #[serde(default)]
    pub created_at: DateTime<Utc>,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            access_jwt: String::new(),
            refresh_jwt: String::new(),
            did: String::new(),
            handle: String::new(),
            pds: None,
            created_at: Utc::now(),
        }
    }
}

pub struct SessionManager {
    service: String,
    account: String,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self {
            service: "tangled-cli".into(),
            account: "default".into(),
        }
    }
}

impl SessionManager {
    pub fn new(service: &str, account: &str) -> Self {
        Self {
            service: service.into(),
            account: account.into(),
        }
    }

    pub fn save(&self, session: &Session) -> Result<()> {
        let keychain = Keychain::new(&self.service, &self.account);
        let json = serde_json::to_string(session)?;
        keychain.set_password(&json)
    }

    pub fn load(&self) -> Result<Option<Session>> {
        let keychain = Keychain::new(&self.service, &self.account);
        match keychain.get_password() {
            Ok(json) => Ok(Some(serde_json::from_str(&json)?)),
            Err(_) => Ok(None),
        }
    }

    pub fn clear(&self) -> Result<()> {
        let keychain = Keychain::new(&self.service, &self.account);
        keychain.delete_password()
    }
}
