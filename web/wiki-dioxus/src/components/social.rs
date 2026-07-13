use dioxus::prelude::*;

use crate::graphql::NodeWithChildren;
use crate::i18n::t;

use super::loader::icon_el;

/// SocialApp — a live social wall backed by **Bluesky only** (#137). It searches
/// the public Bluesky AppView for posts matching a term (the context's
/// configured `data.bluesky` tag, else its name) and renders them. Mastodon /
/// PixelFed are intentionally out of scope.
#[component]
pub fn SocialApp(node: NodeWithChildren) -> Element {
    let default_query = node
        .data
        .as_ref()
        .and_then(|d| d.0.get("bluesky"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| node.name.clone());

    let mut query = use_signal(|| default_query.clone());

    let posts = crate::use_data_resource!(move || {
        let q = query.read().clone();
        async move { fetch_bluesky(&q).await }
    });

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", {icon_el("app/social")} }
                div {
                    h3 { class: "title-medium", "{node.name}" }
                    p {
                        class: "body-small",
                        class: "text-muted",
                        "{t(\"social.poweredBy\")}"
                    }
                }
            }
            div { class: "card-content",
                div { class: "text-field", style: "margin: 0;",
                    label { "{t(\"social.query\")}" }
                    input {
                        r#type: "text",
                        value: "{query}",
                        oninput: move |e| query.set(e.value()),
                    }
                }
            }
        }

        match &*posts.read() {
            Some(Ok(list)) if !list.is_empty() => rsx! {
                div { class: "stack stack-v",
                    for (i , post) in list.iter().enumerate() {
                        PostCard { key: "{i}", post: post.clone() }
                    }
                }
            },
            Some(Ok(_)) => rsx! {
                // EXPERIMENT: orb empty state for a search with no results.
                div { class: "card",
                    div { class: "empty-state empty-state-sm",
                        div { class: "empty-state-orb empty-state-orb-sm",
                            span { class: "material-icons", "search_off" }
                        }
                        p { class: "empty-state-body", "{t(\"social.empty\")}" }
                    }
                }
            },
            Some(Err(e)) => rsx! {
                // EXPERIMENT: expressive error state (error-tinted orb).
                div { class: "card accent-error",
                    div { class: "empty-state empty-state-sm",
                        div { class: "empty-state-orb empty-state-orb-sm error-orb",
                            span { class: "material-icons", "error_outline" }
                        }
                        h3 { class: "empty-state-title", "{t(\"error.somethingWentWrong\")}" }
                        pre { class: "error-fallback", "{e}" }
                    }
                }
            },
            None => rsx! { super::widgets::Spinner {} },
        }
    }
}

#[component]
fn PostCard(post: Post) -> Element {
    rsx! {
        div { class: "card",
            div { class: "card-content",
                a {
                    class: "post-header post-author-link",
                    href: "https://bsky.app/profile/{post.handle}",
                    target: "_blank",
                    rel: "noopener noreferrer",
                    if let Some(avatar) = post.avatar.clone() {
                        img {
                            src: "{avatar}",
                            class: "post-avatar",
                            alt: "",
                            loading: "lazy",
                            decoding: "async",
                            referrerpolicy: "no-referrer",
                        }
                    } else {
                        div { class: "avatar small", span { class: "material-icons", "public" } }
                    }
                    div {
                        div { class: "list-item-primary", "{post.display_name}" }
                        div {
                            class: "list-item-secondary",
                            class: "text-muted",
                            "@{post.handle}"
                        }
                    }
                }
                p { class: "post-text body-medium mt-1", "{post.text}" }
            }
        }
    }
}

/// One Bluesky post, flattened to the fields the wall shows.
#[derive(Clone, PartialEq)]
struct Post {
    display_name: String,
    handle: String,
    avatar: Option<String>,
    text: String,
}

/// Search the public Bluesky AppView for posts matching `query`. No auth is
/// needed — `public.api.bsky.app` serves the app.bsky.* read APIs with CORS.
async fn fetch_bluesky(query: &str) -> Result<Vec<Post>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let resp = reqwest::Client::new()
        .get("https://public.api.bsky.app/xrpc/app.bsky.feed.searchPosts")
        .query(&[("q", query), ("limit", "25")])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    Ok(parse_posts(&json))
}

/// Flatten a `searchPosts` response into `Post`s. Pure so it can be unit-tested.
fn parse_posts(json: &serde_json::Value) -> Vec<Post> {
    let Some(posts) = json.get("posts").and_then(|p| p.as_array()) else {
        return Vec::new();
    };
    posts
        .iter()
        .filter_map(|p| {
            let author = p.get("author")?;
            let handle = author.get("handle").and_then(|h| h.as_str())?.to_string();
            let display_name = author
                .get("displayName")
                .and_then(|d| d.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(&handle)
                .to_string();
            let avatar = author
                .get("avatar")
                .and_then(|a| a.as_str())
                .map(|s| s.to_string());
            let text = p
                .get("record")
                .and_then(|r| r.get("text"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            Some(Post {
                display_name,
                handle,
                avatar,
                text,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_posts_flattens_author_and_record() {
        let json = serde_json::json!({
            "posts": [
                {
                    "author": { "handle": "a.bsky.social", "displayName": "Alice", "avatar": "https://github.com/example/a.jpg" },
                    "record": { "text": "hello wall" }
                },
                {
                    "author": { "handle": "b.bsky.social" },
                    "record": { "text": "no name here" }
                }
            ]
        });
        let posts = parse_posts(&json);
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0].display_name, "Alice");
        assert_eq!(posts[0].text, "hello wall");
        // Missing displayName falls back to the handle.
        assert_eq!(posts[1].display_name, "b.bsky.social");
        assert!(posts[1].avatar.is_none());
        // Missing "posts" → empty.
        assert!(parse_posts(&serde_json::json!({})).is_empty());
    }
}
