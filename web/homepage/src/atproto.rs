//! atproto platform detection.
//!
//! Everything a graph needs about *which* platforms an account uses comes from
//! its PDS: `com.atproto.repo.describeRepo` returns the `collections` list, and
//! every collection NSID is a reverse-DNS domain identifying the app that owns
//! that lexicon. This module turns that raw list into a tidy set of platforms.
//!
//! Each platform gets a real bundled logo where we ship one (pre-made circular
//! badges in `assets/icons/`), otherwise a generated circular badge — never a
//! runtime fetch — so every node looks consistent and stays crisp.
//!
//! It is deliberately pure (no web-sys), so the interesting logic — grouping,
//! aliasing, domain derivation, badge generation, profile links — is unit
//! tested on the host. The browser-only fetch glue lives in [`crate::atproto_web`].

/// A platform's icon: a real bundled logo where we ship one, otherwise a
/// generated circular badge (so any app still gets a consistent round icon).
#[derive(Clone, PartialEq, Debug)]
pub enum Icon {
    /// A bundled logo asset filename (e.g. `"leaflet.avif"`).
    Bundled(&'static str),
    /// A generated circular badge as an SVG data URL.
    Badge(String),
}

impl Icon {
    /// Resolve to a final, loadable image URL.
    pub fn resolve(&self) -> String {
        match self {
            Icon::Bundled(file) => crate::graph::texture::icon_url(file),
            Icon::Badge(data_url) => data_url.clone(),
        }
    }
}

/// One atproto app the account has records in.
#[derive(Clone, PartialEq, Debug)]
pub struct Platform {
    pub name: String,
    pub domain: String,
    pub icon: Icon,
    /// Where clicking the node should take you (a profile if we know the shape,
    /// otherwise the app's homepage).
    pub profile_url: String,
    /// The intermediate category node this platform groups under.
    pub category: &'static str,
}

/// Real bundled logos, keyed by NSID authority. These are pre-made circular
/// badges in `assets/icons/` (fetched from each app and processed once), so the
/// runtime fetches nothing. Apps without an entry fall back to a generated badge.
fn bundled_icon(prefix: &str) -> Option<&'static str> {
    Some(match prefix {
        "app.bsky" => "bluesky.avif",
        "sh.tangled" => "tangled.avif",
        "app.rocksky" => "rocksky.avif",
        "app.pinkleap" => "pinkleap.avif",
        "social.popfeed" => "popfeed.avif",
        "pub.leaflet" => "leaflet.avif",
        "events.smokesignal" => "smokesignal.avif",
        "place.stream" => "streamplace.avif",
        "id.sifa" => "sifa.avif",
        "app.fitsky" => "fitsky.avif",
        "com.atprotofans" => "atprotofans.avif",
        "app.skyreader" => "skyreader.avif",
        "computer.aetheros" => "aetheros.avif",
        "dev.npmx" => "npmx.avif",
        "net.anisota" => "anisota.avif",
        "so.sprk" => "spark.avif",
        "fm.teal" => "teal.avif",
        "actor.rpg" => "rpg.avif",
        "equipment.rpg" => "rpg.avif",
        "link.woosh" => "woosh.avif",
        "com.vibe-coded" => "vibecoded.avif",
        "place.atwork" => "atwork.avif",
        "app.shadowsky" | "com.shadowsky" => "shadowsky.avif",
        "blue.pronouns" => "pronouns.avif",
        "farm.smol" => "smol.avif",
        "app.sidetrail" => "sidetrail.avif",
        "at.marque" => "marque.avif",
        "blue.checkmate" => "checkmate.avif",
        "site.standard" => "standard.avif",
        _ => return None,
    })
}

