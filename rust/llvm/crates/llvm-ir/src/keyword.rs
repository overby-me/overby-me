//! One macro, used everywhere a piece of the IR is a fixed set of keywords.
//!
//! Writing the enum and both directions of the keyword mapping by hand is how
//! a variant ends up parsing to one spelling and printing as another, so the
//! two halves are generated from a single list.

macro_rules! define_keyword_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
        pub enum $name {
            $($variant),*
        }

        impl $name {
            /// How this variant is spelled in the IR.
            pub fn keyword(self) -> &'static str {
                match self {
                    $($name::$variant => $text),*
                }
            }

            /// The variant a keyword names, or `None` when the keyword is not
            /// one of these.
            pub fn from_keyword(text: &str) -> Option<$name> {
                Some(match text {
                    $($text => $name::$variant,)*
                    _ => return None,
                })
            }
        }
    };
}

pub(crate) use define_keyword_enum;
