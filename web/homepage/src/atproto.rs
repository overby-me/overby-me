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
        "blue.2048" => "2048.avif",
        "blue.linkat" => "linkat.avif",
        "com.skymeetsblue" => "skymeetsblue.avif",
        "at.youandme" => "youandme.avif",
        "blog.pckt" => "pckt.avif",
        "network.cosmik" => "cosmik.avif",
        "org.atmosphereconf" => "atmoconf.avif",
        "pub.chive" => "chive.avif",
        "social.mu" => "musocial.avif",
        "social.twinkl" => "twinkl.avif",
        "space.roomy" => "roomy.avif",
        "app.cartes" => "cartes.avif",
        "coop.hypha" => "hypha.avif",
        "app.lanyards" => "lanyards.avif",
        "com.semble" => "semble.avif",
        "blue.flashes" => "flashes.avif",
        // Known apps with no fetchable logo (dead/parked/unreachable domain,
        // Cloudflare-locked, banner-only og:image, or same-name GitHub is an
        // unrelated person) — they fall back to a generated badge. Add here if a
        // logo turns up: skytube, com.minomobi (mmopaint), xyz.atmomo,
        // blue.protopro, my.skylights, app.loghz, africa.kandake, ing.dasl,
        // space.polypod, club.feeed.
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
        // The smol ecosystem spans several domains (games, life tools, quests).
        "life.smol" | "quest.smol" => "farm.smol",
        other => other,
    }
}

/// The intermediate category a platform groups under, using a fine-grained set
/// that mirrors atstore.fyi's tags. Unknown apps land under `Explore`.
fn category_for(prefix: &str) -> &'static str {
    match prefix {
        // Social & messaging.
        "app.bsky" | "so.sprk" | "app.shadowsky" | "com.shadowsky" | "space.roomy"
        | "social.twinkl" | "social.mu" | "at.youandme" | "com.skymeetsblue" => "Social",
        // Photo & video sharing.
        "app.pinkleap" | "blue.flashes" => "Moments",
        // Games.
        "net.anisota" | "actor.rpg" | "equipment.rpg" | "farm.smol" | "blue.checkmate"
        | "blue.2048" => "Games",
        // Music, audio, podcasts.
        "app.rocksky" | "fm.teal" => "Listen",
        // Video, streaming.
        "place.stream" | "app.skytube" => "Watch",
        // Books, reviews, news, feeds, bookmarks, collections.
        "app.skyreader" | "social.popfeed" | "my.skylights" | "network.cosmik" | "com.semble"
        | "club.feeed" => "Read",
        // Writing, publishing, creative.
        "pub.leaflet" | "blog.pckt" | "pub.chive" | "site.standard" | "xyz.atmomo" => "Write",
        // Developer, code, tools, data.
        "sh.tangled" | "dev.npmx" | "com.vibe-coded" | "app.cartes" | "coop.hypha" | "ing.dasl"
        | "space.polypod" | "app.loghz" | "com.minomobi" | "blue.protopro" => "Build",
        // Work, productivity, fitness.
        "place.atwork" | "app.fitsky" | "app.sidetrail" => "Work",
        // Personal page, links, profile, identity.
        "id.sifa" | "link.woosh" | "blue.linkat" | "app.lanyards" | "blue.pronouns"
        | "computer.aetheros" | "at.marque" => "Identity",
        // Events, community, location, food.
        "events.smokesignal" | "org.atmosphereconf" | "com.atprotofans" | "africa.kandake" => {
            "Gather"
        }
        _ => "Explore",
    }
}

/// Category node ordering, so hubs appear in a stable, sensible sequence. Only
/// hubs with at least one platform are drawn (empty ones are hidden), so this
/// fine-grained set — mirroring atstore.fyi's tags — stays uncluttered per user.
pub const CATEGORY_ORDER: &[&str] = &[
    "Social",
    "Moments",
    "Games",
    "Listen",
    "Watch",
    "Read",
    "Write",
    "Build",
    "Work",
    "Identity",
    "Gather",
    "Explore",
    "Elsewhere",
];

/// The category for external (non-atproto) profile links extracted from
/// link-aggregator records. Kept distinct so the graph stays honest about which
/// presence is on atproto and which is off it.
pub const EXTERNAL_CATEGORY: &str = "Elsewhere";

