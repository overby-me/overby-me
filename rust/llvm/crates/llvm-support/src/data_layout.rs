//! Data layout strings.
//!
//! A module's `target datalayout` string decides the size and alignment of
//! every type, so getting it wrong is an ABI break rather than a cosmetic
//! difference. The string is kept verbatim alongside the parsed form and is
//! what gets printed, because re-canonicalising a layout is a way to fail
//! byte-for-byte round trips without ever being more correct.

use core::fmt;

/// Byte order of the target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Endianness {
    Little,
    Big,
}

/// Symbol mangling flavour, the `m:` specification.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mangling {
    Elf,
    GOFF,
    Mips,
    MachO,
    WindowsX86Coff,
    WindowsCoff,
    XCoff,
}

/// Alignment pair in bits: what the ABI requires, and what the target would
/// prefer if it had a free choice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AlignSpec {
    pub abi_bits: u32,
    pub preferred_bits: u32,
}

impl AlignSpec {
    pub fn abi_bytes(self) -> u64 {
        u64::from(self.abi_bits).div_ceil(8)
    }

    pub fn preferred_bytes(self) -> u64 {
        u64::from(self.preferred_bits).div_ceil(8)
    }
}

/// One `p[n]:` entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PointerSpec {
    pub address_space: u32,
    pub size_bits: u32,
    pub align: AlignSpec,
    /// Width of the integer that indexes this pointer, which is the pointer
    /// size unless the target says otherwise.
    pub index_bits: u32,
}

/// Sized alignment entry for integers, floats and vectors.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct SizedAlign {
    size_bits: u32,
    align: AlignSpec,
}

/// Why a data layout string could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataLayoutParseError {
    pub specification: String,
    pub reason: String,
}

impl fmt::Display for DataLayoutParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid data layout specification '{}': {}",
            self.specification, self.reason
        )
    }
}

impl std::error::Error for DataLayoutParseError {}

/// A parsed `target datalayout`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DataLayout {
    raw: String,
    endianness: Endianness,
    mangling: Option<Mangling>,
    stack_natural_align_bits: Option<u32>,
    program_address_space: u32,
    global_address_space: u32,
    alloca_address_space: u32,
    function_pointer_align: Option<FunctionPointerAlign>,
    pointers: Vec<PointerSpec>,
    integers: Vec<SizedAlign>,
    floats: Vec<SizedAlign>,
    vectors: Vec<SizedAlign>,
    aggregate: AlignSpec,
    native_int_widths: Vec<u32>,
    non_integral_address_spaces: Vec<u32>,
}

/// The `F` specification: how much alignment a function pointer carries.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FunctionPointerAlign {
    /// `Fi` means the alignment is independent of the function's own
    /// alignment, `Fn` means it is at least the function's alignment.
    pub independent: bool,
    pub align_bits: u32,
}

impl Default for DataLayout {
    fn default() -> Self {
        Self::from_specs_only("")
    }
}

impl DataLayout {
    /// The layout LLVM assumes when a module names no `target datalayout`.
    fn from_specs_only(raw: &str) -> Self {
        DataLayout {
            raw: raw.to_string(),
            endianness: Endianness::Little,
            mangling: None,
            stack_natural_align_bits: None,
            program_address_space: 0,
            global_address_space: 0,
            alloca_address_space: 0,
            function_pointer_align: None,
            pointers: vec![PointerSpec {
                address_space: 0,
                size_bits: 64,
                align: AlignSpec {
                    abi_bits: 64,
                    preferred_bits: 64,
                },
                index_bits: 64,
            }],
            integers: vec![
                sized(1, 8, 8),
                sized(8, 8, 8),
                sized(16, 16, 16),
                sized(32, 32, 32),
                sized(64, 32, 64),
            ],
            floats: vec![
                sized(16, 16, 16),
                sized(32, 32, 32),
                sized(64, 64, 64),
                sized(128, 128, 128),
            ],
            vectors: vec![sized(64, 64, 64), sized(128, 128, 128)],
            aggregate: AlignSpec {
                abi_bits: 0,
                preferred_bits: 64,
            },
            native_int_widths: Vec::new(),
            non_integral_address_spaces: Vec::new(),
        }
    }

    /// Reads a `target datalayout` string. Unknown specifications are errors,
    /// never ignored: a silently dropped `p1:` entry is a miscompilation on a
    /// target with more than one address space.
    pub fn parse(raw: &str) -> Result<Self, DataLayoutParseError> {
        let mut layout = Self::from_specs_only(raw);
        for spec in raw.split('-').filter(|s| !s.is_empty()) {
            layout.apply(spec)?;
        }
        layout.pointers.sort_by_key(|p| p.address_space);
        layout.integers.sort_by_key(|i| i.size_bits);
        layout.floats.sort_by_key(|f| f.size_bits);
        layout.vectors.sort_by_key(|v| v.size_bits);
        Ok(layout)
    }

