/// Backtracking NFA matcher for PCRE2 patterns.
use crate::ast::*;
use crate::Error;

/// Match state during execution.
pub struct MatchState<'a> {
    subject: &'a [u8],
    captures: Vec<Option<(usize, usize)>>,
    match_limit: u32,
    depth_limit: u32,
    steps: u32,
    depth: u32,
    options: Options,
}

impl<'a> MatchState<'a> {
    pub fn new(
        subject: &'a [u8],
        num_captures: u32,
        match_limit: u32,
        depth_limit: u32,
        options: Options,
    ) -> Self {
        Self {
            subject,
            captures: vec![None; (num_captures + 1) as usize],
            match_limit,
            depth_limit,
            steps: 0,
            depth: 0,
            options,
        }
    }

    /// Try to match the node at the given position. Returns new position on success.
    pub fn try_match(&mut self, node: &Node, pos: usize) -> Result<Option<usize>, Error> {
        self.steps += 1;
        if self.steps > self.match_limit {
            return Err(Error::MatchLimit);
        }

        match node {
            Node::Empty => Ok(Some(pos)),

            Node::Literal(b) => {
                if pos < self.subject.len() {
                    let actual = self.subject[pos];
                    let matches = if self.options.caseless {
                        actual.to_ascii_lowercase() == b.to_ascii_lowercase()
                    } else {
                        actual == *b
                    };
                    if matches {
                        Ok(Some(pos + 1))
                    } else {
                        Ok(None)
                    }
                } else {
                    Ok(None)
                }
            }

            Node::AnyByte => {
                if pos < self.subject.len() {
                    Ok(Some(pos + 1))
                } else {
                    Ok(None)
                }
            }

            Node::Class(class) => {
                if pos < self.subject.len() {
                    let b = self.subject[pos];
                    let in_class = class_matches(class, b, self.options.caseless);
                    if in_class {
                        Ok(Some(pos + 1))
                    } else {
                        Ok(None)
                    }
                } else {
                    Ok(None)
                }
            }

            Node::Anchor(kind) => {
                let matches = match kind {
                    AnchorKind::StartOfString => pos == 0,
                    AnchorKind::Start => pos == 0 || (pos > 0 && self.subject[pos - 1] == b'\n'),
                    AnchorKind::EndOfString => pos == self.subject.len(),
                    AnchorKind::EndOfStringBeforeNewline => {
                        pos == self.subject.len()
                            || (pos + 1 == self.subject.len() && self.subject[pos] == b'\n')
                    }
                    AnchorKind::End => {
                        pos == self.subject.len()
                            || self.subject[pos] == b'\n'
                    }
                };
                if matches {
                    Ok(Some(pos))
                } else {
                    Ok(None)
                }
            }

            Node::WordBoundary(positive) => {
                let at_boundary = is_word_boundary(self.subject, pos);
                if at_boundary == *positive {
                    Ok(Some(pos))
                } else {
                    Ok(None)
                }
            }

            Node::Concat(nodes) => self.match_concat(nodes, 0, pos),

            Node::Alternation(branches) => {
                for branch in branches {
                    let saved = self.save_captures();
                    match self.try_match(branch, pos)? {
                        Some(end) => return Ok(Some(end)),
                        None => self.restore_captures(saved),
                    }
                }
                Ok(None)
            }

            Node::Group { index, node, .. } => {
                self.depth += 1;
                if self.depth > self.depth_limit {
                    self.depth -= 1;
                    return Err(Error::DepthLimit);
                }
                let saved = self.captures[*index as usize];
                self.captures[*index as usize] = Some((pos, pos));
                match self.try_match(node, pos)? {
                    Some(end) => {
                        self.captures[*index as usize] = Some((pos, end));
                        self.depth -= 1;
                        Ok(Some(end))
                    }
                    None => {
                        self.captures[*index as usize] = saved;
                        self.depth -= 1;
                        Ok(None)
                    }
                }
            }

            Node::NonCapGroup(node) | Node::AtomicGroup(node) => {
                self.depth += 1;
                if self.depth > self.depth_limit {
                    self.depth -= 1;
                    return Err(Error::DepthLimit);
                }
                let result = self.try_match(node, pos);
                self.depth -= 1;
                result
            }

            Node::Backref(index) => {
                if let Some(Some((start, end))) = self.captures.get(*index as usize) {
                    let captured = &self.subject[*start..*end];
                    let remaining = &self.subject[pos..];
                    if remaining.len() >= captured.len() {
                        let matches = if self.options.caseless {
                            captured
                                .iter()
                                .zip(&remaining[..captured.len()])
                                .all(|(a, b)| a.to_ascii_lowercase() == b.to_ascii_lowercase())
                        } else {
                            &remaining[..captured.len()] == captured
                        };
                        if matches {
                            Ok(Some(pos + captured.len()))
                        } else {
                            Ok(None)
                        }
                    } else {
                        Ok(None)
                    }
                } else {
                    Ok(None)
                }
            }

            Node::Lookahead { node, positive } => {
                let saved = self.save_captures();
                let result = self.try_match(node, pos)?;
                let matched = result.is_some();
                if !positive {
                    self.restore_captures(saved);
                }
                if matched == *positive {
                    Ok(Some(pos))
                } else {
                    Ok(None)
                }
            }

            Node::Lookbehind { node, positive } => {
                // Try matching the lookbehind at decreasing positions before `pos`
                let saved = self.save_captures();
                let max_len = pos.min(self.subject.len());
                let mut matched = false;
                for start in (pos.saturating_sub(max_len))..=pos {
                    if let Some(end) = self.try_match(node, start)? {
                        if end == pos {
                            matched = true;
                            break;
                        }
                    }
                    self.restore_captures(saved.clone());
                }
                if !positive {
                    self.restore_captures(saved);
                }
                if matched == *positive {
                    Ok(Some(pos))
                } else {
                    Ok(None)
                }
            }

            Node::Quantifier {
                node,
                kind,
                greedy,
                possessive,
            } => self.match_quantifier(node, pos, *kind, *greedy, *possessive),

            Node::SetOptions {
                set,
                clear,
                node: Some(inner),
            } => {
                let saved_opts = self.options;
                self.apply_options(*set, *clear);
                let result = self.try_match(inner, pos);
                self.options = saved_opts;
                result
            }

            Node::SetOptions {
                set,
                clear,
                node: None,
            } => {
                self.apply_options(*set, *clear);
                Ok(Some(pos))
            }
        }
    }