/// NSID prefixes that are shared/community namespaces or core protocol, not a
/// single user-facing app. They should not become their own platform node.
const SKIP_PREFIXES: &[&str] = &[
    "com.atproto",       // core protocol
    "community.lexicon", // shared community lexicons (e.g. calendar)
    "my.test",           // scratch/test records, not a real app
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

/// Collapse prefixes that belong to the same app: Bluesky's chat lexicons live
/// under `chat.bsky`, and Pinksky was renamed to PinkLeap (`social.pinksky` is
/// the old lexicon).
fn canonical_prefix(prefix: &str) -> &str {
    match prefix {
        "chat.bsky" => "app.bsky",
        "social.pinksky" => "app.pinkleap",
        other => other,
    }
}

/// The intermediate category a platform groups under (like the personal graph's
/// Connect / Immerse / … hubs). Unknown apps land under `Explore`.
fn category_for(prefix: &str) -> &'static str {
    match prefix {
        // Social & messaging.
        "app.bsky" | "app.pinkleap" | "so.sprk" | "app.shadowsky" | "com.shadowsky" => "Connect",
        // Writing, code, publishing, tools.
        "pub.leaflet" | "sh.tangled" | "dev.npmx" | "com.vibe-coded" | "site.standard"
        | "at.marque" | "com.minomobi" => "Create",
        // Music, video, games, reading.
        "app.rocksky" | "fm.teal" | "place.stream" | "app.skytube" | "app.skyreader"
        | "social.popfeed" | "net.anisota" | "actor.rpg" | "equipment.rpg" | "farm.smol"
        | "blue.checkmate" => "Immerse",
        // Events, community, links.
        "events.smokesignal" | "link.woosh" | "com.atprotofans" => "Gather",
        // Profile, work, fitness, identity.
        "id.sifa" | "place.atwork" | "app.fitsky" | "computer.aetheros" | "blue.pronouns"
        | "app.sidetrail" => "Grow",
        _ => "Explore",
    }
}

/// Category node ordering, so hubs appear in a stable, sensible sequence.
pub const CATEGORY_ORDER: &[&str] = &["Connect", "Create", "Immerse", "Gather", "Grow", "Explore"];

// Category hub symbols: white glyphs on a transparent background, shown inside
// the node's translucent colored halo.
const CONNECT_SYMBOL: &str = "<rect x='56' y='66' width='144' height='100' rx='26' fill='#fff'/><polygon points='96,166 96,198 128,166' fill='#fff'/>";
const CREATE_SYMBOL: &str =
    "<polygon points='128,58 143,113 198,128 143,143 128,198 113,143 58,128 113,113' fill='#fff'/>";
const IMMERSE_SYMBOL: &str = "<polygon points='104,82 104,174 182,128' fill='#fff'/>";
const GATHER_SYMBOL: &str = "<circle cx='100' cy='114' r='28' fill='#fff'/><circle cx='156' cy='114' r='28' fill='#fff'/><circle cx='128' cy='166' r='28' fill='#fff'/>";
const GROW_SYMBOL: &str =
    "<polygon points='128,58 192,128 156,128 156,198 100,198 100,128 64,128' fill='#fff'/>";
const EXPLORE_SYMBOL: &str = "<circle cx='128' cy='128' r='56' fill='none' stroke='#fff' stroke-width='14'/><polygon points='128,98 140,128 128,158 116,128' fill='#fff'/>";

/// Halo color + symbol icon (SVG data URL) for a category hub node.
pub fn category_meta(name: &str) -> (&'static str, String) {
    let (color, symbol) = match name {
        "Connect" => ("#e34234", CONNECT_SYMBOL),
        "Create" => ("#7fc24a", CREATE_SYMBOL),
        "Immerse" => ("#ff7f50", IMMERSE_SYMBOL),
        "Gather" => ("#45b1e8", GATHER_SYMBOL),
        "Grow" => ("#6a5acd", GROW_SYMBOL),
        _ => ("#9aa0b5", EXPLORE_SYMBOL),
    };
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='256' height='256' viewBox='0 0 256 256'>{symbol}</svg>"
    );
    (
        color,
        format!("data:image/svg+xml,{}", percent_encode(&svg)),
    )
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
    /// Brand color for the badge disc.
    color: &'static str,
    profile_url: String,
}

