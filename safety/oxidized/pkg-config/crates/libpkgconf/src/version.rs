//! Version comparison using RPM-style algorithm.
//!
//! This implements the same version comparison semantics as pkgconf/pkg-config,
//! which follows the RPM version comparison algorithm (rpmvercmp).
//!
//! The algorithm splits version strings into segments of digits or letters,
//! then compares segment by segment:
//! - Digit segments are compared numerically (with leading zeros stripped).
//! - Letter segments are compared lexicographically.
//! - Digit segments are always considered newer than letter segments.
//! - A version with remaining segments is considered newer than one without.
//! - Tilde (`~`) segments sort before anything, even the empty string (pre-release).

use crate::error::{Error, Result};

/// Compare two version strings using RPM-style version comparison.
///
/// Returns:
/// - `> 0` if `a` is newer than `b`
/// - `0` if `a` and `b` are equal
/// - `< 0` if `a` is older than `b`
///
/// # Examples
///
/// ```
/// use libpkgconf::version::compare;
///
/// assert!(compare("1.2.3", "1.2.2") > 0);
/// assert!(compare("1.2.3", "1.2.3") == 0);
/// assert!(compare("1.2.2", "1.2.3") < 0);
/// assert!(compare("1.0", "1.0.0") < 0);
/// assert!(compare("1.0~rc1", "1.0") < 0);
/// ```
pub fn compare(a: &str, b: &str) -> i32 {
    if a == b {
        return 0;
    }

    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let mut ai = 0;
    let mut bi = 0;

    loop {
        // Skip over non-alphanumeric, non-tilde characters
        while ai < a_bytes.len() && !a_bytes[ai].is_ascii_alphanumeric() && a_bytes[ai] != b'~' {
            ai += 1;
        }
        while bi < b_bytes.len() && !b_bytes[bi].is_ascii_alphanumeric() && b_bytes[bi] != b'~' {
            bi += 1;
        }

        // Handle tilde: sorts before everything, even empty segment.
        // This allows pre-release versions like "1.0~rc1" < "1.0".
        let a_tilde = ai < a_bytes.len() && a_bytes[ai] == b'~';
        let b_tilde = bi < b_bytes.len() && b_bytes[bi] == b'~';

        if a_tilde || b_tilde {
            if !a_tilde {
                return 1;
            }
            if !b_tilde {
                return -1;
            }
            // Both have tilde, skip them and continue
            ai += 1;
            bi += 1;
            continue;
        }

        // If we've exhausted both strings, they're equal
        if ai >= a_bytes.len() && bi >= b_bytes.len() {
            return 0;
        }

        // Whichever string still has content is "newer"
        if ai >= a_bytes.len() {
            return -1;
        }
        if bi >= b_bytes.len() {
            return 1;
        }

        // Determine if this segment is digits or letters
        let is_digit = a_bytes[ai].is_ascii_digit();

        // Extract segment from a
        let a_start = ai;
        if is_digit {
            while ai < a_bytes.len() && a_bytes[ai].is_ascii_digit() {
                ai += 1;
            }
        } else {
            while ai < a_bytes.len() && a_bytes[ai].is_ascii_alphabetic() {
                ai += 1;
            }
        }
        let a_seg = &a_bytes[a_start..ai];

        // Extract segment from b using the same character class
        let b_start = bi;
        if is_digit {
            while bi < b_bytes.len() && b_bytes[bi].is_ascii_digit() {
                bi += 1;
            }
        } else {
            while bi < b_bytes.len() && b_bytes[bi].is_ascii_alphabetic() {
                bi += 1;
            }
        }
        let b_seg = &b_bytes[b_start..bi];

        // If segments are different types (one is digits, the other is letters at
        // the same position), the digit segment is newer
        if a_seg.is_empty() {
            return -1;
        }
        if b_seg.is_empty() {
            return if is_digit { 1 } else { -1 };
        }

        if is_digit {
            // Compare numerically: skip leading zeros, then compare
            let a_trimmed = trim_leading_zeros(a_seg);
            let b_trimmed = trim_leading_zeros(b_seg);

            // Longer number (after trimming leading zeros) is greater
            match a_trimmed.len().cmp(&b_trimmed.len()) {
                std::cmp::Ordering::Greater => return 1,
                std::cmp::Ordering::Less => return -1,
                std::cmp::Ordering::Equal => {
                    // Same length: compare lexicographically (works for digit strings)
                    match a_trimmed.cmp(b_trimmed) {
                        std::cmp::Ordering::Greater => return 1,
                        std::cmp::Ordering::Less => return -1,
                        std::cmp::Ordering::Equal => {}
                    }
                }
            }
        } else {
            // Compare letter segments lexicographically
            match a_seg.cmp(b_seg) {
                std::cmp::Ordering::Greater => return 1,
                std::cmp::Ordering::Less => return -1,
                std::cmp::Ordering::Equal => {}
            }
        }

        // Segments are equal, continue to next segment
    }
}

