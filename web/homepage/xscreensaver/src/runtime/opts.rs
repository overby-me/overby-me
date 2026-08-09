//! The resource database and the option descriptors that drive the UI panel.
//!
//! Upstream a hack reads its settings from the X resource database with
//! `get_integer_resource (dpy, "delay", "Integer")`. The values come from three
//! places: the hack's own `NAME_defaults[]` array, the user's `.Xresources`,
//! and the command line, which `xscreensaver-settings` builds from the hack's
//! `hacks/config/NAME.xml`.
//!
//! Here the same three layers survive, in the same precedence order:
//!
//! 1. the hack's `defaults` array, copied verbatim from the C,
//! 2. the defaults declared by the XML-derived [`Opt`] list,
//! 3. the URL query, which is what the options panel writes.
//!
//! Only layer 3 is user-visible, so only layer 3 goes back into the URL: an
//! option left alone is left out, which keeps a shared link short.
//!
//! Hacks read their resources once, in `init`. Changing an option therefore
//! restarts the hack rather than being picked up live; that is what upstream
//! does when you press Apply, and it avoids every hack having to grow a
//! reconfigure path it does not have in C either.

use std::collections::BTreeMap;

use super::color::{BLACK, Pixel, WHITE, parse_color};

/// One entry of a `<select>`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectItem {
    pub value: &'static str,
    pub label: &'static str,
}

/// What kind of control an option gets in the panel. Mirrors the element types
/// used by `hacks/config/*.xml`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OptKind {
    /// `<number type="slider">`
    Slider {
        low: f64,
        high: f64,
        step: f64,
        decimals: u8,
        /// The XML's `convert="invert"`: the slider runs the other way from the
        /// value. Every `delay` knob is like this, because a bigger delay is a
        /// lower frame rate and the label says "Frame rate".
        invert: bool,
    },
    /// `<number type="spinbutton">`
    Spin { low: f64, high: f64 },
    /// `<boolean>`
    Bool,
    /// `<select>`
    Select(&'static [SelectItem]),
}

/// One configurable knob.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Opt {
    /// The resource name the hack reads, and the query-string key.
    pub key: &'static str,
    /// The label from the XML, already stripped of its `_` gettext marker.
    pub label: &'static str,
    pub kind: OptKind,
    /// The default as a string, so it compares directly with a resource value.
    pub default: &'static str,
}

impl Opt {
    pub const fn slider(
        key: &'static str,
        label: &'static str,
        low: f64,
        high: f64,
        step: f64,
        decimals: u8,
        default: &'static str,
    ) -> Self {
        Self {
            key,
            label,
            kind: OptKind::Slider {
                low,
                high,
                step,
                decimals,
                invert: false,
            },
            default,
        }
    }

    /// Mark a slider as running backwards (the XML's `convert="invert"`).
    pub const fn inverted(self) -> Self {
        let kind = match self.kind {
            OptKind::Slider {
                low,
                high,
                step,
                decimals,
                ..
            } => OptKind::Slider {
                low,
                high,
                step,
                decimals,
                invert: true,
            },
            other => other,
        };
        Self {
            key: self.key,
            label: self.label,
            kind,
            default: self.default,
        }
    }

    pub const fn boolean(key: &'static str, label: &'static str, default: &'static str) -> Self {
        Self {
            key,
            label,
            kind: OptKind::Bool,
            default,
        }
    }

    pub const fn select(
        key: &'static str,
        label: &'static str,
        items: &'static [SelectItem],
        default: &'static str,
    ) -> Self {
        Self {
            key,
            label,
            kind: OptKind::Select(items),
            default,
        }
    }
}

/// The resolved resource database for one running hack.
#[derive(Clone, Debug, Default)]
pub struct Resources {
    /// Everything the hack can see, keyed by resource name.
    values: BTreeMap<String, String>,
    /// Just the layer the panel owns, so it can be written back to the URL.
    overrides: BTreeMap<String, String>,
}

