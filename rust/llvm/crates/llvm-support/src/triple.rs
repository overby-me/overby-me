//! Target triples.
//!
//! A triple is `arch-vendor-os` with an optional fourth environment
//! component, except that real triples drop components, and the fourth one is
//! sometimes an object format instead. Like [`crate::DataLayout`], a `Triple`
//! keeps its source string and prints it back unchanged; the parsed view is
//! for asking questions, not for rewriting the module.

use core::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arch {
    X86,
    X86_64,
    Arm,
    Aarch64,
    Aarch64Be,
    RiscV32,
    RiscV64,
    Wasm32,
    Wasm64,
    PowerPc64,
    PowerPc64Le,
    SystemZ,
    LoongArch64,
    /// A triple naming an architecture this crate has no opinion about. Not an
    /// error: a module for an unknown target still parses and prints.
    Unknown,
}

impl Arch {
    /// Pointer width in bits, or `None` when the architecture is unknown.
    pub fn pointer_width(self) -> Option<u32> {
        Some(match self {
            Arch::X86 | Arch::Arm | Arch::RiscV32 | Arch::Wasm32 => 32,
            Arch::X86_64
            | Arch::Aarch64
            | Arch::Aarch64Be
            | Arch::RiscV64
            | Arch::Wasm64
            | Arch::PowerPc64
            | Arch::PowerPc64Le
            | Arch::SystemZ
            | Arch::LoongArch64 => 64,
            Arch::Unknown => return None,
        })
    }

