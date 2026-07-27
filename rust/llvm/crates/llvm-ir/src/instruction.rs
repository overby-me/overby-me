//! Instructions.
//!
//! One `Instruction` is a result name, a result type, a kind holding the
//! operands, and any metadata attachments. Instructions live in a per-function
//! arena and are referred to by `InstId`, so an id stays valid when the
//! surrounding block is rewritten.
//!
//! Flags are grouped rather than repeated per opcode: an `add` can carry `nuw`
//! and `nsw`, an `or` can carry `disjoint`, and a `zext` can carry `nneg`, but
//! they all print in one fixed order and the verifier is what says which
//! opcode may set which flag.

use crate::attribute::AttributeSet;
use crate::constant::{CastOp, GepFlags};
use crate::keyword::define_keyword_enum;
use crate::metadata::{MdAttachment, MdOperand};
use crate::types::TypeId;
use crate::value::{BlockId, Name, Value};
use llvm_support::Align;

/// Integer and pointer flags, printed in the order they are declared here.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct IntFlags {
    pub nuw: bool,
    pub nsw: bool,
    pub exact: bool,
    pub disjoint: bool,
    pub nneg: bool,
    pub samesign: bool,
}

impl IntFlags {
    pub fn is_empty(self) -> bool {
        self == IntFlags::default()
    }
}

/// Fast-math flags. `fast` is the spelling when every one of them is set.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct FastMathFlags {
    pub nnan: bool,
    pub ninf: bool,
    pub nsz: bool,
    pub arcp: bool,
    pub contract: bool,
    pub afn: bool,
    pub reassoc: bool,
}

impl FastMathFlags {
    pub fn is_empty(self) -> bool {
        self == FastMathFlags::default()
    }

    pub fn is_fast(self) -> bool {
        self.nnan && self.ninf && self.nsz && self.arcp && self.contract && self.afn && self.reassoc
    }

    pub fn all() -> FastMathFlags {
        FastMathFlags {
            nnan: true,
            ninf: true,
            nsz: true,
            arcp: true,
            contract: true,
            afn: true,
            reassoc: true,
        }
    }

    pub fn set_by_keyword(&mut self, keyword: &str) -> bool {
        match keyword {
            "nnan" => self.nnan = true,
            "ninf" => self.ninf = true,
            "nsz" => self.nsz = true,
            "arcp" => self.arcp = true,
            "contract" => self.contract = true,
            "afn" => self.afn = true,
            "reassoc" => self.reassoc = true,
            "fast" => *self = FastMathFlags::all(),
            _ => return false,
        }
        true
    }
}

define_keyword_enum! {
    /// Binary operators, integer and floating point.
    BinOp {
        Add => "add",
        FAdd => "fadd",
        Sub => "sub",
        FSub => "fsub",
        Mul => "mul",
        FMul => "fmul",
        UDiv => "udiv",
        SDiv => "sdiv",
        FDiv => "fdiv",
        URem => "urem",
        SRem => "srem",
        FRem => "frem",
        Shl => "shl",
        LShr => "lshr",
        AShr => "ashr",
        And => "and",
        Or => "or",
        Xor => "xor",
    }
}

impl BinOp {
    pub fn is_floating_point(self) -> bool {
        matches!(
            self,
            BinOp::FAdd | BinOp::FSub | BinOp::FMul | BinOp::FDiv | BinOp::FRem
        )
    }
}

define_keyword_enum! {
    /// Integer comparison predicates.
    IntPredicate {
        Eq => "eq",
        Ne => "ne",
        Ugt => "ugt",
        Uge => "uge",
        Ult => "ult",
        Ule => "ule",
        Sgt => "sgt",
        Sge => "sge",
        Slt => "slt",
        Sle => "sle",
    }
}