// Category hub symbols: white glyphs on a transparent background, shown inside
// the node's translucent colored halo.
const SOCIAL_SYMBOL: &str = "<rect x='56' y='66' width='144' height='100' rx='26' fill='#fff'/><polygon points='96,166 96,198 128,166' fill='#fff'/>";
// Camera (covers both photo and video sharing).
const MOMENTS_SYMBOL: &str = "<rect x='48' y='94' width='160' height='94' rx='18' fill='#fff'/><rect x='96' y='76' width='48' height='22' rx='6' fill='#fff'/><circle cx='128' cy='141' r='30' fill='#2b2b36'/><circle cx='128' cy='141' r='16' fill='#fff'/>";
const GAMES_SYMBOL: &str = "<circle cx='128' cy='82' r='24' fill='#fff'/><rect x='118' y='98' width='20' height='58' rx='9' fill='#fff'/><polygon points='82,196 100,152 156,152 174,196' fill='#fff'/>";
const LISTEN_SYMBOL: &str = "<rect x='108' y='70' width='12' height='96' fill='#fff'/><rect x='176' y='56' width='12' height='96' fill='#fff'/><polygon points='108,70 188,52 188,78 108,96' fill='#fff'/><circle cx='102' cy='166' r='22' fill='#fff'/><circle cx='170' cy='152' r='22' fill='#fff'/>";
const WATCH_SYMBOL: &str = "<polygon points='104,82 104,174 182,128' fill='#fff'/>";
const READ_SYMBOL: &str = "<polygon points='122,86 60,72 60,180 122,194' fill='#fff'/><polygon points='134,86 196,72 196,180 134,194' fill='#fff'/>";
const WRITE_SYMBOL: &str = "<polygon points='158,66 190,98 104,184 72,152' fill='#fff'/><polygon points='72,152 104,184 58,196' fill='#fff'/>";
const BUILD_SYMBOL: &str = "<polygon points='98,78 118,96 86,128 118,160 98,178 48,128' fill='#fff'/><polygon points='158,78 138,96 170,128 138,160 158,178 208,128' fill='#fff'/>";
const WORK_SYMBOL: &str = "<rect x='52' y='102' width='152' height='94' rx='16' fill='#fff'/><path d='M100 102 v-14 a10 10 0 0 1 10 -10 h36 a10 10 0 0 1 10 10 v14' fill='none' stroke='#fff' stroke-width='14'/>";
const IDENTITY_SYMBOL: &str = "<circle cx='128' cy='94' r='34' fill='#fff'/><path d='M66 196 c0 -40 28 -62 62 -62 s62 22 62 62 z' fill='#fff'/>";
const GATHER_SYMBOL: &str = "<rect x='54' y='72' width='148' height='120' rx='18' fill='#fff'/><rect x='54' y='72' width='148' height='34' rx='18' fill='#c9c9d6'/><rect x='84' y='58' width='14' height='34' rx='7' fill='#fff'/><rect x='158' y='58' width='14' height='34' rx='7' fill='#fff'/>";
const EXPLORE_SYMBOL: &str = "<circle cx='128' cy='128' r='56' fill='none' stroke='#fff' stroke-width='14'/><polygon points='128,98 140,128 128,158 116,128' fill='#fff'/>";
// "Open in a new place": a box with its top-right corner opened and an arrow
// leaving through it — the universal external-link glyph.
const ELSEWHERE_SYMBOL: &str = "<path d='M118 80 H84 a20 20 0 0 0 -20 20 v72 a20 20 0 0 0 20 20 h72 a20 20 0 0 0 20 -20 v-34' fill='none' stroke='#fff' stroke-width='16' stroke-linecap='round'/><path d='M140 72 h44 v44' fill='none' stroke='#fff' stroke-width='16' stroke-linecap='round' stroke-linejoin='round'/><path d='M184 72 L120 136' fill='none' stroke='#fff' stroke-width='16' stroke-linecap='round'/>";