    /// Match a concatenation with backtracking support.
    fn match_concat(
        &mut self,
        nodes: &[Node],
        index: usize,
        pos: usize,
    ) -> Result<Option<usize>, Error> {
        if index >= nodes.len() {
            return Ok(Some(pos));
        }

        let node = &nodes[index];

        // For quantifiers, use the continuation-aware version
        if let Node::Quantifier {
            node: inner,
            kind,
            greedy,
            possessive,
        } = node
        {
            return self.match_quantifier_with_cont(
                inner,
                pos,
                *kind,
                *greedy,
                *possessive,
                nodes,
                index + 1,
            );
        }

        // For other nodes, try matching then continue
        let saved = self.save_captures();
        match self.try_match(node, pos)? {
            Some(next) => {
                match self.match_concat(nodes, index + 1, next)? {
                    Some(end) => Ok(Some(end)),
                    None => {
                        self.restore_captures(saved);
                        Ok(None)
                    }
                }
            }
            None => Ok(None),
        }
    }

    fn match_quantifier(
        &mut self,
        node: &Node,
        pos: usize,
        kind: QuantKind,
        greedy: bool,
        possessive: bool,
    ) -> Result<Option<usize>, Error> {
        // Standalone quantifier (not in concat) — no continuation
        self.match_quantifier_with_cont(node, pos, kind, greedy, possessive, &[], 0)
    }