impl Resources {
    /// Build the database from the three layers.
    pub fn new(defaults: &[&str], opts: &[Opt], query: &str) -> Self {
        let mut values = BTreeMap::new();

        // Layer 1: the hack's own defaults, in C form (`"*delay:  10000"`).
        for entry in defaults {
            if let Some((k, v)) = entry.split_once(':') {
                let k = k.trim().trim_start_matches(['.', '*']);
                if !k.is_empty() {
                    values.insert(k.to_string(), v.trim().to_string());
                }
            }
        }

        // Layer 2: the XML defaults.
        for o in opts {
            values.insert(o.key.to_string(), o.default.to_string());
        }

        let mut res = Self {
            values,
            overrides: BTreeMap::new(),
        };

        // Layer 3: the query string, restricted to declared options so a
        // hand-edited URL cannot inject arbitrary resources.
        for (k, v) in parse_query(query) {
            if opts.iter().any(|o| o.key == k) {
                res.set(&k, &v);
            }
        }
        res
    }

    /// Look a resource up.
    ///
    /// Falls back to matching on the last dot-separated component, because the
    /// defaults arrays are full of entries like `"*grid.foreground: white"`
    /// that hacks then ask for as plain `"foreground"`.
    pub fn get(&self, key: &str) -> Option<&str> {
        if let Some(v) = self.values.get(key) {
            return Some(v.as_str());
        }
        self.values
            .iter()
            .find(|(k, _)| k.rsplit('.').next() == Some(key))
            .map(|(_, v)| v.as_str())
    }

    /// Set an option, recording it as a user override.
    pub fn set(&mut self, key: &str, value: &str) {
        self.values.insert(key.to_string(), value.to_string());
        self.overrides.insert(key.to_string(), value.to_string());
    }

    /// Drop a user override, going back to the underlying default.
    pub fn unset(&mut self, key: &str, opts: &[Opt], defaults: &[&str]) {
        self.overrides.remove(key);
        let fresh = Resources::new(defaults, opts, "");
        match fresh.values.get(key) {
            Some(v) => {
                self.values.insert(key.to_string(), v.clone());
            }
            None => {
                self.values.remove(key);
            }
        }
    }

    /// `get_integer_resource`. C's `atoi` stops at the first non-digit, so a
    /// value like `"10000 "` or `"3px"` still parses.
    pub fn int(&self, key: &str) -> i32 {
        self.float(key) as i32
    }

    /// `get_float_resource`.
    pub fn float(&self, key: &str) -> f64 {
        let Some(v) = self.get(key) else { return 0.0 };
        parse_leading_number(v)
    }

    /// `get_boolean_resource`.
    pub fn bool(&self, key: &str) -> bool {
        let Some(v) = self.get(key) else { return false };
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "true" | "yes" | "on" | "1"
        )
    }

    /// `get_string_resource`.
    pub fn string(&self, key: &str) -> &str {
        self.get(key).unwrap_or("")
    }

    /// `get_pixel_resource`. Colours that will not parse fall back the way X does:
    /// white for a foreground, black for anything else.
    pub fn pixel(&self, key: &str) -> Pixel {
        let spec = self.get(key).unwrap_or("");
        parse_color(spec).unwrap_or(if key.contains("foreground") {
            WHITE
        } else {
            BLACK
        })
    }

    /// The user overrides as a query string, with no leading `?`.
    pub fn to_query(&self) -> String {
        self.overrides
            .iter()
            .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
            .collect::<Vec<_>>()
            .join("&")
    }

    /// The current value of an option as a string, for the panel.
    pub fn value_of(&self, opt: &Opt) -> String {
        self.get(opt.key).unwrap_or(opt.default).to_string()
    }

    /// Whether the panel has changed this option from its default.
    pub fn is_overridden(&self, key: &str) -> bool {
        self.overrides.contains_key(key)
    }
}

/// C's `atoi`/`atof` leniency: take the longest numeric prefix.
fn parse_leading_number(s: &str) -> f64 {
    let s = s.trim();
    let mut end = 0;
    for (i, c) in s.char_indices() {
        let ok = c.is_ascii_digit()
            || (i == 0 && (c == '-' || c == '+'))
            || (c == '.' && !s[..i].contains('.'));
        if !ok {
            break;
        }
        end = i + c.len_utf8();
    }
    s[..end].parse().unwrap_or(0.0)
}

