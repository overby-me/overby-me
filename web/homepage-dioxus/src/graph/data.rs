pub struct GraphNode {
    pub id: &'static str,
    pub desc: &'static str,
    pub icon: &'static str,
    pub color: Option<&'static str>,
    pub opacity: Option<f32>,
    pub url: Option<&'static str>,
}

pub struct GraphLink {
    pub source: &'static str,
    pub target: &'static str,
}

pub static NODES: &[GraphNode] = &[
    GraphNode {
        id: "Niclas Overby",
        desc: "Niclas Overby Ⓝ",
        icon: "me.avif",
        color: None,
        opacity: None,
        url: None,
    },
    GraphNode {
        id: "Commerce",
        desc: "Commerce",
        icon: "commerce.avif",
        color: Some("#45b1e8"),
        opacity: None,
        url: None,
    },
    GraphNode {
        id: "Improve",
        desc: "Improve",
        icon: "improve.avif",
        color: Some("#7fff00"),
        opacity: None,
        url: None,
    },
    GraphNode {
        id: "Connect",
        desc: "Connect",
        icon: "connect.avif",
        color: Some("#e34234"),
        opacity: None,
        url: None,
    },
    GraphNode {
        id: "Immerse",
        desc: "Immerse",
        icon: "immerse.avif",
        color: Some("#ff7f50"),
        opacity: None,
        url: None,
    },
    GraphNode {
        id: "Give",
        desc: "Give",
        icon: "give.avif",
        color: Some("#6a5acd"),
        opacity: None,
        url: None,
    },
    GraphNode {
        id: "Fediverse",
        desc: "Fediverse\nInfo",
        icon: "fediverse.avif",
        color: Some("#000000"),
        opacity: None,
        url: Some("https://fediverse.info"),
    },
    GraphNode {
        id: "LinkedIn",
        desc: "LinkedIn\nProfile",
        icon: "linkedin.avif",
        color: None,
        opacity: None,
        url: Some("https://www.linkedin.com/in/niclasoverby"),
    },
    GraphNode {
        id: "PixelFed",
        desc: "PixelFed\nProfile",
        icon: "pixelfed.avif",
        color: None,
        opacity: None,
        url: Some("https://pixelfed.social/niclasoverby"),
    },
    GraphNode {
        id: "Mail",
        desc: "Send Mail",
        icon: "mail.avif",
        color: None,
        opacity: None,
        url: Some("mailto:niclas@overby.me"),
    },
    GraphNode {
        id: "Matrix",
        desc: "Matrix\nProfile",
        icon: "matrix.avif",
        color: None,
        opacity: None,
        url: Some("https://matrix.to/#/@niclas:overby.me"),
    },
    GraphNode {
        id: "Signal",
        desc: "Signal\nProfile",
        icon: "signal.avif",
        color: None,
        opacity: None,
        url: Some(
            "https://signal.me/#eu/BKjgrHvQhqgDPpy9p2VfcfVj6yx0mJtVGOX8GQ_2htxhX7cDxhREVad8oWL1qAMj",
        ),
    },
    GraphNode {
        id: "Rocksky",
        desc: "Rocksky\nProfile",
        icon: "rocksky.avif",
        color: None,
        opacity: None,
        url: Some("https://rocksky.app/profile/overby.me"),
    },
    GraphNode {
        id: "Atmosphere",
        desc: "Atmosphere",
        icon: "atmosphere.avif",
        color: Some("#00ffff"),
        opacity: Some(0.1),
        url: Some("https://atproto.com/"),
    },
    GraphNode {
        id: "Bridgy",
        desc: "Bridgy Fed",
        icon: "bridgy.avif",
        color: Some("#ffffff"),
        opacity: Some(0.1),
        url: Some("https://fed.brid.gy"),
    },
    GraphNode {
        id: "GitHub",
        desc: "GitHub\nProfile",
        icon: "github.avif",
        color: None,
        opacity: None,
        url: Some("https://github.com/overby-me"),
    },
    GraphNode {
        id: "Codeberg",
        desc: "Codeberg\nProfile",
        icon: "codeberg.avif",
        color: None,
        opacity: None,
        url: Some("https://codeberg.org/overby-me"),
    },
    GraphNode {
        id: "Tangled",
        desc: "Tangled\nProfile",
        icon: "tangled.avif",
        color: None,
        opacity: None,
        url: Some("https://tangled.org/@overby.me"),
    },
    GraphNode {
        id: "Mastodon",
        desc: "Mastodon\nProfile",
        icon: "mastodon.avif",
        color: None,
        opacity: None,
        url: Some("https://mas.to/@niclasoverby"),
    },
    GraphNode {
        id: "Bluesky",
        desc: "Bluesky\nProfile",
        icon: "bluesky.avif",
        color: None,
        opacity: None,
        url: Some("https://bsky.app/profile/overby.me"),
    },
    GraphNode {
        id: "Radikale Venstre",
        desc: "Radikale Venstre\n(Political Effort)",
        icon: "radikale.avif",
        color: None,
        opacity: None,
        url: Some("https://www.radikale.dk"),
    },
    GraphNode {
        id: "Aivero",
        desc: "Aivero\n(Ex-company)",
        icon: "aivero.avif",
        color: None,
        opacity: None,
        url: Some("https://www.aivero.com"),
    },
    GraphNode {
        id: "Factbird",
        desc: "Factbird\n(Ex-company)",
        icon: "factbird.avif",
        color: None,
        opacity: None,
        url: Some("https://www.factbird.com"),
    },
    GraphNode {
        id: "Veo",
        desc: "Veo\n(Commercial Effort)",
        icon: "veo.avif",
        color: None,
        opacity: None,
        url: Some("https://www.veo.co"),
    },
    GraphNode {
        id: "Wikipedia",
        desc: "Wikipedia\nProfile",
        icon: "wikipedia.avif",
        color: None,
        opacity: None,
        url: Some("https://en.wikipedia.org/wiki/User:Niclas_Overby"),
    },
    GraphNode {
        id: "HappyCow",
        desc: "HappyCow\nProfile",
        icon: "happycow.avif",
        color: None,
        opacity: None,
        url: Some("https://www.happycow.net/members/profile/niclasoverby"),
    },
    GraphNode {
        id: "Lemmy",
        desc: "Lemmy\nProfile",
        icon: "lemmy.avif",
        color: None,
        opacity: None,
        url: Some("https://lemmy.world/u/noverby"),
    },
    GraphNode {
        id: "NeoDB",
        desc: "NeoDB\nProfile",
        icon: "neodb.avif",
        color: None,
        opacity: None,
        url: Some("https://neodb.social/users/niclasoverby"),
    },
];

