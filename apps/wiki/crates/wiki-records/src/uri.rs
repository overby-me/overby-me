//! `at://{authority}/space/{type}/{skey}[/{author}/{collection}/{rkey}]`
//! (proposal 0016, "Addressing"). The literal `space` stands where a public
//! URI has a collection, which always has dots, so the two never meet.

use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SpaceUri {
    pub authority: String,
    pub space_type: String,
    pub skey: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordUri {
    pub space: SpaceUri,
    /// The DID whose repository holds the record.
    pub author: String,
    pub collection: String,
    pub rkey: String,
}

impl SpaceUri {
    pub fn record(&self, author: &str, collection: &str, rkey: &str) -> RecordUri {
        RecordUri {
            space: self.clone(),
            author: author.to_string(),
            collection: collection.to_string(),
            rkey: rkey.to_string(),
        }
    }
}

impl fmt::Display for SpaceUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "at://{}/space/{}/{}",
            self.authority, self.space_type, self.skey
        )
    }
}

impl fmt::Display for RecordUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}/{}/{}",
            self.space, self.author, self.collection, self.rkey
        )
    }
}

fn segments(uri: &str) -> Option<Vec<&str>> {
    let parts: Vec<&str> = uri.strip_prefix("at://")?.split('/').collect();
    (parts.get(1) == Some(&"space") && parts.iter().all(|p| !p.is_empty())).then_some(parts)
}

impl FromStr for SpaceUri {
    type Err = ();
    fn from_str(uri: &str) -> Result<Self, ()> {
        match segments(uri).ok_or(())?.as_slice() {
            [authority, _, space_type, skey] => Ok(SpaceUri {
                authority: authority.to_string(),
                space_type: space_type.to_string(),
                skey: skey.to_string(),
            }),
            _ => Err(()),
        }
    }
}

impl FromStr for RecordUri {
    type Err = ();
    fn from_str(uri: &str) -> Result<Self, ()> {
        match segments(uri).ok_or(())?.as_slice() {
            [authority, _, space_type, skey, author, collection, rkey] => Ok(RecordUri {
                space: SpaceUri {
                    authority: authority.to_string(),
                    space_type: space_type.to_string(),
                    skey: skey.to_string(),
                },
                author: author.to_string(),
                collection: collection.to_string(),
                rkey: rkey.to_string(),
            }),
            _ => Err(()),
        }
    }
}
