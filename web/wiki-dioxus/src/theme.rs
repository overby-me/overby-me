use dioxus::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub enum ThemeMode {
    Light,
    Dark,
}

impl ThemeMode {
    pub fn toggle(&self) -> Self {
        match self {
            ThemeMode::Light => ThemeMode::Dark,
            ThemeMode::Dark => ThemeMode::Light,
        }
    }

    pub fn data_attr(&self) -> &'static str {
        match self {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }

    fn from_attr(s: &str) -> Self {
        match s {
            "dark" => ThemeMode::Dark,
            _ => ThemeMode::Light,
        }
    }
}

pub static THEME: GlobalSignal<ThemeMode> = Signal::global(|| ThemeMode::Light);

pub fn use_theme() -> Signal<ThemeMode> {
    THEME.signal()
}

/// Apply theme to the document element, and sync the browser chrome colour
/// (`<meta name="theme-color">`) to the active scheme's primary so the mobile
/// address bar / status bar matches light vs dark (it was a static value).
pub fn apply_theme(mode: &ThemeMode) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(doc) = window.document() else {
        return;
    };
    if let Some(el) = doc.document_element() {
        let _ = el.set_attribute("data-theme", mode.data_attr());
        // Read the now-active --md-primary and push it to the theme-color meta.
        if let Ok(Some(style)) = window.get_computed_style(&el) {
            let color = style
                .get_property_value("--md-primary")
                .unwrap_or_default()
                .trim()
                .to_string();
            if !color.is_empty() {
                if let Some(meta) = doc
                    .query_selector("meta[name=\"theme-color\"]")
                    .ok()
                    .flatten()
                {
                    let _ = meta.set_attribute("content", &color);
                }
            }
        }
    }
}

/// Persist the chosen theme so it survives a reload.
pub fn save_theme(mode: &ThemeMode) {
    if let Some(storage) = local_storage() {
        let _ = storage.set_item("wiki_theme", mode.data_attr());
    }
}

/// Load the persisted theme (defaults to Light) into the global signal.
pub fn load_theme() {
    if let Some(storage) = local_storage() {
        if let Ok(Some(v)) = storage.get_item("wiki_theme") {
            *THEME.write() = ThemeMode::from_attr(&v);
        }
    }
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}