/// Trim leading ASCII zeros from a byte slice, leaving at least one byte.
fn trim_leading_zeros(s: &[u8]) -> &[u8] {
    let mut i = 0;
    while i + 1 < s.len() && s[i] == b'0' {
        i += 1;
    }
    &s[i..]
}

/// Version comparison operators, matching pkgconf's `pkgconf_pkg_comparator_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Comparator {
    /// Any version matches (no constraint).
    #[default]
    Any,
    /// `=`
    Equal,
    /// `!=`
    NotEqual,
    /// `<`
    LessThan,
    /// `<=`
    LessThanEqual,
    /// `>`
    GreaterThan,
    /// `>=`
    GreaterThanEqual,
}

impl Comparator {
    /// The total number of comparator variants.
    pub const COUNT: usize = 7;

    /// Parse a comparator from its string representation.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Result<Self> {
        match s {
            "" => Ok(Self::Any),
            "=" | "==" => Ok(Self::Equal),
            "!=" => Ok(Self::NotEqual),
            "<" => Ok(Self::LessThan),
            "<=" => Ok(Self::LessThanEqual),
            ">" => Ok(Self::GreaterThan),
            ">=" => Ok(Self::GreaterThanEqual),
            _ => Err(Error::InvalidComparator {
                operator: s.to_string(),
            }),
        }
    }

    /// Get the string representation of this comparator.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Any => "(?)",
            Self::Equal => "=",
            Self::NotEqual => "!=",
            Self::LessThan => "<",
            Self::LessThanEqual => "<=",
            Self::GreaterThan => ">",
            Self::GreaterThanEqual => ">=",
        }
    }

    /// Evaluate whether the comparison `actual <op> target` is satisfied.
    ///
    /// Uses RPM-style version comparison internally.
    pub fn eval(self, actual: &str, target: &str) -> bool {
        match self {
            Self::Any => true,
            Self::Equal => compare(actual, target) == 0,
            Self::NotEqual => compare(actual, target) != 0,
            Self::LessThan => compare(actual, target) < 0,
            Self::LessThanEqual => compare(actual, target) <= 0,
            Self::GreaterThan => compare(actual, target) > 0,
            Self::GreaterThanEqual => compare(actual, target) >= 0,
        }
    }
}

impl std::fmt::Display for Comparator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Check whether a character is a version operator character.
pub fn is_operator_char(c: char) -> bool {
    matches!(c, '<' | '>' | '!' | '=')
}