define_keyword_enum! {
    /// Floating-point comparison predicates.
    FloatPredicate {
        False => "false",
        Oeq => "oeq",
        Ogt => "ogt",
        Oge => "oge",
        Olt => "olt",
        Ole => "ole",
        One => "one",
        Ord => "ord",
        Ueq => "ueq",
        Ugt => "ugt",
        Uge => "uge",
        Ult => "ult",
        Ule => "ule",
        Une => "une",
        Uno => "uno",
        True => "true",
    }
}

define_keyword_enum! {
    /// Memory ordering on an atomic operation.
    AtomicOrdering {
        Unordered => "unordered",
        Monotonic => "monotonic",
        Acquire => "acquire",
        Release => "release",
        AcqRel => "acq_rel",
        SeqCst => "seq_cst",
    }
}

define_keyword_enum! {
    /// The read-modify-write operations of `atomicrmw`.
    AtomicRmwOp {
        Xchg => "xchg",
        Add => "add",
        Sub => "sub",
        And => "and",
        Nand => "nand",
        Or => "or",
        Xor => "xor",
        Max => "max",
        Min => "min",
        UMax => "umax",
        UMin => "umin",
        FAdd => "fadd",
        FSub => "fsub",
        FMax => "fmax",
        FMin => "fmin",
        FMaximum => "fmaximum",
        FMinimum => "fminimum",
        UIncWrap => "uinc_wrap",
        UDecWrap => "udec_wrap",
        USubCond => "usub_cond",
        USubSat => "usub_sat",
    }
}

define_keyword_enum! {
    /// Calling conventions upstream prints by name. Anything else is written
    /// `cc<number>`, with no space, which is why `CallingConv::Numbered`
    /// exists next to these.
    NamedCallingConv {
        Fast => "fastcc",
        Cold => "coldcc",
        GHC => "ghccc",
        AnyReg => "anyregcc",
        PreserveMost => "preserve_mostcc",
        PreserveAll => "preserve_allcc",
        PreserveNone => "preserve_nonecc",
        CxxFastTls => "cxx_fast_tlscc",
        Tail => "tailcc",
        Graal => "graalcc",
        CfGuardCheck => "cfguard_checkcc",
        Swift => "swiftcc",
        SwiftTail => "swifttailcc",
        X86StdCall => "x86_stdcallcc",
        X86FastCall => "x86_fastcallcc",
        X86ThisCall => "x86_thiscallcc",
        X86VectorCall => "x86_vectorcallcc",
        X86RegCall => "x86_regcallcc",
        X86Intr => "x86_intrcc",
        ArmApcs => "arm_apcscc",
        ArmAapcs => "arm_aapcscc",
        ArmAapcsVfp => "arm_aapcs_vfpcc",
        Aarch64VectorPcs => "aarch64_vector_pcs",
        Aarch64SveVectorPcs => "aarch64_sve_vector_pcs",
        Win64 => "win64cc",
        X8664SysV => "x86_64_sysvcc",
        AmdgpuCsChain => "amdgpu_cs_chain",
        AmdgpuCsChainPreserve => "amdgpu_cs_chain_preserve",
        AmdgpuCs => "amdgpu_cs",
        AmdgpuKernel => "amdgpu_kernel",
        AmdgpuGfx => "amdgpu_gfx",
        AmdgpuVs => "amdgpu_vs",
        AmdgpuLs => "amdgpu_ls",
        AmdgpuHs => "amdgpu_hs",
        AmdgpuEs => "amdgpu_es",
        AmdgpuGs => "amdgpu_gs",
        AmdgpuPs => "amdgpu_ps",
        SpirKernel => "spir_kernel",
        SpirFunc => "spir_func",
        PtxKernel => "ptx_kernel",
        PtxDevice => "ptx_device",
        RiscvVectorCc => "riscv_vector_cc",
    }
}

/// A calling convention: either one upstream spells out, or a raw number.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum CallingConv {
    /// The C convention, which is the default and prints as nothing.
    #[default]
    C,
    Named(NamedCallingConv),
    Numbered(u32),
    /// `riscv_vls_cc(32)`, the one convention that takes an argument. The
    /// number is the vector length its ABI is fixed to.
    RiscvVls(u32),
}