/// Halo color + symbol icon (SVG data URL) for a category hub node.
pub fn category_meta(name: &str) -> (&'static str, String) {
    let (color, symbol) = match name {
        "Social" => ("#e34234", SOCIAL_SYMBOL),
        "Moments" => ("#ec4899", MOMENTS_SYMBOL),
        "Games" => ("#c026d3", GAMES_SYMBOL),
        "Listen" => ("#f59e0b", LISTEN_SYMBOL),
        "Watch" => ("#ff7f50", WATCH_SYMBOL),
        "Read" => ("#14b8a6", READ_SYMBOL),
        "Write" => ("#22c55e", WRITE_SYMBOL),
        "Build" => ("#3b82f6", BUILD_SYMBOL),
        "Work" => ("#6366f1", WORK_SYMBOL),
        "Identity" => ("#8b5cf6", IDENTITY_SYMBOL),
        "Gather" => ("#06b6d4", GATHER_SYMBOL),
        "Elsewhere" => ("#64748b", ELSEWHERE_SYMBOL),
        _ => ("#9aa0b5", EXPLORE_SYMBOL),
    };
    // Bake the category name into the icon itself: the symbol scaled into the
    // upper half, the name across the lower half. Because the label is part of
    // the node's texture, it lives inside the hub bubble and is depth-sorted with
    // every other node (so it hides correctly when the hub is behind something).
    let svg = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='256' height='256' viewBox='0 0 256 256'>\
         <g transform='translate(128 92) scale(0.52) translate(-128 -128)'>{symbol}</g>\
         <text x='128' y='182' text-anchor='middle' dominant-baseline='central' \
         font-family='Space Grotesk, system-ui, sans-serif' font-size='37' font-weight='700' \
         fill='#ffffff' stroke='#111118' stroke-width='8' paint-order='stroke' \
         stroke-linejoin='round'>{name}</text></svg>"
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
}

/// Hand-curated display metadata for well-known apps: nice names and brand
/// colors. Unknown apps fall back to a derived name/color.
fn curated(prefix: &str) -> Option<Curated> {
    let c = |name, color| Curated { name, color };
    Some(match prefix {
        "app.bsky" => c("Bluesky", "#1185fe"),
        "sh.tangled" => c("Tangled", "#5b4bff"),
        "app.rocksky" => c("Rocksky", "#e0245e"),
        "app.pinkleap" => c("PinkLeap", "#ec4899"),
        "social.popfeed" => c("PopFeed", "#e8590c"),
        "pub.leaflet" => c("Leaflet", "#0f9d58"),
        "fm.teal" => c("Teal.fm", "#0d9488"),
        "events.smokesignal" => c("Smoke Signal", "#ea580c"),
        "place.stream" => c("Stream.place", "#7c3aed"),
        "id.sifa" => c("Sifa", "#2563eb"),
        // NSID reverses to semble.com (an unrelated healthcare site); the real
        // atproto app is semble.so.
        "com.semble" => c("Semble", "#f97316"),
        _ => return None,
    })
}