/// Hand-curated metadata for well-known apps: nice names, brand colors, and real
/// profile links. Unknown apps fall back to derived values.
fn curated(prefix: &str, handle: &str, did: &str) -> Option<Curated> {
    let c = |name, color, profile_url| Curated {
        name,
        color,
        profile_url,
    };
    Some(match prefix {
        "app.bsky" => c(
            "Bluesky",
            "#1185fe",
            format!("https://bsky.app/profile/{handle}"),
        ),
        "sh.tangled" => c(
            "Tangled",
            "#5b4bff",
            format!("https://tangled.org/@{handle}"),
        ),
        "app.rocksky" => c(
            "Rocksky",
            "#e0245e",
            format!("https://rocksky.app/profile/{handle}"),
        ),
        "app.pinkleap" => c(
            "PinkLeap",
            "#ec4899",
            format!("https://pinkleap.app/@{handle}"),
        ),
        "social.popfeed" => c(
            "PopFeed",
            "#e8590c",
            format!("https://popfeed.social/profile/{did}"),
        ),
        "pub.leaflet" => c("Leaflet", "#0f9d58", "https://leaflet.pub".to_string()),
        "fm.teal" => c("Teal.fm", "#0d9488", "https://teal.fm".to_string()),
        "events.smokesignal" => c(
            "Smoke Signal",
            "#ea580c",
            "https://smokesignal.events".to_string(),
        ),
        "place.stream" => c(
            "Stream.place",
            "#7c3aed",
            "https://stream.place".to_string(),
        ),
        "id.sifa" => c("Sifa", "#2563eb", "https://sifa.id".to_string()),
        _ => return None,
    })
}

/// Percent-encode a string for use in a data URL (encode everything outside the
/// unreserved set, so arbitrary SVG/text is always safe).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A high-res circular badge as an SVG data URL: a filled disc in `color` with
/// `name`'s uppercase initial in white. Uniform across platforms and crisp at
/// any size, unlike fetched favicons.
pub fn badge_icon(name: &str, color: &str) -> String {
    let initial: String = name
        .chars()
        .find(|c| c.is_alphanumeric())
        .map_or_else(|| "?".to_string(), |c| c.to_uppercase().to_string());
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='256' height='256' viewBox='0 0 256 256'>\
         <circle cx='128' cy='128' r='128' fill='{color}'/>\
         <text x='128' y='128' text-anchor='middle' dominant-baseline='central' \
         font-family='Space Grotesk, system-ui, sans-serif' font-size='140' font-weight='700' \
         fill='#ffffff'>{initial}</text></svg>"
    );
    format!("data:image/svg+xml,{}", percent_encode(&svg))
}

/// A stable, pleasant disc color for apps without a brand color: hash the seed
/// to a hue with fixed saturation/lightness so the palette stays cohesive and
/// white text keeps good contrast.
fn derived_color(seed: &str) -> String {
    // FNV-1a hash -> hue.
    let mut h: u32 = 2_166_136_261;
    for b in seed.bytes() {
        h = (h ^ b as u32).wrapping_mul(16_777_619);
    }
    hsl_to_hex((h % 360) as f32, 0.62, 0.48)
}

fn hsl_to_hex(h: f32, s: f32, l: f32) -> String {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let byte = |v: f32| ((v + m) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b))
}

/// A fallback avatar badge for an account with no picture: its display initial
/// on a derived color, matching the platform badges.
pub fn fallback_avatar(name: &str, seed: &str) -> String {
    badge_icon(name, &derived_color(seed))
}

