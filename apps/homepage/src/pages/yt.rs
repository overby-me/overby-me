use dioxus::prelude::*;

#[component]
pub fn Yt(url: String) -> Element {
    let video_id = extract_video_id(&url);

    match video_id {
        Some(id) => rsx! {
            div {
                style: "position: relative; overflow: hidden; width: 100%; height: 100vh;",
                iframe {
                    title: "YouTube iframe",
                    width: "100%",
                    height: "100%",
                    src: "https://youtube.com/embed/{id}?enablejsapi=1",
                    r#"frameborder"#: "0",
                    allow: "accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture",
                    allowfullscreen: true,
                }
            }
        },
        None => rsx! {},
    }
}

fn extract_video_id(url: &str) -> Option<String> {
    let idx = url.find("v=")?;
    let rest = &url[idx + 2..];
    if rest.len() >= 11
        && rest[..11]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        Some(rest[..11].to_string())
    } else {
        None
    }
}