/// The scope an atomic operation synchronises with. `None` is the system
/// scope, which prints as nothing.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct SyncScope(pub Option<String>);

impl SyncScope {
    pub fn system() -> SyncScope {
        SyncScope(None)
    }

    pub fn single_thread() -> SyncScope {
        SyncScope(Some("singlethread".to_string()))
    }
}

/// How a call is marked for tail-call treatment.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum TailKind {
    #[default]
    None,
    Tail,
    MustTail,
    NoTail,
}

impl TailKind {
    pub fn keyword(self) -> Option<&'static str> {
        match self {
            TailKind::None => None,
            TailKind::Tail => Some("tail"),
            TailKind::MustTail => Some("musttail"),
            TailKind::NoTail => Some("notail"),
        }
    }
}

/// One argument of a call, with the attributes that apply to it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CallArg {
    pub ty: TypeId,
    pub attrs: AttributeSet,
    pub value: Value,
}

/// An operand bundle: `[ "deopt"(i32 1) ]`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OperandBundle {
    pub tag: String,
    pub args: Vec<(TypeId, Value)>,
}

/// Everything shared by `call`, `invoke` and `callbr`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CallData {
    pub tail: TailKind,
    pub fast_math: FastMathFlags,
    pub calling_conv: CallingConv,
    pub return_attrs: AttributeSet,
    /// The callee's function type, which opaque pointers make necessary to
    /// write out whenever the call is variadic and useful always.
    pub function_type: TypeId,
    /// `call addrspace(1) void @f()`.
    pub address_space: Option<u32>,
    pub callee: Value,
    pub args: Vec<CallArg>,
    pub fn_attrs: AttributeSet,
    pub bundles: Vec<OperandBundle>,
}

/// A clause of a `landingpad`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LandingPadClause {
    Catch { ty: TypeId, value: Value },
    Filter { ty: TypeId, value: Value },
}

impl InstKind {
    /// The ordering an atomic memory operation carries, if it is one.
    pub fn atomic_ordering(&self) -> Option<(SyncScope, AtomicOrdering)> {
        match self {
            InstKind::Load { atomic, .. } | InstKind::Store { atomic, .. } => atomic.clone(),
            _ => None,
        }
    }
}

/// Where a funclet-style terminator unwinds to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnwindTarget {
    Caller,
    Block(BlockId),
}

