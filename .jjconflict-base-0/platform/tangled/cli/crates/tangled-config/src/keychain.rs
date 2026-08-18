use anyhow::{anyhow, Result};
use keyring::Entry;

pub struct Keychain {
    service: String,
    account: String,
}

impl Keychain {
    pub fn new(service: &str, account: &str) -> Self {
        Self {
            service: service.into(),
            account: account.into(),
        }
    }

    fn entry(&self) -> Result<Entry> {
        Entry::new(&self.service, &self.account).map_err(|e| anyhow!("keyring error: {e}"))
    }

    pub fn set_password(&self, secret: &str) -> Result<()> {
        self.entry()?
            .set_password(secret)
            .map_err(|e| anyhow!("keyring error: {e}"))
    }

    pub fn get_password(&self) -> Result<String> {
        self.entry()?
            .get_password()
            .map_err(|e| anyhow!("keyring error: {e}"))
    }

    pub fn delete_password(&self) -> Result<()> {
        self.entry()?
            .delete_credential()
            .map_err(|e| anyhow!("keyring error: {e}"))
    }
}