/// The public profile URL for this account on a given app. Where an app has a
/// per-user profile page we link straight to it; otherwise we fall back to the
/// app's homepage (`domain` is the app's reverse-DNS domain). Patterns were
/// verified per app against overby.me's live profile pages — most key on the
/// handle, PopFeed on the DID, and several apps (Leaflet, Smoke Signal, atwork,
/// Flashes, Aetheros, Spark, Cosmik, …) have no per-user web page at all.
fn profile_url(prefix: &str, domain: &str, handle: &str, did: &str) -> String {
    match prefix {
        "app.bsky" => format!("https://bsky.app/profile/{handle}"),
        "app.rocksky" => format!("https://rocksky.app/profile/{handle}"),
        "social.popfeed" => format!("https://popfeed.social/profile/{did}"),
        "sh.tangled" => format!("https://tangled.org/{handle}"),
        "dev.npmx" => format!("https://npmx.dev/profile/{handle}"),
        "place.stream" => format!("https://stream.place/{handle}"),
        "id.sifa" => format!("https://sifa.id/p/{handle}"),
        "com.semble" => format!("https://semble.so/profile/{handle}"),
        "blue.linkat" => format!("https://linkat.blue/{handle}"),
        "app.pinkleap" => format!("https://pinkleap.app/@{handle}"),
        _ => format!("https://{domain}"),
    }
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
        let (name, color) = match curated(prefix) {
            Some(c) => (c.name.to_string(), c.color.to_string()),
            None => {
                // org label is the second segment (e.g. `sifa` in `id.sifa`).
                let org = prefix.split('.').nth(1).unwrap_or(prefix);
                (title_case(org), derived_color(&domain))
            }
        };
        let profile_url = profile_url(prefix, &domain, handle, did);
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

// --- External (non-atproto) profile links ---
//
// Some atproto apps store links to a user's presence *off* atproto: Sifa's
// external accounts, Woosh/Linkat link boards, Lanyards affiliations, and plain
// URLs written into a Bluesky bio. We harvest those (in [`crate::atproto_web`]),
// then turn them into "Elsewhere" leaves here so the pipeline — dedup, filtering,
// badge generation — stays pure and unit tested.

/// How many external links to surface at most, so a huge link board can't bury
/// the atproto graph.
const MAX_EXTERNAL_LINKS: usize = 16;

/// The lowercased host of an `http(s)` URL, without a `www.` prefix, port, or
/// userinfo. Returns `None` for anything that isn't a real absolute web link, so
/// prose fragments (`he/him`, `Signal robin.77`) never become nodes.
pub fn link_host(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    // Drop any `user@` and `:port`.
    let host = authority.rsplit('@').next()?.split(':').next()?;
    let host = host.strip_prefix("www.").unwrap_or(host);
    (host.len() > 1 && host.contains('.')).then(|| host.to_lowercase())
}

/// Appview hosts that mirror atproto profiles we already draw as their own node,
/// so a link to them is redundant.
fn is_atproto_host(host: &str) -> bool {
    matches!(
        host,
        "bsky.app" | "bsky.social" | "deer.social" | "ouranos.blue" | "zeppelin.social"
    )
}

/// A bundled logo for a well-known external host, so common profile links render
/// with a real icon instead of a letter badge.
fn external_icon(host: &str) -> Option<&'static str> {
    Some(match host {
        "github.com" => "github.avif",
        "codeberg.org" => "codeberg.avif",
        "linkedin.com" => "linkedin.avif",
        "wikipedia.org" | "en.wikipedia.org" => "wikipedia.avif",
        "matrix.to" => "matrix.avif",
        "signal.me" | "signal.org" => "signal.avif",
        "lemmy.world" => "lemmy.avif",
        "happycow.net" => "happycow.avif",
        "mastodon.social" => "mastodon.avif",
        _ => return None,
    })
}

/// Tidy a link label for display: collapse whitespace and cap the length so a
/// verbose link-board title stays a legible node caption.
fn clean_label(label: &str) -> String {
    let collapsed = label.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > 32 {
        let mut s: String = collapsed.chars().take(31).collect();
        s.push('…');
        s
    } else {
        collapsed
    }
}

/// Normalize a URL to a dedup key: host + path, without scheme, `www.`, or a
/// trailing slash, lowercased.
fn url_key(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    rest.trim_end_matches('/').to_lowercase()
}

/// Build a single "Elsewhere" leaf from an external link. `label` is the
/// aggregator's caption for it (Sifa account label, Linkat card text, …); when
/// blank the host stands in. Returns `None` unless `url` is a real web link.
pub fn external_link_platform(url: &str, label: Option<&str>) -> Option<Platform> {
    let host = link_host(url)?;
    let name = label
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(clean_label)
        .unwrap_or_else(|| host.clone());
    let icon = match external_icon(&host) {
        Some(file) => Icon::Bundled(file),
        None => Icon::Badge(badge_icon(&name, &derived_color(&host))),
    };
    Some(Platform {
        name,
        domain: host,
        icon,
        profile_url: url.to_string(),
        category: EXTERNAL_CATEGORY,
    })
}

/// Turn raw `(url, label)` candidates harvested from link-aggregator records into
/// deduped "Elsewhere" leaves: drop links to atproto nodes already shown, collapse
/// duplicate URLs, keep every display name unique, and cap the count. Earlier
/// candidates win, so callers should pass labelled (structured) sources before
/// bare bio URLs.
pub fn external_platforms(
    candidates: &[(String, Option<String>)],
    detected: &[Platform],
) -> Vec<Platform> {
    use std::collections::HashSet;

    // Hosts already represented by an atproto platform node.
    let mut shown: HashSet<String> = HashSet::new();
    for p in detected {
        shown.insert(p.domain.to_lowercase());
        if let Some(h) = link_host(&p.profile_url) {
            shown.insert(h);
        }
    }
    let mut used_names: HashSet<String> = detected.iter().map(|p| p.name.to_lowercase()).collect();
    let mut seen_urls: HashSet<String> = HashSet::new();
    let mut out: Vec<Platform> = Vec::new();

    for (url, label) in candidates {
        if out.len() >= MAX_EXTERNAL_LINKS {
            break;
        }
        let Some(mut platform) = external_link_platform(url, label.as_deref()) else {
            continue;
        };
        if is_atproto_host(&platform.domain) || shown.contains(&platform.domain) {
            continue;
        }
        if !seen_urls.insert(url_key(url)) {
            continue;
        }
        // Keep the caption unique across the whole graph (node ids are names):
        // fall back to the host, then to a numbered host, on a clash.
        let mut name = platform.name.clone();
        if used_names.contains(&name.to_lowercase()) {
            name = platform.domain.clone();
        }
        let mut n = 2;
        while used_names.contains(&name.to_lowercase()) {
            name = format!("{} ({n})", platform.domain);
            n += 1;
        }
        used_names.insert(name.to_lowercase());
        platform.name = name;
        out.push(platform);
    }

    out.sort_by_key(|p| p.name.to_lowercase());
    out
}

