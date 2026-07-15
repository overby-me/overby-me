use crate::graph::texture::icon_url;

#[derive(Clone, PartialEq)]
pub struct GraphNode {
    pub id: String,
    pub desc: String,
    /// Fully resolved image URL for the node icon (a bundled asset URL for the
    /// personal graph, or a remote avatar/favicon URL for atproto graphs).
    pub icon: String,
    pub color: Option<String>,
    pub opacity: Option<f32>,
    pub url: Option<String>,
    /// The large hub node the layout centers on (drawn bigger, easier to grab).
    pub center: bool,
    /// A category hub: in a collapsible graph, tapping it expands/collapses the
    /// platform leaves linked under it.
    pub hub: bool,
}

#[derive(Clone, PartialEq)]
pub struct GraphLink {
    pub source: String,
    pub target: String,
}

/// A complete graph: the nodes and the links between them. Both the curated
/// personal homepage and the auto-generated atproto graphs are just different
/// `GraphData` values fed to the same simulation/renderer.
#[derive(Clone, PartialEq, Default)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub links: Vec<GraphLink>,
    /// Clip icon sprites to a circle (used by the atproto graphs, whose badges
    /// and avatars are square textures). The personal graph leaves this off.
    pub circular_icons: bool,
    /// Start collapsed (center + hubs only) and reveal a hub's leaves on tap.
    /// Used by the atproto graphs; the personal graph shows everything at once.
    pub collapsible: bool,
}

