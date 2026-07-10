//! atproto platform detection.
//!
//! Everything a graph needs about *which* platforms an account uses comes from
//! its PDS: `com.atproto.repo.describeRepo` returns the `collections` list, and
//! every collection NSID is a reverse-DNS domain identifying the app that owns
//! that lexicon. This module turns that raw list into a tidy set of platforms.
//!
//! It is deliberately pure (no web-sys), so the interesting logic — grouping,
//! aliasing, domain derivation, profile links — is unit tested on the host. The
//! browser-only fetch glue lives in [`crate::atproto_web`].

/// Where a platform node's icon comes from.
#[derive(Clone, PartialEq, Debug)]
pub enum IconSpec {
    /// A bundled local logo, by asset filename (e.g. `"bluesky.avif"`).
    Bundled(&'static str),
    /// The favicon of the given domain, fetched through a CORS-enabled service.
    Favicon(String),
}

/// One atproto app the account has records in.
#[derive(Clone, PartialEq, Debug)]
pub struct Platform {
    pub name: String,
    pub domain: String,
    pub color: Option<&'static str>,
    pub icon: IconSpec,
    /// Where clicking the node should take you (a profile if we know the shape,
    /// otherwise the app's homepage).
    pub profile_url: String,
}

/// NSID prefixes that are shared/community namespaces or core protocol, not a
/// single user-facing app. They should not become their own platform node.
const SKIP_PREFIXES: &[&str] = &[
    "com.atproto",       // core protocol
    "community.lexicon", // shared community lexicons (e.g. calendar)
];

/// The 2-segment authority prefix of an NSID (`app.bsky.feed.post` -> `app.bsky`).
/// Returns `None` for anything without at least an authority + one more segment.
fn group_prefix(nsid: &str) -> Option<String> {
    let mut it = nsid.split('.');
    let a = it.next()?;
    let b = it.next()?;
    // Require at least one further segment so bare two-part strings (which are
    // never valid collection NSIDs) don't slip through as platforms.
    it.next()?;
    if a.is_empty() || b.is_empty() {
        return None;
    }
    Some(format!("{a}.{b}"))
}

/// Collapse prefixes that belong to the same app (e.g. Bluesky's chat lexicons
/// live under `chat.bsky` but are still Bluesky).
fn canonical_prefix(prefix: &str) -> &str {
    match prefix {
        "chat.bsky" => "app.bsky",
        other => other,
    }
}

/// Reverse a 2-segment prefix into a domain: `sh.tangled` -> `tangled.sh`.
fn reverse_domain(prefix: &str) -> String {
    let mut parts: Vec<&str> = prefix.split('.').collect();
    parts.reverse();
    parts.join(".")
}

/// Uppercase the first character of the org label for a display name fallback.
fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

struct Curated {
    name: &'static str,
    color: Option<&'static str>,
    /// Bundled logo filename, or `None` to fall back to the domain favicon.
    icon: Option<&'static str>,
    profile_url: String,
}

/// Hand-curated metadata for well-known apps: nice names, brand-accurate logos,
/// and real profile links. Unknown apps fall back to derived values.
fn curated(prefix: &str, handle: &str, did: &str) -> Option<Curated> {
    let c = |name, color, icon, profile_url: String| Curated {
        name,
        color,
        icon,
        profile_url,
    };
    Some(match prefix {
        "app.bsky" => c(
            "Bluesky",
            None,
            Some("bluesky.avif"),
            format!("https://bsky.app/profile/{handle}"),
        ),
        "sh.tangled" => c(
            "Tangled",
            None,
            Some("tangled.avif"),
            format!("https://tangled.org/@{handle}"),
        ),
        "app.rocksky" => c(
            "Rocksky",
            None,
            Some("rocksky.avif"),
            format!("https://rocksky.app/profile/{handle}"),
        ),
        "app.pinkleap" => c(
            "PinkLeap",
            None,
            Some("pinkleap.avif"),
            format!("https://pinkleap.app/@{handle}"),
        ),
        "social.popfeed" => c(
            "PopFeed",
            None,
            Some("popfeed.avif"),
            format!("https://popfeed.social/profile/{did}"),
        ),
        "social.pinksky" => c(
            "Pinksky",
            None,
            None,
            format!("https://pinksky.social/profile/{handle}"),
        ),
        "pub.leaflet" => c("Leaflet", None, None, "https://leaflet.pub".to_string()),
        "fm.teal" => c("Teal.fm", None, None, "https://teal.fm".to_string()),
        "events.smokesignal" => c(
            "Smoke Signal",
            None,
            None,
            "https://smokesignal.events".to_string(),
        ),
        "place.stream" => c(
            "Stream.place",
            None,
            None,
            "https://stream.place".to_string(),
        ),
        "id.sifa" => c("Sifa", None, None, "https://sifa.id".to_string()),
        _ => return None,
    })
}

/// The favicon URL for a domain, via a CORS-enabled favicon service (required so
/// the image can be used as a WebGL texture without tainting the canvas).
pub fn favicon_url(domain: &str) -> String {
    format!("https://favicone.com/{domain}?s=128")
}

impl IconSpec {
    /// Resolve to a final, loadable image URL.
    pub fn resolve(&self) -> String {
        match self {
            IconSpec::Bundled(name) => crate::graph::texture::icon_url(name),
            IconSpec::Favicon(domain) => favicon_url(domain),
        }
    }
}

/// Detect the platforms an account uses from its `describeRepo` collection list.
///
/// Collections are grouped by 2-segment NSID authority, aliased and de-duped by
/// display name, then enriched from the curated registry (or derived for unknown
/// apps). The result is sorted by name for a stable, tidy graph.
pub fn detect_platforms(collections: &[String], handle: &str, did: &str) -> Vec<Platform> {
    // Unique canonical prefixes, skipping shared/infra namespaces.
    let mut prefixes: Vec<String> = Vec::new();
    for nsid in collections {
        let Some(prefix) = group_prefix(nsid) else {
            continue;
        };
        let prefix = canonical_prefix(&prefix).to_string();
        if SKIP_PREFIXES.contains(&prefix.as_str()) {
            continue;
        }
        if !prefixes.contains(&prefix) {
            prefixes.push(prefix);
        }
    }

    let mut platforms: Vec<Platform> = Vec::new();
    let mut seen_names: Vec<String> = Vec::new();
    for prefix in &prefixes {
        let domain = reverse_domain(prefix);
        let platform = match curated(prefix, handle, did) {
            Some(c) => Platform {
                name: c.name.to_string(),
                domain: domain.clone(),
                color: c.color,
                icon: c
                    .icon
                    .map_or_else(|| IconSpec::Favicon(domain.clone()), IconSpec::Bundled),
                profile_url: c.profile_url,
            },
            None => {
                // org label is the second segment (e.g. `sifa` in `id.sifa`).
                let org = prefix.split('.').nth(1).unwrap_or(prefix);
                Platform {
                    name: title_case(org),
                    domain: domain.clone(),
                    color: None,
                    icon: IconSpec::Favicon(domain.clone()),
                    profile_url: format!("https://{domain}"),
                }
            }
        };
        // Collapse apps that resolve to the same display name across TLDs
        // (e.g. `app.shadowsky` + `com.shadowsky`).
        let key = platform.name.to_lowercase();
        if seen_names.contains(&key) {
            continue;
        }
        seen_names.push(key);
        platforms.push(platform);
    }

    platforms.sort_by_key(|p| p.name.to_lowercase());
    platforms
}

#[cfg(test)]
mod tests {
    use super::*;

