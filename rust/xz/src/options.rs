//! Shared option/flag types for the xz CLI.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Compress,
    Decompress,
    Test,
    /// `-l`/`--list`: list information about .xz files.
    List,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Auto-detect on decompress, default xz on compress. (Same as
    /// upstream xz's `--format=auto`, which is the default.)
    Auto,
    /// `.xz` container.
    Xz,
    /// LZMA-alone (`.lzma`) container.
    Lzma,
    /// Lzip (`.lz`) container — decode-only for now.
    Lzip,
    /// Raw filter chain — no container, no integrity check, no
    /// magic bytes. Requires an explicit filter chain (e.g.
    /// `--lzma2=preset=N` or `--filters=…`) and a `--suffix=`.
    Raw,
}

/// One element of a parsed filter chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    /// LZMA1 filter using `preset=N`. Only valid as the final filter.
    Lzma1Preset(u32),
    /// LZMA2 filter using `preset=N`. Only valid as the final filter.
    Lzma2Preset(u32),
    /// Delta filter with the default distance (1) — used by upstream
    /// `test_compress.sh`.
    Delta,
    /// BCJ filter selecting one of liblzma's branch-call-jump filters.
    Bcj(BcjArch),
}

/// Branch/call/jump (BCJ) filter architecture selector. These match
/// xz's `--x86`, `--arm`, `--arm64`, `--armthumb`, `--powerpc`,
/// `--ia64`, `--sparc`, `--riscv` flags one-to-one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BcjArch {
    X86,
    Arm,
    Arm64,
    ArmThumb,
    PowerPc,
    Ia64,
    Sparc,
    RiscV,
}

/// Parsed filter chain. The order matches the order the filters were
/// given on the command line; LZMA1/LZMA2 must be the last entry.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FilterChain(pub Vec<FilterKind>);

impl FilterChain {
    pub fn push(&mut self, k: FilterKind) {
        self.0.push(k);
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn as_slice(&self) -> &[FilterKind] {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub mode: Mode,
    pub stdout: bool,
    pub keep: bool,
    pub force: bool,
    pub level: u32,
    pub verbose: bool,
    pub quiet: bool,
    pub format: Format,
    pub no_warn: bool,
    /// Custom output suffix from `--suffix=...`. Used in compress
    /// mode (replaces `.xz`/`.lzma`) and required in decompress
    /// mode for raw format.
    pub suffix: Option<String>,
    /// Filter chain from `--lzma1=`/`--lzma2=`/`--filters=`/BCJ
    /// flags. Only meaningful in raw format mode (or paired with
    /// LZMA2 in xz-format compression).
    pub filter: Option<FilterChain>,
    pub files: Vec<String>,
}

impl Options {
    /// Defaults derived from the program name (argv[0]).
    pub fn defaults_for(prog_name: &str) -> Self {
        let (mode, stdout, format) = match prog_name {
            "unxz" => (Mode::Decompress, false, Format::Auto),
            "xzcat" => (Mode::Decompress, true, Format::Auto),
            "lzma" => (Mode::Compress, false, Format::Lzma),
            "unlzma" => (Mode::Decompress, false, Format::Auto),
            "lzcat" => (Mode::Decompress, true, Format::Auto),
            _ => (Mode::Compress, false, Format::Auto),
        };
        Self {
            mode,
            stdout,
            keep: false,
            force: false,
            level: 6,
            verbose: false,
            quiet: false,
            format,
            no_warn: false,
            suffix: None,
            filter: None,
            files: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_for_xz() {
        let o = Options::defaults_for("xz");
        assert_eq!(o.mode, Mode::Compress);
        assert!(!o.stdout);
        assert_eq!(o.format, Format::Auto);
        assert_eq!(o.level, 6);
    }

    #[test]
    fn defaults_for_unxz() {
        let o = Options::defaults_for("unxz");
        assert_eq!(o.mode, Mode::Decompress);
        assert!(!o.stdout);
    }

    #[test]
    fn defaults_for_xzcat() {
        let o = Options::defaults_for("xzcat");
        assert_eq!(o.mode, Mode::Decompress);
        assert!(o.stdout);
    }

    #[test]
    fn defaults_for_lzma_family() {
        assert_eq!(Options::defaults_for("lzma").format, Format::Lzma);
        assert_eq!(Options::defaults_for("lzma").mode, Mode::Compress);
        assert_eq!(Options::defaults_for("unlzma").mode, Mode::Decompress);
        assert_eq!(Options::defaults_for("lzcat").stdout, true);
        assert_eq!(Options::defaults_for("lzcat").format, Format::Auto);
    }

    #[test]
    fn filter_chain_default_is_empty() {
        let c = FilterChain::default();
        assert!(c.is_empty());
    }

    #[test]
    fn filter_chain_push_preserves_order() {
        let mut c = FilterChain::default();
        c.push(FilterKind::Bcj(BcjArch::X86));
        c.push(FilterKind::Lzma2Preset(0));
        assert_eq!(c.as_slice().len(), 2);
        assert_eq!(c.as_slice()[0], FilterKind::Bcj(BcjArch::X86));
    }
}