/// Check whether a character is a module separator.
pub fn is_module_separator(c: char) -> bool {
    c == ',' || c.is_ascii_whitespace()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_equal_versions() {
        assert_eq!(compare("1.0", "1.0"), 0);
        assert_eq!(compare("1.2.3", "1.2.3"), 0);
        assert_eq!(compare("0", "0"), 0);
        assert_eq!(compare("abc", "abc"), 0);
    }

    #[test]
    fn test_numeric_comparison() {
        assert!(compare("1.1", "1.0") > 0);
        assert!(compare("1.0", "1.1") < 0);
        assert!(compare("2.0", "1.9") > 0);
        assert!(compare("1.10", "1.9") > 0);
        assert!(compare("1.9", "1.10") < 0);
    }

    #[test]
    fn test_leading_zeros() {
        assert_eq!(compare("01", "1"), 0);
        assert_eq!(compare("001", "1"), 0);
        assert_eq!(compare("1.01", "1.1"), 0);
    }

    #[test]
    fn test_different_segment_counts() {
        assert!(compare("1.0.0", "1.0") > 0);
        assert!(compare("1.0", "1.0.0") < 0);
        assert!(compare("1.0.1", "1.0") > 0);
    }

    #[test]
    fn test_mixed_alpha_numeric() {
        // Digit segments are newer than letter segments
        assert!(compare("1.0a", "1.0") > 0);
        assert!(compare("1.0.1", "1.0a") > 0);
    }

    #[test]
    fn test_alpha_comparison() {
        assert!(compare("1.0b", "1.0a") > 0);
        assert!(compare("1.0a", "1.0b") < 0);
        assert_eq!(compare("1.0a", "1.0a"), 0);
    }

    #[test]
    fn test_tilde_prerelease() {
        // Tilde sorts before everything, enabling pre-release versions
        assert!(compare("1.0~rc1", "1.0") < 0);
        assert!(compare("1.0", "1.0~rc1") > 0);
        assert!(compare("1.0~rc1", "1.0~rc2") < 0);
        assert!(compare("1.0~rc2", "1.0~rc1") > 0);
        assert_eq!(compare("1.0~rc1", "1.0~rc1"), 0);
    }

    #[test]
    fn test_tilde_both_sides() {
        assert!(compare("1.0~alpha", "1.0~beta") < 0);
        assert!(compare("1.0~beta", "1.0~alpha") > 0);
    }

    #[test]
    fn test_empty_strings() {
        assert_eq!(compare("", ""), 0);
        assert!(compare("1", "") > 0);
        assert!(compare("", "1") < 0);
    }

    #[test]
    fn test_separators_ignored() {
        // Non-alphanumeric, non-tilde separators are skipped
        assert_eq!(compare("1.0.0", "1-0-0"), 0);
        assert_eq!(compare("1_0_0", "1.0.0"), 0);
        assert_eq!(compare("1:0:0", "1.0.0"), 0);
    }

    #[test]
    fn test_real_world_versions() {
        assert!(compare("3.22.1", "3.22.0") > 0);
        assert!(compare("2.76.1", "2.75.0") > 0);
        assert!(compare("0.29.1", "0.28.0") > 0);
        assert!(compare("1.16.3", "1.16.2") > 0);
    }

    #[test]
    fn test_comparator_from_str() {
        assert_eq!(Comparator::from_str("").unwrap(), Comparator::Any);
        assert_eq!(Comparator::from_str("=").unwrap(), Comparator::Equal);
        assert_eq!(Comparator::from_str("==").unwrap(), Comparator::Equal);
        assert_eq!(Comparator::from_str("!=").unwrap(), Comparator::NotEqual);
        assert_eq!(Comparator::from_str("<").unwrap(), Comparator::LessThan);
        assert_eq!(
            Comparator::from_str("<=").unwrap(),
            Comparator::LessThanEqual
        );
        assert_eq!(Comparator::from_str(">").unwrap(), Comparator::GreaterThan);
        assert_eq!(
            Comparator::from_str(">=").unwrap(),
            Comparator::GreaterThanEqual
        );
        assert!(Comparator::from_str("~=").is_err());
    }

    #[test]
    fn test_comparator_as_str() {
        assert_eq!(Comparator::Any.as_str(), "(?)");
        assert_eq!(Comparator::Equal.as_str(), "=");
        assert_eq!(Comparator::NotEqual.as_str(), "!=");
        assert_eq!(Comparator::LessThan.as_str(), "<");
        assert_eq!(Comparator::LessThanEqual.as_str(), "<=");
        assert_eq!(Comparator::GreaterThan.as_str(), ">");
        assert_eq!(Comparator::GreaterThanEqual.as_str(), ">=");
    }

    #[test]
    fn test_comparator_eval() {
        assert!(Comparator::Any.eval("0.0.1", "999.999.999"));
        assert!(Comparator::Equal.eval("1.0", "1.0"));
        assert!(!Comparator::Equal.eval("1.0", "2.0"));
        assert!(Comparator::NotEqual.eval("1.0", "2.0"));
        assert!(!Comparator::NotEqual.eval("1.0", "1.0"));
        assert!(Comparator::LessThan.eval("1.0", "2.0"));
        assert!(!Comparator::LessThan.eval("2.0", "1.0"));
        assert!(Comparator::LessThanEqual.eval("1.0", "1.0"));
        assert!(Comparator::LessThanEqual.eval("1.0", "2.0"));
        assert!(!Comparator::LessThanEqual.eval("2.0", "1.0"));
        assert!(Comparator::GreaterThan.eval("2.0", "1.0"));
        assert!(!Comparator::GreaterThan.eval("1.0", "2.0"));
        assert!(Comparator::GreaterThanEqual.eval("1.0", "1.0"));
        assert!(Comparator::GreaterThanEqual.eval("2.0", "1.0"));
        assert!(!Comparator::GreaterThanEqual.eval("1.0", "2.0"));
    }

    #[test]
    fn test_comparator_display() {
        assert_eq!(format!("{}", Comparator::GreaterThanEqual), ">=");
        assert_eq!(format!("{}", Comparator::Any), "(?)");
    }

    #[test]
    fn test_comparator_default() {
        assert_eq!(Comparator::default(), Comparator::Any);
    }

    #[test]
    fn test_is_operator_char() {
        assert!(is_operator_char('<'));
        assert!(is_operator_char('>'));
        assert!(is_operator_char('!'));
        assert!(is_operator_char('='));
        assert!(!is_operator_char('+'));
        assert!(!is_operator_char(' '));
        assert!(!is_operator_char('a'));
    }

    #[test]
    fn test_is_module_separator() {
        assert!(is_module_separator(','));
        assert!(is_module_separator(' '));
        assert!(is_module_separator('\t'));
        assert!(is_module_separator('\n'));
        assert!(!is_module_separator('a'));
        assert!(!is_module_separator('-'));
    }

    #[test]
    fn test_pkgconf_compatibility_edge_cases() {
        // These cases ensure compatibility with pkgconf's rpmvercmp behaviour
        assert_eq!(compare("1.0.0", "1.0.0"), 0);
        assert!(compare("1.0.0.0", "1.0.0") > 0);
        assert!(compare("1.0", "1.0.0.0") < 0);

        // Pure alpha versions
        assert!(compare("alpha", "beta") < 0);
        assert!(compare("beta", "alpha") > 0);
        assert_eq!(compare("alpha", "alpha"), 0);

        // Number vs alpha at same position: number wins
        assert!(compare("1.1", "1.a") > 0);
    }

    #[test]
    fn test_large_version_numbers() {
        assert!(compare("1.999999999", "1.999999998") > 0);
        assert!(compare("999999999.0", "1.0") > 0);
    }
}