impl GraphData {
    pub fn node_index(&self, id: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.id == id)
    }

    /// The hand-curated graph shown on the homepage root (`/`).
    pub fn personal() -> Self {
        // Small builders keep the node/link tables readable.
        fn node(
            id: &str,
            desc: &str,
            icon: &str,
            color: Option<&str>,
            opacity: Option<f32>,
            url: Option<&str>,
            center: bool,
        ) -> GraphNode {
            GraphNode {
                id: id.to_string(),
                desc: desc.to_string(),
                icon: icon_url(icon),
                color: color.map(str::to_string),
                opacity,
                url: url.map(str::to_string),
                center,
                hub: false,
            }
        }
        fn link(source: &str, target: &str) -> GraphLink {
            GraphLink {
                source: source.to_string(),
                target: target.to_string(),
            }
        }

        let nodes = vec![
            node(
                "Niclas Overby",
                "Niclas Overby Ⓝ",
                "me.avif",
                None,
                None,
                None,
                true,
            ),
            node(
                "Commerce",
                "Commerce",
                "commerce.avif",
                Some("#45b1e8"),
                None,
                None,
                false,
            ),
            node(
                "Improve",
                "Improve",
                "improve.avif",
                Some("#7fff00"),
                None,
                None,
                false,
            ),
            node(
                "Connect",
                "Connect",
                "connect.avif",
                Some("#e34234"),
                None,
                None,
                false,
            ),
            node(
                "Immerse",
                "Immerse",
                "immerse.avif",
                Some("#ff7f50"),
                None,
                None,
                false,
            ),
            node(
                "Give",
                "Give",
                "give.avif",
                Some("#6a5acd"),
                None,
                None,
                false,
            ),
            node(
                "LinkedIn",
                "LinkedIn\nProfile",
                "linkedin.avif",
                None,
                None,
                Some("https://www.linkedin.com/in/niclasoverby"),
                false,
            ),
            node(
                "PinkLeap",
                "PinkLeap\nProfile",
                "pinkleap.avif",
                None,
                None,
                Some("https://pinkleap.app/@overby.me"),
                false,
            ),
            node(
                "Mail",
                "Send Mail",
                "mail.avif",
                None,
                None,
                Some("mailto:niclas@overby.me"),
                false,
            ),
            node(
                "Matrix",
                "Matrix\nProfile",
                "matrix.avif",
                None,
                None,
                Some("https://matrix.to/#/@niclas:overby.me"),
                false,
            ),
            node(
                "Signal",
                "Signal\nProfile",
                "signal.avif",
                None,
                None,
                Some(
                    "https://signal.me/#eu/BKjgrHvQhqgDPpy9p2VfcfVj6yx0mJtVGOX8GQ_2htxhX7cDxhREVad8oWL1qAMj",
                ),
                false,
            ),
            node(
                "Rocksky",
                "Rocksky\nProfile",
                "rocksky.avif",
                None,
                None,
                Some("https://rocksky.app/profile/overby.me"),
                false,
            ),
            node(
                "GitHub",
                "GitHub\nProfile",
                "github.avif",
                None,
                None,
                Some("https://github.com/overby-me"),
                false,
            ),
            node(
                "Codeberg",
                "Codeberg\nProfile",
                "codeberg.avif",
                None,
                None,
                Some("https://codeberg.org/overby-me"),
                false,
            ),
            node(
                "Tangled",
                "Tangled\nProfile",
                "tangled.avif",
                None,
                None,
                Some("https://tangled.org/@overby.me"),
                false,
            ),
            node(
                "Bridgy Fed",
                "Bridgy Fed\nProfile",
                "bridgy.avif",
                None,
                None,
                Some("https://mastodon.social/@overby.me@bsky.brid.gy"),
                false,
            ),
            node(
                "Mastodon",
                "Mastodon\nProfile",
                "mastodon.avif",
                None,
                None,
                Some("https://mas.to/@overby.me@bsky.brid.gy"),
                false,
            ),
            node(
                "Bluesky",
                "Bluesky\nProfile",
                "bluesky.avif",
                None,
                None,
                Some("https://bsky.app/profile/overby.me"),
                false,
            ),
            node(
                "Radikale Venstre",
                "Radikale Venstre\n(Political Effort)",
                "radikale.avif",
                None,
                None,
                Some("https://www.radikale.dk"),
                false,
            ),
            node(
                "Aivero",
                "Aivero\n(Ex-company)",
                "aivero.avif",
                None,
                None,
                Some("https://www.aivero.com"),
                false,
            ),
            node(
                "Factbird",
                "Factbird\n(Ex-company)",
                "factbird.avif",
                None,
                None,
                Some("https://www.factbird.com"),
                false,
            ),
            node(
                "Veo",
                "Veo\n(Commercial Effort)",
                "veo.avif",
                None,
                None,
                Some("https://www.veo.co"),
                false,
            ),
            node(
                "Wikipedia",
                "Wikipedia\nProfile",
                "wikipedia.avif",
                None,
                None,
                Some("https://en.wikipedia.org/wiki/User:Niclas_Overby"),
                false,
            ),
            node(
                "HappyCow",
                "HappyCow\nProfile",
                "happycow.avif",
                None,
                None,
                Some("https://www.happycow.net/members/profile/niclasoverby"),
                false,
            ),
            node(
                "Lemmy",
                "Lemmy\nProfile",
                "lemmy.avif",
                None,
                None,
                Some("https://lemmy.world/u/noverby"),
                false,
            ),
            node(
                "PopFeed",
                "PopFeed\nProfile",
                "popfeed.avif",
                None,
                None,
                Some("https://popfeed.social/profile/did:plc:eukcx4amfqmhfrnkix7zwm34"),
                false,
            ),
        ];

        let links = vec![
            link("Niclas Overby", "Commerce"),
            link("Niclas Overby", "Improve"),
            link("Niclas Overby", "Connect"),
            link("Niclas Overby", "Immerse"),
            link("Niclas Overby", "Give"),
            link("Connect", "Mail"),
            link("Connect", "Matrix"),
            link("Connect", "LinkedIn"),
            link("Connect", "Bridgy Fed"),
            link("Connect", "Mastodon"),
            link("Connect", "PinkLeap"),
            link("Connect", "Bluesky"),
            link("Connect", "Signal"),
            link("Commerce", "LinkedIn"),
            link("Commerce", "Aivero"),
            link("Commerce", "Factbird"),
            link("Commerce", "Veo"),
            link("Commerce", "GitHub"),
            link("Immerse", "PinkLeap"),
            link("Immerse", "Rocksky"),
            link("Immerse", "PopFeed"),
            link("Immerse", "Wikipedia"),
            link("Immerse", "HappyCow"),
            link("Immerse", "Lemmy"),
            link("Give", "Wikipedia"),
            link("Give", "Codeberg"),
            link("Give", "Tangled"),
            link("Give", "Radikale Venstre"),
            link("Give", "HappyCow"),
            link("Improve", "Codeberg"),
            link("Improve", "Tangled"),
            link("Improve", "PopFeed"),
        ];

        GraphData {
            nodes,
            links,
            circular_icons: false,
            collapsible: false,
        }
    }
}