/// Pull bare `http(s)` URLs out of free text (e.g. a Bluesky bio), trimming
/// surrounding punctuation. This is the "indirect" source: links a user only
/// wrote in prose rather than in a structured link field.
pub fn extract_urls(text: &str) -> Vec<String> {
    text.split(char::is_whitespace)
        .filter_map(|tok| {
            let tok = tok.trim_matches(|c: char| "\"'()<>[]{}|\\^`.,;:!?".contains(c));
            (tok.starts_with("http://") || tok.starts_with("https://"))
                .then(|| tok.to_string())
                .filter(|u| link_host(u).is_some())
        })
        .collect()
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
        assert_eq!(cat_of("Bluesky"), Some("Social"));
        assert_eq!(cat_of("Tangled"), Some("Build"));
        assert_eq!(cat_of("Rocksky"), Some("Listen"));
        assert_eq!(cat_of("Rpg"), Some("Games"));
        assert_eq!(cat_of("Smoke Signal"), Some("Gather"));
        assert_eq!(cat_of("Sifa"), Some("Identity"));
        assert_eq!(cat_of("PinkLeap"), Some("Moments"));
        assert_eq!(cat_of("Leaflet"), Some("Write"));
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
    fn profile_url_links_to_the_user_profile() {
        let did = "did:plc:eukcx4amfqmhfrnkix7zwm34";
        let p = |prefix, domain| profile_url(prefix, domain, "overby.me", did);
        // Verified per app against overby.me's live profile pages.
        assert_eq!(
            p("app.bsky", "bsky.app"),
            "https://bsky.app/profile/overby.me"
        );
        assert_eq!(
            p("sh.tangled", "tangled.sh"),
            "https://tangled.org/overby.me"
        );
        assert_eq!(
            p("dev.npmx", "npmx.dev"),
            "https://npmx.dev/profile/overby.me"
        );
        assert_eq!(
            p("place.stream", "stream.place"),
            "https://stream.place/overby.me"
        );
        assert_eq!(p("id.sifa", "sifa.id"), "https://sifa.id/p/overby.me");
        assert_eq!(
            p("com.semble", "semble.so"),
            "https://semble.so/profile/overby.me"
        );
        assert_eq!(
            p("blue.linkat", "linkat.blue"),
            "https://linkat.blue/overby.me"
        );
        assert_eq!(
            p("social.popfeed", "popfeed.social"),
            "https://popfeed.social/profile/did:plc:eukcx4amfqmhfrnkix7zwm34"
        );
        // Apps with no per-user web page fall back to the homepage.
        assert_eq!(p("pub.leaflet", "leaflet.pub"), "https://leaflet.pub");
        assert_eq!(
            p("computer.aetheros", "aetheros.computer"),
            "https://aetheros.computer"
        );
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

    #[test]
    fn link_host_extracts_and_rejects() {
        // Path stripped.
        assert_eq!(
            link_host("https://github.com/darobin/").as_deref(),
            Some("github.com")
        );
        // `www.` dropped and host lowercased.
        assert_eq!(
            link_host("https://www.GitHub.com/").as_deref(),
            Some("github.com")
        );
        // Port stripped.
        assert_eq!(
            link_host("https://bsky.app:443/profile/robin").as_deref(),
            Some("bsky.app")
        );
        // Userinfo, query, and fragment stripped.
        assert_eq!(
            link_host("https://user@orcid.org/0000-1?x=1#f").as_deref(),
            Some("orcid.org")
        );
        // Not absolute web links.
        assert_eq!(link_host("he/him"), None);
        assert_eq!(link_host("example.com"), None); // bare host, no scheme
        assert_eq!(link_host("Signal robin.77"), None);
    }

    #[test]
    fn extract_urls_pulls_links_from_prose() {
        let bio = "Political technologist.\n• tech, cats\n• https://berjon.com/ \n• he/him\n• Signal robin.77";
        assert_eq!(extract_urls(bio), vec!["https://berjon.com/"]);
        // Trailing sentence punctuation is trimmed; bare domains are ignored.
        assert_eq!(
            extract_urls("see https://orcid.org/0000-1, and example.com too"),
            vec!["https://orcid.org/0000-1"]
        );
    }

    #[test]
    fn external_link_platform_picks_icon_and_category() {
        let gh = external_link_platform("https://github.com/darobin/", Some("GitHub")).unwrap();
        assert_eq!(gh.name, "GitHub");
        assert_eq!(gh.domain, "github.com");
        assert_eq!(gh.icon, Icon::Bundled("github.avif"));
        assert_eq!(gh.category, "Elsewhere");
        assert_eq!(gh.profile_url, "https://github.com/darobin/");

        // Unknown host, no label -> host caption + generated badge.
        let orcid = external_link_platform("https://orcid.org/0000-1", None).unwrap();
        assert_eq!(orcid.name, "orcid.org");
        assert!(matches!(orcid.icon, Icon::Badge(_)));

        assert!(external_link_platform("not a url", None).is_none());
    }

    fn platform(name: &str, domain: &str, profile_url: &str) -> Platform {
        Platform {
            name: name.to_string(),
            domain: domain.to_string(),
            icon: Icon::Badge(badge_icon(name, "#000000")),
            profile_url: profile_url.to_string(),
            category: "Social",
        }
    }

    #[test]
    fn external_platforms_dedupes_filters_and_renames() {
        let detected = [
            platform("Bluesky", "bsky.app", "https://bsky.app/profile/robin"),
            platform(
                "Rocksky",
                "rocksky.app",
                "https://rocksky.app/profile/robin",
            ),
        ];
        let candidates = vec![
            (
                "https://github.com/darobin/".to_string(),
                Some("GitHub".to_string()),
            ),
            (
                "https://orcid.org/0000-1".to_string(),
                Some("ORCID".to_string()),
            ),
            ("https://berjon.com/".to_string(), Some("Blog".to_string())),
            // Redundant: an appview host and an already-detected platform host.
            (
                "https://bsky.app/profile/robin".to_string(),
                Some("Bluesky".to_string()),
            ),
            (
                "https://rocksky.app/profile/robin".to_string(),
                Some("Music".to_string()),
            ),
            // Duplicate URL (trailing slash / case) collapses.
            (
                "https://GitHub.com/darobin".to_string(),
                Some("gh".to_string()),
            ),
        ];
        let ext = external_platforms(&candidates, &detected);
        let names: Vec<_> = ext.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["Blog", "GitHub", "ORCID"]);
        assert!(ext.iter().all(|p| p.category == "Elsewhere"));
        assert!(
            !ext.iter()
                .any(|p| p.domain == "bsky.app" || p.domain == "rocksky.app")
        );
    }

    #[test]
    fn external_platforms_disambiguates_duplicate_labels() {
        let candidates = vec![
            (
                "https://orcid.org/0000-1".to_string(),
                Some("Website".to_string()),
            ),
            (
                "https://berjon.com/".to_string(),
                Some("Website".to_string()),
            ),
        ];
        let ext = external_platforms(&candidates, &[]);
        // First keeps the label; the clash falls back to the host.
        let names: Vec<_> = ext.iter().map(|p| p.name.clone()).collect();
        assert!(names.contains(&"Website".to_string()), "{names:?}");
        assert!(names.contains(&"berjon.com".to_string()), "{names:?}");
        assert_eq!(ext.len(), 2);
    }

    #[test]
    fn external_platforms_caps_the_count() {
        let candidates: Vec<_> = (0..40)
            .map(|i| (format!("https://example.com/?n={i}"), None))
            .collect();
        assert_eq!(
            external_platforms(&candidates, &[]).len(),
            MAX_EXTERNAL_LINKS
        );
    }
}