    // The real describeRepo collection list for @overby.me, captured live. This
    // pins the detection behavior to actual PDS data.
    fn overby_collections() -> Vec<String> {
        [
            "actor.rpg.generator",
            "actor.rpg.sprite",
            "actor.rpg.stats",
            "app.bsky.actor.profile",
            "app.bsky.feed.like",
            "app.bsky.feed.post",
            "app.bsky.feed.repost",
            "app.bsky.graph.block",
            "app.bsky.graph.follow",
            "app.fitsky.profile",
            "app.pinkleap.declaration",
            "app.rocksky.album",
            "app.rocksky.scrobble",
            "app.rocksky.song",
            "chat.bsky.actor.declaration",
            "com.atprotofans.profile",
            "community.lexicon.calendar.event",
            "community.lexicon.calendar.rsvp",
            "events.smokesignal.calendar.acceptance",
            "fm.teal.alpha.actor.status",
            "fm.teal.alpha.feed.play",
            "id.sifa.profile.self",
            "id.sifa.profile.skill",
            "place.stream.livestream",
            "pub.leaflet.interactions.recommend",
            "sh.tangled.repo",
            "sh.tangled.feed.star",
            "social.pinksky.app.preference",
            "social.popfeed.actor.profile",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    fn names(platforms: &[Platform]) -> Vec<String> {
        platforms.iter().map(|p| p.name.clone()).collect()
    }

    #[test]
    fn detects_expected_platforms() {
        let p = detect_platforms(&overby_collections(), "overby.me", "did:plc:abc");
        let names = names(&p);
        for expected in [
            "Bluesky",
            "Tangled",
            "Rocksky",
            "PinkLeap",
            "PopFeed",
            "Leaflet",
            "Teal.fm",
            "Smoke Signal",
            "Pinksky",
            "Stream.place",
            "Sifa",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "missing {expected} in {names:?}"
            );
        }
    }

    #[test]
    fn merges_bluesky_chat_into_one_node() {
        let p = detect_platforms(&overby_collections(), "overby.me", "did:plc:abc");
        let bsky = p.iter().filter(|p| p.name == "Bluesky").count();
        assert_eq!(
            bsky, 1,
            "app.bsky and chat.bsky should collapse to one Bluesky node"
        );
    }

    #[test]
    fn skips_shared_and_infra_namespaces() {
        let p = detect_platforms(&overby_collections(), "overby.me", "did:plc:abc");
        // community.lexicon.* must not surface as a platform.
        assert!(
            !names(&p).iter().any(|n| n.eq_ignore_ascii_case("lexicon")),
            "community.lexicon should be skipped: {:?}",
            names(&p)
        );
    }

    #[test]
    fn derives_domain_and_favicon_for_unknown_apps() {
        let p = detect_platforms(
            &["net.anisota.beta.game.log".to_string()],
            "overby.me",
            "did:plc:abc",
        );
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "Anisota");
        assert_eq!(p[0].domain, "anisota.net");
        assert_eq!(p[0].icon, IconSpec::Favicon("anisota.net".to_string()));
        assert_eq!(p[0].profile_url, "https://anisota.net");
    }

    #[test]
    fn curated_bluesky_uses_bundled_icon_and_profile_link() {
        let p = detect_platforms(
            &["app.bsky.feed.post".to_string()],
            "overby.me",
            "did:plc:abc",
        );
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].icon, IconSpec::Bundled("bluesky.avif"));
        assert_eq!(p[0].profile_url, "https://bsky.app/profile/overby.me");
    }

    #[test]
    fn reverse_domain_reverses_two_segments() {
        assert_eq!(reverse_domain("sh.tangled"), "tangled.sh");
        assert_eq!(reverse_domain("social.popfeed"), "popfeed.social");
        assert_eq!(reverse_domain("app.bsky"), "bsky.app");
    }

    #[test]
    fn group_prefix_needs_three_segments() {
        assert_eq!(
            group_prefix("app.bsky.feed.post").as_deref(),
            Some("app.bsky")
        );
        assert_eq!(
            group_prefix("app.bsky.profile").as_deref(),
            Some("app.bsky")
        );
        assert_eq!(group_prefix("app.bsky"), None); // authority only, not a collection
        assert_eq!(group_prefix("bsky"), None);
    }
}