    fn match_quantifier_with_cont(
        &mut self,
        node: &Node,
        pos: usize,
        kind: QuantKind,
        greedy: bool,
        possessive: bool,
        cont_nodes: &[Node],
        cont_index: usize,
    ) -> Result<Option<usize>, Error> {
        let (min, max) = match kind {
            QuantKind::ZeroOrMore => (0, u32::MAX),
            QuantKind::OneOrMore => (1, u32::MAX),
            QuantKind::ZeroOrOne => (0, 1),
            QuantKind::Exactly(n) => (n, n),
            QuantKind::AtLeast(n) => (n, u32::MAX),
            QuantKind::Range(n, m) => (n, m),
        };

        if possessive {
            // Possessive: match as many as possible, no backtracking
            let mut cur = pos;
            let mut count = 0u32;
            while count < max {
                let saved = self.save_captures();
                match self.try_match(node, cur)? {
                    Some(next) if next > cur => {
                        cur = next;
                        count += 1;
                    }
                    _ => {
                        self.restore_captures(saved);
                        break;
                    }
                }
            }
            if count < min {
                return Ok(None);
            }
            return self.match_concat(cont_nodes, cont_index, cur);
        }

        // Recursive backtracking quantifier
        self.match_quant_recursive(node, pos, min, max, 0, greedy, cont_nodes, cont_index)
    }

    fn match_quant_recursive(
        &mut self,
        node: &Node,
        pos: usize,
        min: u32,
        max: u32,
        count: u32,
        greedy: bool,
        cont_nodes: &[Node],
        cont_index: usize,
    ) -> Result<Option<usize>, Error> {
        self.steps += 1;
        if self.steps > self.match_limit {
            return Err(Error::MatchLimit);
        }

        // If we've reached minimum, try the continuation
        if count >= min {
            if greedy && count < max {
                // Greedy: try matching more first, then fall back to continuation
                let saved = self.save_captures();
                match self.try_match(node, pos)? {
                    Some(next) if next > pos => {
                        match self.match_quant_recursive(
                            node, next, min, max, count + 1, greedy, cont_nodes, cont_index,
                        )? {
                            Some(end) => return Ok(Some(end)),
                            None => self.restore_captures(saved),
                        }
                    }
                    _ => self.restore_captures(saved),
                }
                // Greedy fallback: try continuation at current position
                return self.match_concat(cont_nodes, cont_index, pos);
            } else if !greedy {
                // Lazy: try continuation first, then match more
                let saved = self.save_captures();
                match self.match_concat(cont_nodes, cont_index, pos)? {
                    Some(end) => return Ok(Some(end)),
                    None => self.restore_captures(saved),
                }
                if count < max {
                    let saved = self.save_captures();
                    match self.try_match(node, pos)? {
                        Some(next) if next > pos => {
                            match self.match_quant_recursive(
                                node, next, min, max, count + 1, greedy, cont_nodes, cont_index,
                            )? {
                                Some(end) => return Ok(Some(end)),
                                None => self.restore_captures(saved),
                            }
                        }
                        _ => self.restore_captures(saved),
                    }
                }
                return Ok(None);
            } else {
                // Exact count reached — try continuation
                return self.match_concat(cont_nodes, cont_index, pos);
            }
        }

        // Haven't reached minimum yet — must match more
        let saved = self.save_captures();
        match self.try_match(node, pos)? {
            Some(next) if next > pos => self.match_quant_recursive(
                node, next, min, max, count + 1, greedy, cont_nodes, cont_index,
            ),
            Some(_) => {
                // Zero-width match
                self.restore_captures(saved);
                if count >= min {
                    self.match_concat(cont_nodes, cont_index, pos)
                } else {
                    Ok(None)
                }
            }
            None => {
                self.restore_captures(saved);
                Ok(None)
            }
        }
    }

    fn save_captures(&self) -> Vec<Option<(usize, usize)>> {
        self.captures.clone()
    }

