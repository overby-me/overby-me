/// Backtracking NFA matcher for PCRE2 patterns.
///
/// Uses continuation-passing style: every match operation takes a continuation
/// representing the rest of the pattern to match after the current node.
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

/// A continuation — the rest of the pattern after the current node.
/// Represented as a slice of nodes in a Concat.
struct Cont<'a> {
    nodes: &'a [Node],
}

impl<'a> Cont<'a> {
    fn empty() -> Self {
        Self { nodes: &[] }
    }
    fn from_slice(nodes: &'a [Node]) -> Self {
        Self { nodes }
    }
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

    fn check_limit(&mut self) -> Result<(), Error> {
        self.steps += 1;
        if self.steps > self.match_limit {
            Err(Error::MatchLimit)
        } else {
            Ok(())
        }
    }

    /// Try to match the node at position, then match the continuation.
    /// Returns the final position if everything succeeds.
    pub fn try_match(&mut self, node: &Node, pos: usize) -> Result<Option<usize>, Error> {
        self.try_match_cont(node, pos, &Cont::empty())
    }

    fn try_match_cont(
        &mut self,
        node: &Node,
        pos: usize,
        cont: &Cont,
    ) -> Result<Option<usize>, Error> {
        self.check_limit()?;

        match node {
            Node::Empty => self.run_cont(cont, pos),

            Node::Literal(b) => {
                if pos < self.subject.len() {
                    let actual = self.subject[pos];
                    let matches = if self.options.caseless {
                        actual.to_ascii_lowercase() == b.to_ascii_lowercase()
                    } else {
                        actual == *b
                    };
                    if matches {
                        self.run_cont(cont, pos + 1)
                    } else {
                        Ok(None)
                    }
                } else {
                    Ok(None)
                }
            }

            Node::AnyByte => {
                if pos < self.subject.len() {
                    self.run_cont(cont, pos + 1)
                } else {
                    Ok(None)
                }
            }

            Node::Class(class) => {
                if pos < self.subject.len() {
                    let b = self.subject[pos];
                    if class_matches(class, b, self.options.caseless) {
                        self.run_cont(cont, pos + 1)
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
                    AnchorKind::End => pos == self.subject.len() || self.subject[pos] == b'\n',
                };
                if matches {
                    self.run_cont(cont, pos)
                } else {
                    Ok(None)
                }
            }

            Node::WordBoundary(positive) => {
                if is_word_boundary(self.subject, pos) == *positive {
                    self.run_cont(cont, pos)
                } else {
                    Ok(None)
                }
            }

            Node::Concat(nodes) => self.match_concat_cont(nodes, 0, pos, cont),

            Node::Alternation(branches) => {
                for branch in branches {
                    let saved = self.save();
                    match self.try_match_cont(branch, pos, cont)? {
                        Some(end) => return Ok(Some(end)),
                        None => self.restore(saved),
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
                let saved_cap = self.captures[*index as usize];
                self.captures[*index as usize] = Some((pos, pos));
                // Wrap continuation to update capture end position
                let result = self.try_match_group(*index, node, pos, cont);
                if result.as_ref().is_ok_and(|r| r.is_none()) {
                    self.captures[*index as usize] = saved_cap;
                }
                self.depth -= 1;
                result
            }

            Node::NonCapGroup(node) => {
                self.depth += 1;
                if self.depth > self.depth_limit {
                    self.depth -= 1;
                    return Err(Error::DepthLimit);
                }
                let result = self.try_match_cont(node, pos, cont);
                self.depth -= 1;
                result
            }

            Node::AtomicGroup(node) => {
                // Atomic: match the sub-expression without cont, then run cont
                self.depth += 1;
                if self.depth > self.depth_limit {
                    self.depth -= 1;
                    return Err(Error::DepthLimit);
                }
                let result = match self.try_match(node, pos)? {
                    Some(end) => self.run_cont(cont, end),
                    None => Ok(None),
                };
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
                            self.run_cont(cont, pos + captured.len())
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
                let saved = self.save();
                let result = self.try_match(node, pos)?;
                let matched = result.is_some();
                if !positive {
                    self.restore(saved);
                }
                if matched == *positive {
                    self.run_cont(cont, pos)
                } else {
                    Ok(None)
                }
            }

            Node::Lookbehind { node, positive } => {
                let saved = self.save();
                let mut matched = false;
                for start in (0..=pos).rev() {
                    if let Some(end) = self.try_match(node, start)? {
                        if end == pos {
                            matched = true;
                            break;
                        }
                    }
                    self.restore(saved.clone());
                }
                if !positive {
                    self.restore(saved);
                }
                if matched == *positive {
                    self.run_cont(cont, pos)
                } else {
                    Ok(None)
                }
            }

            Node::Quantifier {
                node,
                kind,
                greedy,
                possessive,
            } => self.match_quant(node, pos, *kind, *greedy, *possessive, cont),

            Node::SetOptions {
                set,
                clear,
                node: Some(inner),
            } => {
                let saved_opts = self.options;
                self.apply_options(*set, *clear);
                let result = self.try_match_cont(inner, pos, cont);
                self.options = saved_opts;
                result
            }

            Node::SetOptions {
                set,
                clear,
                node: None,
            } => {
                self.apply_options(*set, *clear);
                self.run_cont(cont, pos)
            }
        }
    }

    /// Run the continuation (remaining nodes in a concat).
    fn run_cont(&mut self, cont: &Cont, pos: usize) -> Result<Option<usize>, Error> {
        if cont.nodes.is_empty() {
            return Ok(Some(pos));
        }
        self.match_concat_cont(cont.nodes, 0, pos, &Cont::empty())
    }

    /// Match a concat with continuation support.
    fn match_concat_cont(
        &mut self,
        nodes: &[Node],
        index: usize,
        pos: usize,
        outer_cont: &Cont,
    ) -> Result<Option<usize>, Error> {
        if index >= nodes.len() {
            return self.run_cont(outer_cont, pos);
        }

        // Build continuation from remaining nodes + outer continuation
        let rest = &nodes[index + 1..];

        // For the current node, the continuation is: rest of this concat + outer cont
        // We pass both by creating a combined continuation
        let node = &nodes[index];

        // Special handling for quantifiers — they need the full continuation
        if let Node::Quantifier {
            node: inner,
            kind,
            greedy,
            possessive,
        } = node
        {
            // Build a continuation that matches the rest of this concat then outer
            return self.match_quant_in_concat(
                inner, pos, *kind, *greedy, *possessive, nodes, index + 1, outer_cont,
            );
        }

        // For other nodes, create a struct-based continuation
        // We need to match `node` at `pos`, then match rest at the new pos
        let saved = self.save();
        // Create inner cont as the rest of this concat
        let inner_cont = if rest.is_empty() {
            outer_cont.clone()
        } else {
            // We can't easily combine, so just recurse
            Cont::empty() // placeholder
        };

        match self.try_match_cont(node, pos, &inner_cont)? {
            Some(next_pos) => {
                if rest.is_empty() {
                    // Already ran outer_cont via inner_cont
                    Ok(Some(next_pos))
                } else {
                    match self.match_concat_cont(nodes, index + 1, next_pos, outer_cont)? {
                        Some(end) => Ok(Some(end)),
                        None => {
                            self.restore(saved);
                            Ok(None)
                        }
                    }
                }
            }
            None => {
                self.restore(saved);
                Ok(None)
            }
        }
    }

    /// Match a group, tracking capture positions.
    fn try_match_group(
        &mut self,
        index: u32,
        node: &Node,
        start: usize,
        cont: &Cont,
    ) -> Result<Option<usize>, Error> {
        let saved = self.save();
        match self.try_match(node, start)? {
            Some(end) => {
                self.captures[index as usize] = Some((start, end));
                match self.run_cont(cont, end)? {
                    Some(final_end) => Ok(Some(final_end)),
                    None => {
                        self.restore(saved);
                        Ok(None)
                    }
                }
            }
            None => Ok(None),
        }
    }

    /// Match a quantifier that's part of a Concat, with full continuation.
    fn match_quant_in_concat(
        &mut self,
        node: &Node,
        pos: usize,
        kind: QuantKind,
        greedy: bool,
        possessive: bool,
        concat_nodes: &[Node],
        next_index: usize,
        outer_cont: &Cont,
    ) -> Result<Option<usize>, Error> {
        let (min, max) = match kind {
            QuantKind::ZeroOrMore => (0, u32::MAX),
            QuantKind::OneOrMore => (1, u32::MAX),
            QuantKind::ZeroOrOne => (0, 1),
            QuantKind::Exactly(n) => (n, n),
            QuantKind::AtLeast(n) => (n, u32::MAX),
            QuantKind::Range(n, m) => (n, m),
        };

        self.quant_bt(
            node,
            pos,
            min,
            max,
            0,
            greedy,
            possessive,
            concat_nodes,
            next_index,
            outer_cont,
        )
    }

    /// Recursive backtracking quantifier with full continuation support.
    fn quant_bt(
        &mut self,
        node: &Node,
        pos: usize,
        min: u32,
        max: u32,
        count: u32,
        greedy: bool,
        possessive: bool,
        concat_nodes: &[Node],
        next_index: usize,
        outer_cont: &Cont,
    ) -> Result<Option<usize>, Error> {
        self.check_limit()?;

        let try_cont = |state: &mut Self, p: usize| -> Result<Option<usize>, Error> {
            state.match_concat_cont(concat_nodes, next_index, p, outer_cont)
        };

        if count >= min {
            if possessive {
                // Possessive: match as much as possible, no backtracking
                if count < max {
                    let saved = self.save();
                    match self.try_match(node, pos)? {
                        Some(next) if next > pos => {
                            return self.quant_bt(
                                node,
                                next,
                                min,
                                max,
                                count + 1,
                                greedy,
                                possessive,
                                concat_nodes,
                                next_index,
                                outer_cont,
                            );
                        }
                        _ => self.restore(saved),
                    }
                }
                return try_cont(self, pos);
            }

            if greedy {
                // Greedy: try matching more, then fall back to continuation
                if count < max {
                    let saved = self.save();
                    match self.try_match(node, pos)? {
                        Some(next) if next > pos => {
                            match self.quant_bt(
                                node,
                                next,
                                min,
                                max,
                                count + 1,
                                greedy,
                                possessive,
                                concat_nodes,
                                next_index,
                                outer_cont,
                            )? {
                                Some(end) => return Ok(Some(end)),
                                None => self.restore(saved),
                            }
                        }
                        _ => self.restore(saved),
                    }
                }
                // Fall back: try continuation at current position
                return try_cont(self, pos);
            } else {
                // Lazy: try continuation first, then match more
                let saved = self.save();
                match try_cont(self, pos)? {
                    Some(end) => return Ok(Some(end)),
                    None => self.restore(saved),
                }
                if count < max {
                    let saved = self.save();
                    match self.try_match(node, pos)? {
                        Some(next) if next > pos => {
                            match self.quant_bt(
                                node,
                                next,
                                min,
                                max,
                                count + 1,
                                greedy,
                                possessive,
                                concat_nodes,
                                next_index,
                                outer_cont,
                            )? {
                                Some(end) => return Ok(Some(end)),
                                None => self.restore(saved),
                            }
                        }
                        _ => self.restore(saved),
                    }
                }
                return Ok(None);
            }
        }

        // Haven't reached minimum — must match more
        let saved = self.save();
        match self.try_match(node, pos)? {
            Some(next) if next > pos => self.quant_bt(
                node,
                next,
                min,
                max,
                count + 1,
                greedy,
                possessive,
                concat_nodes,
                next_index,
                outer_cont,
            ),
            _ => {
                self.restore(saved);
                Ok(None)
            }
        }
    }

    /// Match a standalone quantifier (not in a concat).
    fn match_quant(
        &mut self,
        node: &Node,
        pos: usize,
        kind: QuantKind,
        greedy: bool,
        possessive: bool,
        cont: &Cont,
    ) -> Result<Option<usize>, Error> {
        let (min, max) = match kind {
            QuantKind::ZeroOrMore => (0, u32::MAX),
            QuantKind::OneOrMore => (1, u32::MAX),
            QuantKind::ZeroOrOne => (0, 1),
            QuantKind::Exactly(n) => (n, n),
            QuantKind::AtLeast(n) => (n, u32::MAX),
            QuantKind::Range(n, m) => (n, m),
        };
        self.quant_bt_standalone(node, pos, min, max, 0, greedy, possessive, cont)
    }

    fn quant_bt_standalone(
        &mut self,
        node: &Node,
        pos: usize,
        min: u32,
        max: u32,
        count: u32,
        greedy: bool,
        possessive: bool,
        cont: &Cont,
    ) -> Result<Option<usize>, Error> {
        self.check_limit()?;

        if count >= min {
            if possessive {
                if count < max {
                    let saved = self.save();
                    match self.try_match(node, pos)? {
                        Some(next) if next > pos => {
                            return self.quant_bt_standalone(
                                node, next, min, max, count + 1, greedy, possessive, cont,
                            );
                        }
                        _ => self.restore(saved),
                    }
                }
                return self.run_cont(cont, pos);
            }

            if greedy {
                if count < max {
                    let saved = self.save();
                    match self.try_match(node, pos)? {
                        Some(next) if next > pos => {
                            match self.quant_bt_standalone(
                                node, next, min, max, count + 1, greedy, possessive, cont,
                            )? {
                                Some(end) => return Ok(Some(end)),
                                None => self.restore(saved),
                            }
                        }
                        _ => self.restore(saved),
                    }
                }
                return self.run_cont(cont, pos);
            } else {
                let saved = self.save();
                match self.run_cont(cont, pos)? {
                    Some(end) => return Ok(Some(end)),
                    None => self.restore(saved),
                }
                if count < max {
                    let saved = self.save();
                    match self.try_match(node, pos)? {
                        Some(next) if next > pos => {
                            match self.quant_bt_standalone(
                                node, next, min, max, count + 1, greedy, possessive, cont,
                            )? {
                                Some(end) => return Ok(Some(end)),
                                None => self.restore(saved),
                            }
                        }
                        _ => self.restore(saved),
                    }
                }
                return Ok(None);
            }
        }

        let saved = self.save();
        match self.try_match(node, pos)? {
            Some(next) if next > pos => self.quant_bt_standalone(
                node, next, min, max, count + 1, greedy, possessive, cont,
            ),
            _ => {
                self.restore(saved);
                Ok(None)
            }
        }
    }

    fn save(&self) -> Vec<Option<(usize, usize)>> {
        self.captures.clone()
    }

    fn restore(&mut self, saved: Vec<Option<(usize, usize)>>) {
        self.captures = saved;
    }

    fn apply_options(&mut self, set: Options, clear: Options) {
        if set.caseless { self.options.caseless = true; }
        if set.multiline { self.options.multiline = true; }
        if set.dotall { self.options.dotall = true; }
        if set.extended { self.options.extended = true; }
        if set.ungreedy { self.options.ungreedy = true; }
        if clear.caseless { self.options.caseless = false; }
        if clear.multiline { self.options.multiline = false; }
        if clear.dotall { self.options.dotall = false; }
        if clear.extended { self.options.extended = false; }
        if clear.ungreedy { self.options.ungreedy = false; }
    }

    pub fn get_captures(&self) -> &[Option<(usize, usize)>] {
        &self.captures
    }
}

fn class_matches(class: &CharClass, b: u8, caseless: bool) -> bool {
    let in_ranges = class.ranges.iter().any(|r| range_matches(r, b, caseless));
    if class.negated { !in_ranges } else { in_ranges }
}

fn range_matches(range: &ClassRange, b: u8, caseless: bool) -> bool {
    match range {
        ClassRange::Single(c) => {
            if caseless { b.to_ascii_lowercase() == c.to_ascii_lowercase() } else { b == *c }
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
            let m = unicode_property_matches(name, b);
            if *negated { !m } else { m }
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
    match name {
        "L" | "Letter" => b.is_ascii_alphabetic(),
        "Lu" | "Uppercase_Letter" => b.is_ascii_uppercase(),
        "Ll" | "Lowercase_Letter" => b.is_ascii_lowercase(),
        "N" | "Number" | "Nd" | "Decimal_Number" => b.is_ascii_digit(),
        "Z" | "Separator" | "Zs" | "Space_Separator" => b == b' ',
        "P" | "Punctuation" => b.is_ascii_punctuation(),
        _ => false,
    }
}

fn is_word_boundary(subject: &[u8], pos: usize) -> bool {
    let before = pos > 0 && is_word_byte(subject[pos - 1]);
    let after = pos < subject.len() && is_word_byte(subject[pos]);
    before != after
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

impl Clone for Cont<'_> {
    fn clone(&self) -> Self {
        Self { nodes: self.nodes }
    }
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
        assert!(!matches("a.c", "a\nc"));
    }

    #[test]
    fn char_class() {
        assert!(matches("[abc]", "b"));
        assert!(!matches("[abc]", "d"));
        assert!(matches("[a-z]", "m"));
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
        assert!(!matches("cat|dog", "I have a fish"));
    }

    #[test]
    fn groups_and_backrefs() {
        assert!(matches("(a)\\1", "aa"));
        assert!(!matches("(a)\\1", "ab"));
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
    }

    #[test]
    fn greedy_backtracking() {
        // Greedy quantifier must backtrack for the $ to match
        assert!(matches("a+$", "aaa"));
        assert!(!matches("a+$", "aaab"));
    }

    #[test]
    fn match_limit() {
        let mut parser = Parser::new("((a+)*)+$", Options::default());
        let ast = parser.parse().unwrap();
        let num_captures = parser.group_count();
        let subject = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab";
        let mut state = MatchState::new(subject, num_captures, 10_000, 1000, Options::default());
        let result = state.try_match(&ast, 0);
        assert!(result.is_err(), "Expected match limit error, got {result:?}");
    }
}