/// The operands of one instruction.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum InstKind {
    // Terminators.
    /// `ret void`, or `ret <ty> <value>`: the type is the function's result
    /// type, which the instruction's own `void` type cannot supply.
    Ret(Option<(TypeId, Value)>),
    Br {
        target: BlockId,
    },
    CondBr {
        condition: Value,
        if_true: BlockId,
        if_false: BlockId,
    },
    Switch {
        value_type: TypeId,
        value: Value,
        default: BlockId,
        cases: Vec<(Value, BlockId)>,
    },
    IndirectBr {
        address: Value,
        destinations: Vec<BlockId>,
    },
    Invoke {
        call: Box<CallData>,
        normal: BlockId,
        unwind: BlockId,
    },
    CallBr {
        call: Box<CallData>,
        fallthrough: BlockId,
        indirect: Vec<BlockId>,
    },
    Resume {
        ty: TypeId,
        value: Value,
    },
    CatchSwitch {
        parent: Value,
        handlers: Vec<BlockId>,
        unwind: UnwindTarget,
    },
    CatchRet {
        pad: Value,
        target: BlockId,
    },
    CleanupRet {
        pad: Value,
        unwind: UnwindTarget,
    },
    Unreachable,

    // Arithmetic.
    Binary {
        op: BinOp,
        flags: IntFlags,
        fast_math: FastMathFlags,
        lhs: Value,
        rhs: Value,
    },
    FNeg {
        fast_math: FastMathFlags,
        operand: Value,
    },

    // Conversions.
    Cast {
        op: CastOp,
        flags: IntFlags,
        operand: Value,
        /// The source type, which the text writes out before the value.
        source_type: TypeId,
    },

    // Comparisons.
    ICmp {
        predicate: IntPredicate,
        flags: IntFlags,
        operand_type: TypeId,
        lhs: Value,
        rhs: Value,
    },
    FCmp {
        predicate: FloatPredicate,
        fast_math: FastMathFlags,
        operand_type: TypeId,
        lhs: Value,
        rhs: Value,
    },

    // Memory.
    Alloca {
        allocated_type: TypeId,
        count: Option<(TypeId, Value)>,
        align: Option<Align>,
        address_space: Option<u32>,
        inalloca: bool,
        /// `alloca swifterror ptr`: the slot holds the error a call may set,
        /// which the target keeps in a register rather than on the stack.
        swifterror: bool,
    },
    Load {
        loaded_type: TypeId,
        pointer: Value,
        volatile: bool,
        atomic: Option<(SyncScope, AtomicOrdering)>,
        align: Option<Align>,
    },
    Store {
        value_type: TypeId,
        value: Value,
        pointer: Value,
        volatile: bool,
        atomic: Option<(SyncScope, AtomicOrdering)>,
        align: Option<Align>,
    },
    Fence {
        scope: SyncScope,
        ordering: AtomicOrdering,
    },
    CmpXchg {
        pointer: Value,
        compare_type: TypeId,
        compare: Value,
        new: Value,
        weak: bool,
        volatile: bool,
        scope: SyncScope,
        success: AtomicOrdering,
        failure: AtomicOrdering,
        align: Option<Align>,
    },
    AtomicRmw {
        op: AtomicRmwOp,
        pointer: Value,
        value_type: TypeId,
        value: Value,
        volatile: bool,
        scope: SyncScope,
        ordering: AtomicOrdering,
        align: Option<Align>,
    },
    GetElementPtr {
        source_type: TypeId,
        pointer_type: TypeId,
        pointer: Value,
        indices: Vec<(TypeId, Value)>,
        flags: GepFlags,
        inrange: Option<(i64, i64)>,
    },

    // Vectors and aggregates.
    ExtractElement {
        vector_type: TypeId,
        vector: Value,
        index_type: TypeId,
        index: Value,
    },
    InsertElement {
        vector_type: TypeId,
        vector: Value,
        element_type: TypeId,
        element: Value,
        index_type: TypeId,
        index: Value,
    },
    ShuffleVector {
        vector_type: TypeId,
        first: Value,
        second: Value,
        mask_type: TypeId,
        mask: Value,
    },
    ExtractValue {
        aggregate_type: TypeId,
        aggregate: Value,
        indices: Vec<u32>,
    },
    InsertValue {
        aggregate_type: TypeId,
        aggregate: Value,
        element_type: TypeId,
        element: Value,
        indices: Vec<u32>,
    },

    // Everything else.
    Phi {
        fast_math: FastMathFlags,
        incoming: Vec<(Value, BlockId)>,
    },
    Select {
        fast_math: FastMathFlags,
        condition_type: TypeId,
        condition: Value,
        if_true: Value,
        if_false: Value,
    },
    Freeze {
        operand_type: TypeId,
        operand: Value,
    },
    Call(Box<CallData>),
    VaArg {
        list_type: TypeId,
        list: Value,
    },
    LandingPad {
        cleanup: bool,
        clauses: Vec<LandingPadClause>,
    },
    CatchPad {
        parent: Value,
        args: Vec<(TypeId, Value)>,
    },
    CleanupPad {
        parent: Value,
        args: Vec<(TypeId, Value)>,
    },

    /// A debug record, which replaced the `llvm.dbg.*` intrinsic calls: it
    /// sits in the instruction list, produces no value, and prints as
    /// `#dbg_value(...)`.
    DebugRecord {
        /// The record name without its leading `#`.
        name: String,
        operands: Vec<MdOperand>,
    },
}