    fn restore_captures(&mut self, saved: Vec<Option<(usize, usize)>>) {
        self.captures = saved;
    }

    fn apply_options(&mut self, set: Options, clear: Options) {
        if set.caseless {
            self.options.caseless = true;
        }
        if set.multiline {
            self.options.multiline = true;
        }
        if set.dotall {
            self.options.dotall = true;
        }
        if set.extended {
            self.options.extended = true;
        }
        if set.ungreedy {
            self.options.ungreedy = true;
        }
        if clear.caseless {
            self.options.caseless = false;
        }
        if clear.multiline {
            self.options.multiline = false;
        }
        if clear.dotall {
            self.options.dotall = false;
        }
        if clear.extended {
            self.options.extended = false;
        }
        if clear.ungreedy {
            self.options.ungreedy = false;
        }
    }

    pub fn get_captures(&self) -> &[Option<(usize, usize)>] {
        &self.captures
    }
}

/// Check if a byte matches a character class.
fn class_matches(class: &CharClass, b: u8, caseless: bool) -> bool {
    let in_ranges = class.ranges.iter().any(|r| range_matches(r, b, caseless));
    if class.negated {
        !in_ranges
    } else {
        in_ranges
    }
}

fn range_matches(range: &ClassRange, b: u8, caseless: bool) -> bool {
    match range {
        ClassRange::Single(c) => {
            if caseless {
                b.to_ascii_lowercase() == c.to_ascii_lowercase()
            } else {
                b == *c
            }
        }
        ClassRange::Range(start, end) => {
            if caseless {
                let lb = b.to_ascii_lowercase();
                (start.to_ascii_lowercase()..=end.to_ascii_lowercase()).contains(&lb)
            } else {
                (*start..=*end).contains(&b)
            }
        }
        ClassRange::Named(named) => named_class_matches(*named, b),
        ClassRange::UnicodeProperty(name, negated) => {
            let matches = unicode_property_matches(name, b);
            if *negated { !matches } else { matches }
        }
    }
}

fn named_class_matches(class: NamedClass, b: u8) -> bool {
    match class {
        NamedClass::Digit => b.is_ascii_digit(),
        NamedClass::NotDigit => !b.is_ascii_digit(),
        NamedClass::Word => b.is_ascii_alphanumeric() || b == b'_',
        NamedClass::NotWord => !(b.is_ascii_alphanumeric() || b == b'_'),
        NamedClass::Space => matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0C | 0x0B),
        NamedClass::NotSpace => !matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0C | 0x0B),
        NamedClass::HSpace => matches!(b, b' ' | b'\t'),
        NamedClass::NotHSpace => !matches!(b, b' ' | b'\t'),
        NamedClass::VSpace => matches!(b, b'\n' | b'\r' | 0x0B | 0x0C),
        NamedClass::NotVSpace => !matches!(b, b'\n' | b'\r' | 0x0B | 0x0C),
    }
}

fn unicode_property_matches(name: &str, b: u8) -> bool {
    // Simplified — handle ASCII subset of Unicode properties
    match name {
        "L" | "Letter" => b.is_ascii_alphabetic(),
        "Lu" | "Uppercase_Letter" => b.is_ascii_uppercase(),
        "Ll" | "Lowercase_Letter" => b.is_ascii_lowercase(),
        "N" | "Number" | "Nd" | "Decimal_Number" => b.is_ascii_digit(),
        "Z" | "Separator" | "Zs" | "Space_Separator" => b == b' ',
        "P" | "Punctuation" => b.is_ascii_punctuation(),
        "S" | "Symbol" => matches!(b, b'$' | b'+' | b'<' | b'=' | b'>' | b'^' | b'`' | b'|' | b'~'),
        _ => false, // Unknown property — no match for ASCII
    }
}