/// Minimal `application/x-www-form-urlencoded` parse. The panel only ever
/// writes short ASCII values, but a hand-edited URL can contain anything.
fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .trim_start_matches('?')
        .split('&')
        .filter(|p| !p.is_empty())
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((decode(k), decode(v)))
        })
        .collect()
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULTS: &[&str] = &[
        ".background:	black",
        ".foreground:	white",
        "*delay:	10000",
        "*grey:	false",
        "*grid.linewidth:	3",
    ];

    const OPTS: &[Opt] = &[
        Opt::slider("delay", "Speed", 0.0, 100000.0, 1.0, 0, "20000"),
        Opt::boolean("grey", "Greyscale", "false"),
    ];

    #[test]
    fn xml_defaults_beat_hack_defaults() {
        let r = Resources::new(DEFAULTS, OPTS, "");
        assert_eq!(r.int("delay"), 20000);
    }

    #[test]
    fn query_beats_everything() {
        let r = Resources::new(DEFAULTS, OPTS, "?delay=42&grey=true");
        assert_eq!(r.int("delay"), 42);
        assert!(r.bool("grey"));
    }

    #[test]
    fn query_cannot_invent_resources() {
        // Only declared options are settable, so a hand-edited URL cannot
        // reach a resource the hack was never meant to expose.
        let r = Resources::new(DEFAULTS, OPTS, "?background=red&nonesuch=1");
        assert_eq!(r.pixel("background"), BLACK);
        assert_eq!(r.get("nonesuch"), None);
    }

    #[test]
    fn undeclared_resources_still_come_from_the_hack_defaults() {
        let r = Resources::new(DEFAULTS, OPTS, "");
        assert_eq!(r.pixel("foreground"), WHITE);
        assert_eq!(r.pixel("background"), BLACK);
    }

    #[test]
    fn dotted_defaults_are_found_by_their_last_component() {
        let r = Resources::new(DEFAULTS, OPTS, "");
        assert_eq!(r.int("linewidth"), 3);
    }

    #[test]
    fn only_overrides_go_back_into_the_url() {
        let mut r = Resources::new(DEFAULTS, OPTS, "");
        assert_eq!(r.to_query(), "");
        r.set("delay", "500");
        assert_eq!(r.to_query(), "delay=500");
        assert!(r.is_overridden("delay"));
        r.unset("delay", OPTS, DEFAULTS);
        assert_eq!(r.to_query(), "");
        assert_eq!(r.int("delay"), 20000, "unset should restore the default");
    }

    #[test]
    fn query_round_trips_through_encoding() {
        let mut r = Resources::new(DEFAULTS, OPTS, "");
        r.set("grey", "a b&c=d");
        let q = r.to_query();
        let back = Resources::new(DEFAULTS, OPTS, &q);
        assert_eq!(back.string("grey"), "a b&c=d");
    }

    #[test]
    fn numbers_parse_like_atoi() {
        assert_eq!(parse_leading_number("10000"), 10000.0);
        assert_eq!(parse_leading_number(" -3.5deg "), -3.5);
        assert_eq!(parse_leading_number("nope"), 0.0);
        assert_eq!(parse_leading_number("1.2.3"), 1.2);
        assert_eq!(parse_leading_number(""), 0.0);
    }

    #[test]
    fn booleans_accept_the_spellings_the_defaults_use() {
        for (v, want) in [
            ("true", true),
            ("True", true),
            ("yes", true),
            ("1", true),
            ("false", false),
            ("", false),
            ("nope", false),
        ] {
            let mut r = Resources::default();
            r.set("k", v);
            assert_eq!(r.bool("k"), want, "{v:?}");
        }
    }

    #[test]
    fn missing_resources_read_as_zero_not_a_panic() {
        let r = Resources::default();
        assert_eq!(r.int("nope"), 0);
        assert_eq!(r.float("nope"), 0.0);
        assert!(!r.bool("nope"));
        assert_eq!(r.string("nope"), "");
    }
}