impl InstKind {
    pub fn is_terminator(&self) -> bool {
        matches!(
            self,
            InstKind::Ret(_)
                | InstKind::Br { .. }
                | InstKind::CondBr { .. }
                | InstKind::Switch { .. }
                | InstKind::IndirectBr { .. }
                | InstKind::Invoke { .. }
                | InstKind::CallBr { .. }
                | InstKind::Resume { .. }
                | InstKind::CatchSwitch { .. }
                | InstKind::CatchRet { .. }
                | InstKind::CleanupRet { .. }
                | InstKind::Unreachable
        )
    }

    /// The opcode as it is spelled in the text.
    pub fn opcode(&self) -> &'static str {
        match self {
            InstKind::Ret(_) => "ret",
            InstKind::Br { .. } | InstKind::CondBr { .. } => "br",
            InstKind::Switch { .. } => "switch",
            InstKind::IndirectBr { .. } => "indirectbr",
            InstKind::Invoke { .. } => "invoke",
            InstKind::CallBr { .. } => "callbr",
            InstKind::Resume { .. } => "resume",
            InstKind::CatchSwitch { .. } => "catchswitch",
            InstKind::CatchRet { .. } => "catchret",
            InstKind::CleanupRet { .. } => "cleanupret",
            InstKind::Unreachable => "unreachable",
            InstKind::Binary { op, .. } => op.keyword(),
            InstKind::FNeg { .. } => "fneg",
            InstKind::Cast { op, .. } => op.keyword(),
            InstKind::ICmp { .. } => "icmp",
            InstKind::FCmp { .. } => "fcmp",
            InstKind::Alloca { .. } => "alloca",
            InstKind::Load { .. } => "load",
            InstKind::Store { .. } => "store",
            InstKind::Fence { .. } => "fence",
            InstKind::CmpXchg { .. } => "cmpxchg",
            InstKind::AtomicRmw { .. } => "atomicrmw",
            InstKind::GetElementPtr { .. } => "getelementptr",
            InstKind::ExtractElement { .. } => "extractelement",
            InstKind::InsertElement { .. } => "insertelement",
            InstKind::ShuffleVector { .. } => "shufflevector",
            InstKind::ExtractValue { .. } => "extractvalue",
            InstKind::InsertValue { .. } => "insertvalue",
            InstKind::Phi { .. } => "phi",
            InstKind::Select { .. } => "select",
            InstKind::Freeze { .. } => "freeze",
            InstKind::Call(_) => "call",
            InstKind::VaArg { .. } => "va_arg",
            InstKind::LandingPad { .. } => "landingpad",
            InstKind::CatchPad { .. } => "catchpad",
            InstKind::CleanupPad { .. } => "cleanuppad",
            InstKind::DebugRecord { .. } => "#dbg",
        }
    }

    /// The blocks this instruction can transfer control to.
    pub fn successors(&self) -> Vec<BlockId> {
        match self {
            InstKind::Br { target } => vec![*target],
            InstKind::CondBr {
                if_true, if_false, ..
            } => vec![*if_true, *if_false],
            InstKind::Switch { default, cases, .. } => {
                let mut blocks = vec![*default];
                blocks.extend(cases.iter().map(|(_, block)| *block));
                blocks
            }
            InstKind::IndirectBr { destinations, .. } => destinations.clone(),
            InstKind::Invoke { normal, unwind, .. } => vec![*normal, *unwind],
            InstKind::CallBr {
                fallthrough,
                indirect,
                ..
            } => {
                let mut blocks = vec![*fallthrough];
                blocks.extend(indirect.iter().copied());
                blocks
            }
            InstKind::CatchSwitch {
                handlers, unwind, ..
            } => {
                let mut blocks = handlers.clone();
                if let UnwindTarget::Block(block) = unwind {
                    blocks.push(*block);
                }
                blocks
            }
            InstKind::CatchRet { target, .. } => vec![*target],
            InstKind::CleanupRet { unwind, .. } => match unwind {
                UnwindTarget::Block(block) => vec![*block],
                UnwindTarget::Caller => Vec::new(),
            },
            _ => Vec::new(),
        }
    }
}