    fn apply(&mut self, spec: &str) -> Result<(), DataLayoutParseError> {
        let fail = |reason: &str| DataLayoutParseError {
            specification: spec.to_string(),
            reason: reason.to_string(),
        };
        let mut chars = spec.chars();
        let kind = chars.next().ok_or_else(|| fail("empty specification"))?;
        let rest: String = chars.collect();
        let fields: Vec<&str> = rest.split(':').collect();

        let number = |text: &str, what: &str| -> Result<u32, DataLayoutParseError> {
            text.parse::<u32>()
                .map_err(|_| fail(&format!("{what} is not a number")))
        };

        match kind {
            'e' => self.endianness = Endianness::Little,
            'E' => self.endianness = Endianness::Big,
            'S' => self.stack_natural_align_bits = Some(number(&rest, "stack alignment")?),
            'P' => self.program_address_space = number(&rest, "program address space")?,
            'G' => self.global_address_space = number(&rest, "global address space")?,
            'A' => self.alloca_address_space = number(&rest, "alloca address space")?,
            'm' => {
                if fields.len() != 2 || !fields[0].is_empty() {
                    return Err(fail("mangling takes one field after a colon"));
                }
                self.mangling = Some(match fields[1] {
                    "e" => Mangling::Elf,
                    "l" => Mangling::GOFF,
                    "m" => Mangling::Mips,
                    "o" => Mangling::MachO,
                    "x" => Mangling::WindowsX86Coff,
                    "w" => Mangling::WindowsCoff,
                    "a" => Mangling::XCoff,
                    other => return Err(fail(&format!("unknown mangling '{other}'"))),
                });
            }
            'n' => {
                if let Some(spaces) = rest.strip_prefix("i:") {
                    self.non_integral_address_spaces = spaces
                        .split(':')
                        .map(|f| number(f, "address space"))
                        .collect::<Result<_, _>>()?;
                } else {
                    self.native_int_widths = fields
                        .iter()
                        .map(|f| number(f, "native integer width"))
                        .collect::<Result<_, _>>()?;
                }
            }
            'p' => {
                let address_space = if fields[0].is_empty() {
                    0
                } else {
                    number(fields[0], "address space")?
                };
                if fields.len() < 3 {
                    return Err(fail("pointer needs a size and an ABI alignment"));
                }
                let size_bits = number(fields[1], "pointer size")?;
                let abi_bits = number(fields[2], "ABI alignment")?;
                let preferred_bits = match fields.get(3) {
                    Some(text) => number(text, "preferred alignment")?,
                    None => abi_bits,
                };
                let index_bits = match fields.get(4) {
                    Some(text) => number(text, "index size")?,
                    None => size_bits,
                };
                self.pointers.retain(|p| p.address_space != address_space);
                self.pointers.push(PointerSpec {
                    address_space,
                    size_bits,
                    align: AlignSpec {
                        abi_bits,
                        preferred_bits,
                    },
                    index_bits,
                });
            }
            'i' | 'f' | 'v' => {
                if fields.len() < 2 {
                    return Err(fail("needs a size and an ABI alignment"));
                }
                let size_bits = number(fields[0], "size")?;
                let abi_bits = number(fields[1], "ABI alignment")?;
                let preferred_bits = match fields.get(2) {
                    Some(text) => number(text, "preferred alignment")?,
                    None => abi_bits,
                };
                let entry = SizedAlign {
                    size_bits,
                    align: AlignSpec {
                        abi_bits,
                        preferred_bits,
                    },
                };
                let table = match kind {
                    'i' => &mut self.integers,
                    'f' => &mut self.floats,
                    _ => &mut self.vectors,
                };
                table.retain(|e| e.size_bits != size_bits);
                table.push(entry);
            }
            'a' => {
                if fields.len() < 2 || !fields[0].is_empty() {
                    return Err(fail("aggregate alignment takes no size"));
                }
                let abi_bits = number(fields[1], "ABI alignment")?;
                let preferred_bits = match fields.get(2) {
                    Some(text) => number(text, "preferred alignment")?,
                    None => abi_bits,
                };
                self.aggregate = AlignSpec {
                    abi_bits,
                    preferred_bits,
                };
            }
            'F' => {
                let mut inner = rest.chars();
                let flavour = inner
                    .next()
                    .ok_or_else(|| fail("function pointer alignment needs i or n"))?;
                let independent = match flavour {
                    'i' => true,
                    'n' => false,
                    other => {
                        return Err(fail(&format!("unknown function pointer flavour '{other}'")));
                    }
                };
                let align_bits = number(&inner.collect::<String>(), "alignment")?;
                self.function_pointer_align = Some(FunctionPointerAlign {
                    independent,
                    align_bits,
                });
            }
            other => return Err(fail(&format!("unknown specification '{other}'"))),
        }
        Ok(())
    }