    fn from_component(text: &str) -> Self {
        match text {
            "i386" | "i486" | "i586" | "i686" | "i786" | "i886" | "i986" => Arch::X86,
            "x86_64" | "amd64" | "x86_64h" => Arch::X86_64,
            "aarch64" | "arm64" | "arm64e" => Arch::Aarch64,
            "aarch64_be" => Arch::Aarch64Be,
            "riscv32" => Arch::RiscV32,
            "riscv64" => Arch::RiscV64,
            "wasm32" => Arch::Wasm32,
            "wasm64" => Arch::Wasm64,
            "powerpc64" | "ppc64" => Arch::PowerPc64,
            "powerpc64le" | "ppc64le" => Arch::PowerPc64Le,
            "s390x" => Arch::SystemZ,
            "loongarch64" => Arch::LoongArch64,
            // arm, armv7, thumbv7 and friends all land here.
            other if other.starts_with("arm") || other.starts_with("thumb") => Arch::Arm,
            _ => Arch::Unknown,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Vendor {
    Apple,
    Pc,
    Ibm,
    Amd,
    Nvidia,
    UnknownVendor,
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Os {
    Linux,
    Darwin,
    MacOsx,
    Ios,
    Windows,
    FreeBsd,
    NetBsd,
    OpenBsd,
    Wasi,
    Emscripten,
    Fuchsia,
    Uefi,
    /// The `none` component of a bare-metal triple.
    None,
    Unknown,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Env {
    Gnu,
    GnuEabi,
    GnuEabiHf,
    GnuAbi64,
    Musl,
    MuslEabi,
    MuslEabiHf,
    Msvc,
    Android,
    Eabi,
    EabiHf,
    Elf,
    Macho,
    Unknown,
}

/// A target triple: the source string plus the parsed components.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Triple {
    raw: String,
    arch_component: String,
    vendor_component: String,
    os_component: String,
    env_component: String,
    arch: Arch,
    vendor: Vendor,
    os: Os,
    env: Env,
}

impl Triple {
    pub fn parse(raw: &str) -> Self {
        let mut parts = raw.splitn(4, '-');
        let arch_component = parts.next().unwrap_or_default().to_string();
        let vendor_component = parts.next().unwrap_or_default().to_string();
        let os_component = parts.next().unwrap_or_default().to_string();
        let env_component = parts.next().unwrap_or_default().to_string();

        // Triples routinely omit the vendor: `wasm32-wasi` and
        // `thumbv7em-none-eabihf` both name an operating system where a vendor
        // belongs. When the vendor slot holds an OS and the OS slot does not,
        // every later component shifts one place left.
        let (vendor_component, os_component, env_component) = if parse_os(&vendor_component)
            != Os::Unknown
            && parse_os(&os_component) == Os::Unknown
            && env_component.is_empty()
        {
            (String::new(), vendor_component, os_component)
        } else {
            (vendor_component, os_component, env_component)
        };

        let arch = Arch::from_component(&arch_component);
        let vendor = match vendor_component.as_str() {
            "apple" => Vendor::Apple,
            "pc" => Vendor::Pc,
            "ibm" => Vendor::Ibm,
            "amd" => Vendor::Amd,
            "nvidia" => Vendor::Nvidia,
            "unknown" => Vendor::UnknownVendor,
            _ => Vendor::Unknown,
        };
        let os = parse_os(&os_component);
        let env = match env_component.as_str() {
            "gnu" => Env::Gnu,
            "gnueabi" => Env::GnuEabi,
            "gnueabihf" => Env::GnuEabiHf,
            "gnuabi64" => Env::GnuAbi64,
            "musl" => Env::Musl,
            "musleabi" => Env::MuslEabi,
            "musleabihf" => Env::MuslEabiHf,
            "msvc" => Env::Msvc,
            "android" => Env::Android,
            "eabi" => Env::Eabi,
            "eabihf" => Env::EabiHf,
            "elf" => Env::Elf,
            "macho" => Env::Macho,
            _ => Env::Unknown,
        };

        Triple {
            raw: raw.to_string(),
            arch_component,
            vendor_component,
            os_component,
            env_component,
            arch,
            vendor,
            os,
            env,
        }
    }

    /// The string this triple was parsed from, which is what a module prints.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    pub fn arch(&self) -> Arch {
        self.arch
    }

    pub fn vendor(&self) -> Vendor {
        self.vendor
    }

    pub fn os(&self) -> Os {
        self.os
    }

    pub fn env(&self) -> Env {
        self.env
    }

    pub fn arch_name(&self) -> &str {
        &self.arch_component
    }

    pub fn vendor_name(&self) -> &str {
        &self.vendor_component
    }

    pub fn os_name(&self) -> &str {
        &self.os_component
    }

    pub fn env_name(&self) -> &str {
        &self.env_component
    }

    pub fn pointer_width(&self) -> Option<u32> {
        self.arch.pointer_width()
    }

    pub fn is_elf(&self) -> bool {
        !matches!(
            self.os,
            Os::Darwin | Os::MacOsx | Os::Ios | Os::Windows | Os::Uefi
        ) && !matches!(self.arch, Arch::Wasm32 | Arch::Wasm64)
    }

    pub fn is_macho(&self) -> bool {
        matches!(self.os, Os::Darwin | Os::MacOsx | Os::Ios)
    }
}

fn parse_os(text: &str) -> Os {
    match text {
        "linux" => Os::Linux,
        "darwin" => Os::Darwin,
        "macosx" => Os::MacOsx,
        "ios" => Os::Ios,
        "windows" | "win32" => Os::Windows,
        "freebsd" => Os::FreeBsd,
        "netbsd" => Os::NetBsd,
        "openbsd" => Os::OpenBsd,
        "wasi" => Os::Wasi,
        "emscripten" => Os::Emscripten,
        "fuchsia" => Os::Fuchsia,
        "uefi" => Os::Uefi,
        "none" => Os::None,
        // Versioned operating systems keep their version in the component,
        // for example `macosx14.0.0`.
        other if other.starts_with("darwin") => Os::Darwin,
        other if other.starts_with("macosx") => Os::MacOsx,
        other if other.starts_with("ios") => Os::Ios,
        _ => Os::Unknown,
    }
}

impl fmt::Display for Triple {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_verbatim() {
        for text in [
            "x86_64-unknown-linux-gnu",
            "aarch64-apple-darwin",
            "wasm32-unknown-unknown",
            "",
            "something-entirely-made-up-here",
        ] {
            assert_eq!(Triple::parse(text).as_str(), text);
            assert_eq!(Triple::parse(text).to_string(), text);
        }
    }

    #[test]
    fn reads_the_targets_we_care_about_first() {
        let t = Triple::parse("x86_64-unknown-linux-gnu");
        assert_eq!(t.arch(), Arch::X86_64);
        assert_eq!(t.vendor(), Vendor::UnknownVendor);
        assert_eq!(t.os(), Os::Linux);
        assert_eq!(t.env(), Env::Gnu);
        assert_eq!(t.pointer_width(), Some(64));
        assert!(t.is_elf());
        assert!(!t.is_macho());

        let t = Triple::parse("aarch64-unknown-linux-gnu");
        assert_eq!(t.arch(), Arch::Aarch64);
        assert_eq!(t.pointer_width(), Some(64));

        let t = Triple::parse("aarch64-apple-darwin");
        assert_eq!(t.vendor(), Vendor::Apple);
        assert_eq!(t.os(), Os::Darwin);
        assert!(t.is_macho());
        assert!(!t.is_elf());
    }

    #[test]
    fn handles_short_and_versioned_triples() {
        let t = Triple::parse("wasm32-wasi");
        assert_eq!(t.arch(), Arch::Wasm32);
        assert_eq!(t.os(), Os::Wasi);
        assert_eq!(t.vendor_name(), "");
        assert!(!t.is_elf());

        let t = Triple::parse("arm64-apple-macosx14.0.0");
        assert_eq!(t.arch(), Arch::Aarch64);
        assert_eq!(t.os(), Os::MacOsx);
        assert_eq!(t.os_name(), "macosx14.0.0");

        let t = Triple::parse("thumbv7em-none-eabihf");
        assert_eq!(t.arch(), Arch::Arm);
        assert_eq!(t.os(), Os::None);
        assert_eq!(t.env(), Env::EabiHf);
        assert_eq!(t.pointer_width(), Some(32));
    }

    #[test]
    fn unknown_components_do_not_fail() {
        let t = Triple::parse("sparcv9-sun-solaris");
        assert_eq!(t.arch(), Arch::Unknown);
        assert_eq!(t.pointer_width(), None);
        assert_eq!(t.arch_name(), "sparcv9");
        assert_eq!(t.vendor_name(), "sun");
        assert_eq!(t.os_name(), "solaris");
    }

    #[test]
    fn the_fourth_component_can_hold_anything() {
        let t = Triple::parse("x86_64-unknown-linux-gnu-extra-bits");
        assert_eq!(t.env_name(), "gnu-extra-bits");
        assert_eq!(t.as_str(), "x86_64-unknown-linux-gnu-extra-bits");
    }
}
