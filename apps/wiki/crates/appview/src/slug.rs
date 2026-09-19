//! The URL segment a node gets from its name.
//!
//! A key is forever: it is the URL, and every descendant's path carries it. This
//! is the frontend's `slug_base` (`src/components/loader.rs`) moved to the
//! server, which now picks the key because only it can see every sibling. It
//! must slug a name exactly as that function does, or a name would get one URL
//! before the cutover and another after it; the tests below are its cases.

/// The longest a generated key may be. Keys already stored are left as they are.
pub const KEY_MAXLEN: usize = 60;

/// How many numbered keys to try before giving up on a readable one.
pub const KEY_ATTEMPTS: u32 = 20;

/// The clean key for a name: lowercase, letters and digits kept (ø, æ and å
/// included, as the wiki's keys have always kept them), a hyphen between two
/// letters kept as part of the word, everything else collapsed to one
/// underscore. Empty when the name holds nothing sluggable.
pub fn slug_base(name: &str) -> String {
    let lowered = name.trim().to_lowercase();
    let chars: Vec<char> = lowered.chars().collect();
    let mut base = String::new();
    let mut prev_sep = true;
    for (i, &c) in chars.iter().enumerate() {
        if c.is_alphanumeric() {
            base.push(c);
            prev_sep = false;
            continue;
        }
        // `to-statsløsningen` keeps its hyphen; `Klima-, og` does not.
        let joins_words =
            c == '-' && !prev_sep && chars.get(i + 1).is_some_and(|next| next.is_alphanumeric());
        if joins_words {
            base.push('-');
            prev_sep = false;
        } else if !prev_sep {
            base.push('_');
            prev_sep = true;
        }
    }
    truncate_key(base.trim_end_matches('_'))
}

/// Cut a slug to [`KEY_MAXLEN`] characters, on a word boundary where there is
/// one past the halfway mark, so it still reads as the title it came from.
fn truncate_key(base: &str) -> String {
    let chars: Vec<char> = base.chars().collect();
    if chars.len() <= KEY_MAXLEN {
        return base.to_string();
    }
    let cut: String = chars[..KEY_MAXLEN].iter().collect();
    let boundary = cut
        .rfind('_')
        .filter(|i| *i >= KEY_MAXLEN / 2)
        .unwrap_or(cut.len());
    cut[..boundary].trim_end_matches(['_', '-']).to_string()
}

/// The keys to try for a name, cleanest first: `name`, `name-2` ... up to
/// [`KEY_ATTEMPTS`], then one that cannot collide. A number is spent only when
/// the plain key is taken, because it stays in the URL forever.
pub fn candidates(name: &str) -> impl Iterator<Item = String> {
    let base = match slug_base(name) {
        base if base.is_empty() => "n".to_string(),
        base => base,
    };
    let tail: String = crate::util::random_bytes(3)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let last = format!("{base}-{tail}");
    (1..=KEY_ATTEMPTS)
        .map(move |attempt| match attempt {
            1 => base.clone(),
            n => format!("{base}-{n}"),
        })
        .chain(std::iter::once(last))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_slugs_as_the_frontend_always_slugged_it() {
        for (name, key) in [
            ("Hello, World! 123", "hello_world_123"),
            ("  Trim -- Me  ", "trim_me"),
            ("!!!", ""),
            ("Landsmøde 2026", "landsmøde_2026"),
            ("Asger Holm Ørskov", "asger_holm_ørskov"),
            ("Saint-Laguës metode", "saint-laguës_metode"),
            ("To-statsløsningen", "to-statsløsningen"),
            ("Klima-, og Miljøudvalget", "klima_og_miljøudvalget"),
            ("EU- og Udenrigsudvalget", "eu_og_udenrigsudvalget"),
            (
                "Hvor pengene kommer fra - en finansplan",
                "hvor_pengene_kommer_fra_en_finansplan",
            ),
            ("Klima- ", "klima"),
            ("--", ""),
            ("- Leading", "leading"),
            ("Dagsorden 3.0", "dagsorden_3_0"),
        ] {
            assert_eq!(slug_base(name), key, "{name:?}");
        }
    }

    #[test]
    fn a_key_is_cut_on_a_word_and_by_character() {
        let pasted = "## Kandidatur til Klima- og Miljøudvalget  Kære alle,  jeg stiller (igen) op til Klima- og Miljøudvalget, fordi jeg mener at";
        let key = slug_base(pasted);
        assert!(key.chars().count() <= KEY_MAXLEN, "{key}");
        assert!(!key.ends_with('_'), "{key}");
        assert!(key.starts_with("kandidatur_til_klima_og_milj"), "{key}");

        assert_eq!(slug_base(&"a".repeat(200)).chars().count(), KEY_MAXLEN);
        // æ, ø and å are two bytes each: a byte-wise cut would split one.
        let multibyte = slug_base(&"æøå".repeat(40));
        assert!(multibyte.chars().count() <= KEY_MAXLEN, "{multibyte}");
    }

    #[test]
    fn the_clean_key_is_offered_first_and_a_unique_one_last() {
        let keys: Vec<String> = candidates("Dagsorden").collect();
        assert_eq!(keys[0], "dagsorden");
        assert_eq!(keys[1], "dagsorden-2");
        assert_eq!(keys[19], "dagsorden-20");
        assert_eq!(keys.len(), 21);
        assert!(keys[20].starts_with("dagsorden-"), "{}", keys[20]);
        assert_ne!(
            keys[20], "dagsorden-21",
            "the last key must not be guessable"
        );

        assert_eq!(candidates("!!!").next().as_deref(), Some("n"));
    }
}