pub static LINKS: &[GraphLink] = &[
    GraphLink {
        source: "Niclas Overby",
        target: "Commerce",
    },
    GraphLink {
        source: "Niclas Overby",
        target: "Improve",
    },
    GraphLink {
        source: "Niclas Overby",
        target: "Connect",
    },
    GraphLink {
        source: "Niclas Overby",
        target: "Immerse",
    },
    GraphLink {
        source: "Niclas Overby",
        target: "Give",
    },
    GraphLink {
        source: "Connect",
        target: "Mail",
    },
    GraphLink {
        source: "Connect",
        target: "Matrix",
    },
    GraphLink {
        source: "Connect",
        target: "LinkedIn",
    },
    GraphLink {
        source: "Connect",
        target: "Mastodon",
    },
    GraphLink {
        source: "Connect",
        target: "PixelFed",
    },
    GraphLink {
        source: "Connect",
        target: "Bluesky",
    },
    GraphLink {
        source: "Connect",
        target: "Signal",
    },
    GraphLink {
        source: "Commerce",
        target: "LinkedIn",
    },
    GraphLink {
        source: "Commerce",
        target: "Aivero",
    },
    GraphLink {
        source: "Commerce",
        target: "Factbird",
    },
    GraphLink {
        source: "Commerce",
        target: "Veo",
    },
    GraphLink {
        source: "Commerce",
        target: "GitHub",
    },
    GraphLink {
        source: "Immerse",
        target: "PixelFed",
    },
    GraphLink {
        source: "Immerse",
        target: "Rocksky",
    },
    GraphLink {
        source: "Immerse",
        target: "NeoDB",
    },
    GraphLink {
        source: "Immerse",
        target: "Wikipedia",
    },
    GraphLink {
        source: "Immerse",
        target: "HappyCow",
    },
    GraphLink {
        source: "Immerse",
        target: "Lemmy",
    },
    GraphLink {
        source: "Give",
        target: "Wikipedia",
    },
    GraphLink {
        source: "Give",
        target: "Codeberg",
    },
    GraphLink {
        source: "Give",
        target: "Tangled",
    },
    GraphLink {
        source: "Give",
        target: "Radikale Venstre",
    },
    GraphLink {
        source: "Give",
        target: "HappyCow",
    },
    GraphLink {
        source: "Improve",
        target: "Codeberg",
    },
    GraphLink {
        source: "Improve",
        target: "Tangled",
    },
    GraphLink {
        source: "Improve",
        target: "NeoDB",
    },
    GraphLink {
        source: "Bluesky",
        target: "Atmosphere",
    },
    GraphLink {
        source: "Tangled",
        target: "Atmosphere",
    },
    GraphLink {
        source: "Rocksky",
        target: "Atmosphere",
    },
    GraphLink {
        source: "Bridgy",
        target: "Atmosphere",
    },
    GraphLink {
        source: "Atmosphere",
        target: "Bridgy",
    },
    GraphLink {
        source: "Fediverse",
        target: "Bridgy",
    },
    GraphLink {
        source: "Bridgy",
        target: "Fediverse",
    },
    GraphLink {
        source: "PixelFed",
        target: "Fediverse",
    },
    GraphLink {
        source: "Mastodon",
        target: "Fediverse",
    },
    GraphLink {
        source: "Lemmy",
        target: "Fediverse",
    },
    GraphLink {
        source: "NeoDB",
        target: "Fediverse",
    },
];