/// Detect the platforms an account uses from its `describeRepo` collection list.
///
/// Collections are grouped by 2-segment NSID authority, aliased and de-duped by
/// display name, then enriched from the curated registry (or derived for unknown
/// apps) and given a generated badge. The result is sorted by name for a stable,
/// tidy graph.
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
        let (name, color, profile_url) = match curated(prefix, handle, did) {
            Some(c) => (c.name.to_string(), c.color.to_string(), c.profile_url),
            None => {
                // org label is the second segment (e.g. `sifa` in `id.sifa`).
                let org = prefix.split('.').nth(1).unwrap_or(prefix);
                (
                    title_case(org),
                    derived_color(&domain),
                    format!("https://{domain}"),
                )
            }
        };
        // Collapse apps that resolve to the same display name across TLDs
        // (e.g. `app.shadowsky` + `com.shadowsky`).
        let key = name.to_lowercase();
        if seen_names.contains(&key) {
            continue;
        }
        seen_names.push(key);
        // Prefer a real bundled logo; otherwise a generated badge.
        let icon = match bundled_icon(prefix) {
            Some(file) => Icon::Bundled(file),
            None => Icon::Badge(badge_icon(&name, &color)),
        };
        platforms.push(Platform {
            icon,
            category: category_for(prefix),
            name,
            domain,
            profile_url,
        });
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
            "Stream.place",
            "Sifa",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "missing {expected} in {names:?}"
            );
        }
        // Pinksky was renamed to PinkLeap; it must not appear as its own node.
        assert!(
            !names.contains(&"Pinksky".to_string()),
            "Pinksky should merge into PinkLeap: {names:?}"
        );
    }

    #[test]
    fn merges_pinksky_into_pinkleap() {
        // Old (social.pinksky) and new (app.pinkleap) lexicons -> one PinkLeap.
        let p = detect_platforms(
            &[
                "social.pinksky.app.preference".to_string(),
                "app.pinkleap.declaration".to_string(),
            ],
            "overby.me",
            "did:plc:abc",
        );
        assert_eq!(names(&p), vec!["PinkLeap".to_string()]);
        assert_eq!(p[0].icon, Icon::Bundled("pinkleap.avif"));
    }

    #[test]
    fn known_apps_use_bundled_logos() {
        let p = detect_platforms(&overby_collections(), "overby.me", "did:plc:abc");
        let icon_of = |name: &str| p.iter().find(|x| x.name == name).map(|x| x.icon.clone());
        assert_eq!(icon_of("Bluesky"), Some(Icon::Bundled("bluesky.avif")));
        assert_eq!(icon_of("Leaflet"), Some(Icon::Bundled("leaflet.avif")));
        assert_eq!(icon_of("Sifa"), Some(Icon::Bundled("sifa.avif")));
        assert_eq!(
            icon_of("Stream.place"),
            Some(Icon::Bundled("streamplace.avif"))
        );
        assert_eq!(icon_of("Teal.fm"), Some(Icon::Bundled("teal.avif")));
        // (The generated-badge fallback for logo-less apps is covered by
        // `derives_name_and_badge_for_unknown_apps`.)
    }

    #[test]
    fn assigns_platforms_to_categories() {
        let p = detect_platforms(&overby_collections(), "overby.me", "did:plc:abc");
        let cat_of = |name: &str| p.iter().find(|x| x.name == name).map(|x| x.category);
        assert_eq!(cat_of("Bluesky"), Some("Connect"));
        assert_eq!(cat_of("Tangled"), Some("Create"));
        assert_eq!(cat_of("Rocksky"), Some("Immerse"));
        assert_eq!(cat_of("Smoke Signal"), Some("Gather"));
        assert_eq!(cat_of("Sifa"), Some("Grow"));
        // Every category used must be listed in CATEGORY_ORDER.
        for platform in &p {
            assert!(
                CATEGORY_ORDER.contains(&platform.category),
                "{}",
                platform.category
            );
        }
    }

    #[test]
    fn unknown_apps_fall_under_explore() {
        let p = detect_platforms(
            &["com.example.thing.record".to_string()],
            "overby.me",
            "did:plc:abc",
        );
        assert_eq!(p[0].category, "Explore");
    }

    #[test]
    fn category_meta_gives_color_and_svg_icon() {
        for cat in CATEGORY_ORDER {
            let (color, icon) = category_meta(cat);
            assert!(color.starts_with('#'), "{cat} color: {color}");
            assert!(icon.starts_with("data:image/svg+xml,"), "{cat} icon");
        }
        // Unknown name uses the Explore fallback styling.
        assert_eq!(category_meta("Nonsense").0, "#9aa0b5");
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
    fn derives_name_and_badge_for_unknown_apps() {
        let p = detect_platforms(
            &["com.example.thing.record".to_string()],
            "overby.me",
            "did:plc:abc",
        );
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "Example");
        assert_eq!(p[0].domain, "example.com");
        assert_eq!(p[0].profile_url, "https://example.com");
        // A '#'-bearing brand color percent-encodes to %23 in the badge SVG.
        match &p[0].icon {
            Icon::Badge(url) => {
                assert!(url.starts_with("data:image/svg+xml,") && url.contains("%23"))
            }
            other => panic!("unknown app should get a generated badge, got {other:?}"),
        }
    }

    #[test]
    fn curated_bluesky_uses_bundled_logo_and_profile_link() {
        let p = detect_platforms(
            &["app.bsky.feed.post".to_string()],
            "overby.me",
            "did:plc:abc",
        );
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].icon, Icon::Bundled("bluesky.avif"));
        assert_eq!(p[0].profile_url, "https://bsky.app/profile/overby.me");
    }

    #[test]
    fn hsl_to_hex_matches_known_values() {
        assert_eq!(hsl_to_hex(0.0, 1.0, 0.5), "#ff0000");
        assert_eq!(hsl_to_hex(120.0, 1.0, 0.5), "#00ff00");
        assert_eq!(hsl_to_hex(240.0, 1.0, 0.5), "#0000ff");
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