fn is_word_boundary(subject: &[u8], pos: usize) -> bool {
    let before_is_word = pos > 0 && is_word_byte(subject[pos - 1]);
    let after_is_word = pos < subject.len() && is_word_byte(subject[pos]);
    before_is_word != after_is_word
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::Parser;

    fn matches(pattern: &str, subject: &str) -> bool {
        let mut parser = Parser::new(pattern, Options::default());
        let ast = parser.parse().unwrap();
        let num_captures = parser.group_count();

        let subject_bytes = subject.as_bytes();
        // Try matching at each position
        for start in 0..=subject_bytes.len() {
            let mut state =
                MatchState::new(subject_bytes, num_captures, 100_000, 1000, Options::default());
            if let Ok(Some(_)) = state.try_match(&ast, start) {
                return true;
            }
        }
        false
    }

    #[test]
    fn literal_match() {
        assert!(matches("abc", "xabcy"));
        assert!(!matches("abc", "abd"));
    }

    #[test]
    fn dot_match() {
        assert!(matches("a.c", "abc"));
        assert!(matches("a.c", "axc"));
        assert!(!matches("a.c", "a\nc"));
    }

    #[test]
    fn char_class() {
        assert!(matches("[abc]", "b"));
        assert!(!matches("[abc]", "d"));
        assert!(matches("[a-z]", "m"));
        assert!(!matches("[a-z]", "M"));
    }

    #[test]
    fn negated_class() {
        assert!(!matches("[^abc]", "a"));
        assert!(matches("[^abc]", "d"));
    }

    #[test]
    fn quantifiers() {
        assert!(matches("ab*c", "ac"));
        assert!(matches("ab*c", "abbc"));
        assert!(matches("ab+c", "abc"));
        assert!(!matches("ab+c", "ac"));
        assert!(matches("ab?c", "ac"));
        assert!(matches("ab?c", "abc"));
    }

    #[test]
    fn anchors() {
        assert!(matches("^abc", "abc"));
        assert!(!matches("^abc", "xabc"));
        assert!(matches("abc$", "abc"));
        assert!(!matches("abc$", "abcx"));
    }

    #[test]
    fn alternation() {
        assert!(matches("cat|dog", "I have a cat"));
        assert!(matches("cat|dog", "I have a dog"));
        assert!(!matches("cat|dog", "I have a fish"));
    }

    #[test]
    fn groups_and_backrefs() {
        assert!(matches("(a)\\1", "aa"));
        assert!(!matches("(a)\\1", "ab"));
        assert!(matches("(ab)\\1", "abab"));
    }

    #[test]
    fn word_boundary() {
        assert!(matches("\\bword\\b", "a word here"));
        assert!(!matches("\\bword\\b", "awordhere"));
    }

    #[test]
    fn escape_sequences() {
        assert!(matches("\\d+", "abc123def"));
        assert!(matches("\\w+", "hello"));
        assert!(matches("\\s", "a b"));
    }

    #[test]
    fn lookahead() {
        assert!(matches("a(?=b)", "ab"));
        assert!(!matches("a(?=b)", "ac"));
        assert!(matches("a(?!b)", "ac"));
        assert!(!matches("a(?!b)", "ab"));
    }

    #[test]
    fn lookbehind() {
        assert!(matches("(?<=a)b", "ab"));
        assert!(!matches("(?<=a)b", "cb"));
    }

    #[test]
    fn interval() {
        assert!(matches("a{3}", "aaa"));
        assert!(!matches("a{3}", "aa"));
        assert!(matches("a{2,4}", "aaa"));
        assert!(!matches("a{2,4}", "a"));
    }

    #[test]
    fn match_limit() {
        let mut parser = Parser::new("((a+)*)+$", Options::default());
        let ast = parser.parse().unwrap();
        let num_captures = parser.group_count();
        let subject = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab";

        let mut state = MatchState::new(subject, num_captures, 10_000, 1000, Options::default());
        let result = state.try_match(&ast, 0);
        assert!(
            result.is_err(),
            "Expected match limit error, got {result:?}"
        );
    }
}