    /// The string this layout was parsed from, which is what a module prints.
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    pub fn endianness(&self) -> Endianness {
        self.endianness
    }

    pub fn is_little_endian(&self) -> bool {
        self.endianness == Endianness::Little
    }

    pub fn mangling(&self) -> Option<Mangling> {
        self.mangling
    }

    pub fn stack_natural_align_bits(&self) -> Option<u32> {
        self.stack_natural_align_bits
    }

    pub fn program_address_space(&self) -> u32 {
        self.program_address_space
    }

    pub fn global_address_space(&self) -> u32 {
        self.global_address_space
    }

    pub fn alloca_address_space(&self) -> u32 {
        self.alloca_address_space
    }

    pub fn function_pointer_align(&self) -> Option<FunctionPointerAlign> {
        self.function_pointer_align
    }

    pub fn native_int_widths(&self) -> &[u32] {
        &self.native_int_widths
    }

    pub fn is_non_integral_address_space(&self, address_space: u32) -> bool {
        self.non_integral_address_spaces.contains(&address_space)
    }

    /// The pointer specification for an address space, falling back to address
    /// space zero the way LLVM does when a target names no entry for it.
    pub fn pointer_spec(&self, address_space: u32) -> PointerSpec {
        self.pointers
            .iter()
            .find(|p| p.address_space == address_space)
            .or_else(|| self.pointers.iter().find(|p| p.address_space == 0))
            .copied()
            .unwrap_or(PointerSpec {
                address_space,
                size_bits: 64,
                align: AlignSpec {
                    abi_bits: 64,
                    preferred_bits: 64,
                },
                index_bits: 64,
            })
    }

    pub fn pointer_size_bits(&self, address_space: u32) -> u32 {
        self.pointer_spec(address_space).size_bits
    }

    pub fn pointer_index_bits(&self, address_space: u32) -> u32 {
        self.pointer_spec(address_space).index_bits
    }

    /// Alignment of an `iN`.
    ///
    /// An exact entry wins; otherwise the smallest entry wider than `bits`
    /// applies, and if there is none, the widest entry does. That last clause
    /// is why `i128` on x86-64 lands on the `i64:64:64` entry.
    pub fn integer_align(&self, bits: u32) -> AlignSpec {
        Self::sized_lookup(&self.integers, bits).unwrap_or(AlignSpec {
            abi_bits: 8,
            preferred_bits: 8,
        })
    }

    pub fn float_align(&self, bits: u32) -> AlignSpec {
        Self::sized_lookup(&self.floats, bits).unwrap_or(AlignSpec {
            abi_bits: bits,
            preferred_bits: bits,
        })
    }

    /// Alignment of a vector. Unlike integers, a vector with no entry gets its
    /// natural alignment: the size rounded up to a power of two.
    pub fn vector_align(&self, bits: u32) -> AlignSpec {
        if let Some(exact) = self.vectors.iter().find(|v| v.size_bits == bits) {
            return exact.align;
        }
        let natural = bits.next_power_of_two();
        AlignSpec {
            abi_bits: natural,
            preferred_bits: natural,
        }
    }

    pub fn aggregate_align(&self) -> AlignSpec {
        self.aggregate
    }

    fn sized_lookup(table: &[SizedAlign], bits: u32) -> Option<AlignSpec> {
        if let Some(exact) = table.iter().find(|e| e.size_bits == bits) {
            return Some(exact.align);
        }
        table
            .iter()
            .filter(|e| e.size_bits > bits)
            .min_by_key(|e| e.size_bits)
            .or_else(|| table.iter().max_by_key(|e| e.size_bits))
            .map(|e| e.align)
    }
}

const fn sized(size_bits: u32, abi_bits: u32, preferred_bits: u32) -> SizedAlign {
    SizedAlign {
        size_bits,
        align: AlignSpec {
            abi_bits,
            preferred_bits,
        },
    }
}