/// An instruction: a result, a kind, and its metadata.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Instruction {
    /// `None` means the result is unnamed and prints as its slot number, or
    /// that the instruction has no result at all.
    pub name: Option<Name>,
    /// The result type, `void` when the instruction produces no value.
    pub ty: TypeId,
    pub kind: InstKind,
    pub metadata: Vec<MdAttachment>,
}

impl Instruction {
    pub fn new(ty: TypeId, kind: InstKind) -> Instruction {
        Instruction {
            name: None,
            ty,
            kind,
            metadata: Vec::new(),
        }
    }

    pub fn with_name(mut self, name: Name) -> Instruction {
        self.name = Some(name);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_math_flags_collapse_to_fast() {
        let mut flags = FastMathFlags::default();
        assert!(flags.is_empty());
        assert!(!flags.is_fast());
        for keyword in ["nnan", "ninf", "nsz", "arcp", "contract", "afn"] {
            assert!(flags.set_by_keyword(keyword));
        }
        assert!(!flags.is_fast(), "reassoc is still missing");
        assert!(flags.set_by_keyword("reassoc"));
        assert!(flags.is_fast());
        assert_eq!(flags, FastMathFlags::all());
        assert!(!flags.set_by_keyword("nonsense"));
    }

    #[test]
    fn fast_keyword_sets_everything() {
        let mut flags = FastMathFlags::default();
        assert!(flags.set_by_keyword("fast"));
        assert!(flags.is_fast());
    }

    #[test]
    fn terminators_report_their_successors() {
        let entry = BlockId(0);
        let other = BlockId(1);
        let third = BlockId(2);
        assert_eq!(
            InstKind::CondBr {
                condition: Value::Argument(0),
                if_true: entry,
                if_false: other,
            }
            .successors(),
            vec![entry, other]
        );
        assert_eq!(
            InstKind::Switch {
                value_type: TypeId(0),
                value: Value::Argument(0),
                default: third,
                cases: vec![(Value::Argument(1), entry), (Value::Argument(2), other)],
            }
            .successors(),
            vec![third, entry, other]
        );
        assert!(InstKind::Unreachable.successors().is_empty());
        assert!(InstKind::Ret(None).successors().is_empty());
        assert!(
            InstKind::CleanupRet {
                pad: Value::Argument(0),
                unwind: UnwindTarget::Caller,
            }
            .successors()
            .is_empty()
        );
    }

    #[test]
    fn only_terminators_are_terminators() {
        assert!(InstKind::Unreachable.is_terminator());
        assert!(InstKind::Ret(None).is_terminator());
        assert!(InstKind::Br { target: BlockId(0) }.is_terminator());
        assert!(
            !InstKind::FNeg {
                fast_math: FastMathFlags::default(),
                operand: Value::Argument(0),
            }
            .is_terminator()
        );
    }

    #[test]
    fn opcodes_match_their_keywords() {
        assert_eq!(
            InstKind::Binary {
                op: BinOp::Add,
                flags: IntFlags::default(),
                fast_math: FastMathFlags::default(),
                lhs: Value::Argument(0),
                rhs: Value::Argument(1),
            }
            .opcode(),
            "add"
        );
        assert_eq!(BinOp::from_keyword("fadd"), Some(BinOp::FAdd));
        assert!(BinOp::FAdd.is_floating_point());
        assert!(!BinOp::Add.is_floating_point());
        assert_eq!(IntPredicate::from_keyword("sle"), Some(IntPredicate::Sle));
        assert_eq!(
            FloatPredicate::from_keyword("uno"),
            Some(FloatPredicate::Uno)
        );
        assert_eq!(
            AtomicOrdering::from_keyword("seq_cst"),
            Some(AtomicOrdering::SeqCst)
        );
        assert_eq!(AtomicRmwOp::from_keyword("umin"), Some(AtomicRmwOp::UMin));
    }
}