impl fmt::Display for DataLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What rustc puts in every x86_64-unknown-linux-gnu module.
    const X86_64_LINUX: &str =
        "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128";

    #[test]
    fn round_trips_verbatim() {
        let layout = DataLayout::parse(X86_64_LINUX).unwrap();
        assert_eq!(layout.as_str(), X86_64_LINUX);
        assert_eq!(layout.to_string(), X86_64_LINUX);
    }

    #[test]
    fn reads_the_x86_64_linux_layout() {
        let layout = DataLayout::parse(X86_64_LINUX).unwrap();
        assert_eq!(layout.endianness(), Endianness::Little);
        assert_eq!(layout.mangling(), Some(Mangling::Elf));
        assert_eq!(layout.stack_natural_align_bits(), Some(128));
        assert_eq!(layout.native_int_widths(), &[8, 16, 32, 64]);
        // The three x86 segment address spaces.
        assert_eq!(layout.pointer_size_bits(270), 32);
        assert_eq!(layout.pointer_size_bits(271), 32);
        assert_eq!(layout.pointer_size_bits(272), 64);
        // Unlisted address spaces fall back to the default pointer.
        assert_eq!(layout.pointer_size_bits(0), 64);
        assert_eq!(layout.pointer_size_bits(42), 64);
        assert_eq!(layout.integer_align(64).abi_bits, 64);
        assert_eq!(layout.integer_align(128).abi_bits, 128);
        assert_eq!(layout.float_align(80).abi_bits, 128);
    }

    #[test]
    fn integer_alignment_falls_back_by_the_upstream_rule() {
        let layout = DataLayout::parse("e-i8:8-i16:16-i32:32-i64:64").unwrap();
        // Exact.
        assert_eq!(layout.integer_align(32).abi_bits, 32);
        // Smallest larger.
        assert_eq!(layout.integer_align(24).abi_bits, 32);
        assert_eq!(layout.integer_align(1).abi_bits, 8);
        // Nothing larger, so the widest entry.
        assert_eq!(layout.integer_align(128).abi_bits, 64);
        assert_eq!(layout.integer_align(256).abi_bits, 64);
    }

    #[test]
    fn vectors_get_natural_alignment_when_unlisted() {
        let layout = DataLayout::parse("e-v128:128").unwrap();
        assert_eq!(layout.vector_align(128).abi_bits, 128);
        assert_eq!(layout.vector_align(256).abi_bits, 256);
        assert_eq!(layout.vector_align(96).abi_bits, 128);
    }

    #[test]
    fn defaults_match_an_empty_layout() {
        let layout = DataLayout::parse("").unwrap();
        assert_eq!(layout.endianness(), Endianness::Little);
        assert_eq!(layout.pointer_size_bits(0), 64);
        assert_eq!(layout.integer_align(1).abi_bits, 8);
        assert_eq!(layout.integer_align(64).abi_bits, 32);
        assert_eq!(layout.aggregate_align().preferred_bits, 64);
    }

    #[test]
    fn big_endian_and_index_widths() {
        let layout = DataLayout::parse("E-p:32:32:32:16-a:0:64").unwrap();
        assert_eq!(layout.endianness(), Endianness::Big);
        assert!(!layout.is_little_endian());
        assert_eq!(layout.pointer_size_bits(0), 32);
        assert_eq!(layout.pointer_index_bits(0), 16);
        assert_eq!(layout.aggregate_align().abi_bits, 0);
    }

    #[test]
    fn non_integral_and_function_pointer_specifications() {
        let layout = DataLayout::parse("e-ni:1:2-Fn32").unwrap();
        assert!(layout.is_non_integral_address_space(1));
        assert!(layout.is_non_integral_address_space(2));
        assert!(!layout.is_non_integral_address_space(0));
        assert_eq!(
            layout.function_pointer_align(),
            Some(FunctionPointerAlign {
                independent: false,
                align_bits: 32
            })
        );
    }

    #[test]
    fn unknown_specifications_are_rejected() {
        assert!(DataLayout::parse("e-Q42").is_err());
        assert!(DataLayout::parse("e-m:q").is_err());
        assert!(DataLayout::parse("e-p:64").is_err());
        assert!(DataLayout::parse("e-i").is_err());
        let error = DataLayout::parse("e-Q42").unwrap_err();
        assert!(error.to_string().contains("Q42"));
    }

    #[test]
    fn later_specifications_replace_earlier_ones() {
        let layout = DataLayout::parse("e-i32:32-i32:64").unwrap();
        assert_eq!(layout.integer_align(32).abi_bits, 64);
        let layout = DataLayout::parse("e-p:64:64-p:32:32").unwrap();
        assert_eq!(layout.pointer_size_bits(0), 32);
    }

    #[test]
    fn several_real_target_layouts_parse() {
        // aarch64-unknown-linux-gnu, wasm32, and a big-endian s390x, so the
        // parser is exercised past the one target we care about first.
        let layouts = [
            "e-m:e-i8:8:32-i16:16:32-i64:64-i128:128-n32:64-S128-Fn32",
            "e-m:e-p:32:32-p10:8:8-p20:8:8-i64:64-i128:128-n32:64-S128-ni:1:10:20",
            "E-m:e-i1:8:16-i8:8:16-i64:64-f128:64-v128:64-a:8:16-n32:64",
        ];
        for text in layouts {
            let layout = DataLayout::parse(text).unwrap();
            assert_eq!(layout.as_str(), text);
        }
    }
}
