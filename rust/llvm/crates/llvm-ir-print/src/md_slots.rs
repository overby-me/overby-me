//! Metadata numbering.
//!
//! Metadata nodes are uniqued: two structurally identical non-distinct nodes
//! are one node, and `distinct` is the keyword that opts out. A module that
//! writes the same tuple twice prints it once, with both references pointing
//! at the survivor. Numbers are then assigned by walking the module, so they
//! have nothing to do with the numbers the input used.
//!
//! Two node kinds are never numbered at all. `DIExpression` and `DIArgList`
//! print in place at every use, so they have no definition line.
//!
//! This is computed at print time rather than at parse time. The distinction
//! is real but invisible: nothing between the two needs uniqued metadata yet,
//! and keeping the parsed numbering intact makes a parse error easier to
//! trace back to the text that caused it.

use std::collections::{HashMap, HashSet};

use llvm_ir::MdId;
use llvm_ir::metadata::{MdAttachment, MdField, MdOperand, MdRef, Metadata, SpecializedArgs};
use llvm_ir::{Module, Value};

/// Which node each id prints as, and what number that node gets.
#[derive(Debug, Default)]
pub(crate) struct MetadataSlots {
    /// The node an id collapses onto after uniquing.
    canonical: HashMap<MdId, MdId>,
    /// The printed number of a canonical node.
    numbers: HashMap<MdId, u32>,
    /// Canonical nodes in printing order.
    pub(crate) order: Vec<MdId>,
    /// The composite types that took an identifier for themselves, which are
    /// the ones that print `distinct`.
    claimed: HashSet<MdId>,
}

impl MetadataSlots {
    pub(crate) fn compute(module: &Module) -> MetadataSlots {
        let (canonical, claimed) = unique_nodes(module);
        let mut slots = MetadataSlots {
            canonical,
            claimed,
            ..MetadataSlots::default()
        };
        slots.assign_numbers(module);
        slots
    }

    /// Whether this node is the one that took its identifier, which is what
    /// upstream makes `distinct`.
    ///
    /// Having an identifier is not enough. A second node writing the same
    /// identifier under a different tag finds the first already there, fails
    /// to claim it, and is left an ordinary uniqued node, which is measured:
    /// upstream prints the first `distinct` and the second plain.
    pub(crate) fn claimed_an_identifier(&self, id: MdId) -> bool {
        self.claimed.contains(&id)
    }

    /// The number an id prints as, or `None` when the node prints in place.
    pub(crate) fn number(&self, id: MdId) -> Option<u32> {
        self.numbers.get(&self.resolve(id)).copied()
    }

    pub(crate) fn resolve(&self, id: MdId) -> MdId {
        self.canonical.get(&id).copied().unwrap_or(id)
    }

    /// The attachments of one thing, in the order they will be written. A
    /// node is numbered when it is first met and the attachments are written
    /// in a sorted order rather than the read one, so numbering them as read
    /// gives `!prof` a number `!llvm.loop` should have had.
    fn in_print_order(attachments: &[MdAttachment]) -> Vec<&MdAttachment> {
        let mut sorted: Vec<&MdAttachment> = attachments.iter().collect();
        sorted.sort_by_key(|attachment| crate::metadata::attachment_rank(attachment));
        sorted
    }

    /// Walks the module in the order upstream walks it, numbering each node
    /// before its operands.
    fn assign_numbers(&mut self, module: &Module) {
        for global in &module.globals {
            for attachment in Self::in_print_order(&global.metadata) {
                self.visit_ref(module, &attachment.node);
            }
        }
        for alias in &module.aliases {
            for attachment in Self::in_print_order(&alias.metadata) {
                self.visit_ref(module, &attachment.node);
            }
        }
        for ifunc in &module.ifuncs {
            for attachment in Self::in_print_order(&ifunc.metadata) {
                self.visit_ref(module, &attachment.node);
            }
        }
        for named in &module.named_metadata {
            for operand in &named.operands {
                self.visit(module, *operand);
            }
        }
        for function in &module.functions {
            for attachment in Self::in_print_order(&function.metadata) {
                self.visit_ref(module, &attachment.node);
            }
            for (id, _) in function.blocks() {
                for (_, instruction) in function.block_instructions(id) {
                    // A metadata operand of an intrinsic call is numbered
                    // before the instruction's own attachments.
                    for operand in metadata_operands(module, instruction) {
                        self.visit(module, operand);
                    }
                    for attachment in Self::in_print_order(&instruction.metadata) {
                        self.visit_ref(module, &attachment.node);
                    }
                }
            }
        }
    }

    fn visit_ref(&mut self, module: &Module, node: &MdRef) {
        match node {
            MdRef::Id(id) => self.visit(module, *id),
            MdRef::Inline(inline) => self.visit_operands(module, inline),
        }
    }

    fn visit(&mut self, module: &Module, id: MdId) {
        let canonical = self.resolve(id);
        let Some(node) = module.metadata_node(canonical) else {
            return;
        };
        if prints_in_place(node) {
            // Never numbered, but its operands still can be.
            self.visit_operands(module, &node.clone());
            return;
        }
        if self.numbers.contains_key(&canonical) {
            return;
        }
        let number = self.order.len() as u32;
        self.numbers.insert(canonical, number);
        self.order.push(canonical);
        self.visit_operands(module, &node.clone());
    }

    fn visit_operands(&mut self, module: &Module, node: &Metadata) {
        for referenced in references(node) {
            self.visit(module, referenced);
        }
        for inline in inline_nodes(node) {
            self.visit_operands(module, &inline);
        }
    }
}

/// `DIExpression` and `DIArgList` print at every use rather than once.
pub(crate) fn prints_in_place(node: &Metadata) -> bool {
    matches!(
        node,
        Metadata::Specialized { tag, .. } if tag == "DIExpression" || tag == "DIArgList"
    )
}

/// Collapses structurally identical non-distinct nodes onto one representative.
///
/// Whether two nodes are the same depends on whether their operands are, so
/// this runs to a fixpoint rather than in one pass: `!{!1}` and `!{!2}` become
/// the same node once `!1` and `!2` do.
fn unique_nodes(module: &Module) -> (HashMap<MdId, MdId>, HashSet<MdId>) {
    let ids: Vec<MdId> = module.metadata_nodes().map(|(id, _)| id).collect();
    let mut canonical: HashMap<MdId, MdId> = HashMap::new();

    // A `DICompositeType` carrying an identifier is uniqued under that
    // identifier rather than under what it holds, so two of them merge when
    // nothing else about them agrees, and the first written wins. This runs
    // before the structural pass and outside it: the node that claims an
    // identifier prints `distinct`, so the pass below skips it and would
    // otherwise leave two where upstream has one.
    //
    // The identifier alone is the key, and the tag is checked against
    // whatever holds it. A second node writing the identifier under a
    // different tag therefore claims nothing: it does not merge onto the
    // first, and it is not made distinct either, which is the one case where
    // an identifier buys a node nothing at all. Keying on the pair instead
    // would let it claim an identifier of its own and print `distinct`, which
    // upstream does not do.
    let mut by_identifier: HashMap<&str, (u128, MdId)> = HashMap::new();
    let mut claimed: HashSet<MdId> = HashSet::new();
    for id in &ids {
        let Some(node) = module.metadata_node(*id) else {
            continue;
        };
        let Some((tag, identifier)) = node.odr_key() else {
            continue;
        };
        match by_identifier.get(identifier) {
            Some((holder, first)) => {
                if *holder == tag {
                    canonical.insert(*id, *first);
                }
            }
            None => {
                by_identifier.insert(identifier, (tag, *id));
                claimed.insert(*id);
            }
        }
    }

    // The members of a type that took an identifier are uniqued the same
    // way, under the scope and a key of their own rather than under what they
    // hold: two members of one ODR type with one name are one member however
    // much else differs. `Metadata::odr_member_key` says what the key is and
    // how it was measured.
    //
    // These are held out of the structural pass below as well. Two members
    // that merged here differ structurally, by the file they were written in
    // if nothing else, so leaving them in would let that pass separate them
    // again.
    let mut by_member: HashMap<(MdId, String), MdId> = HashMap::new();
    let mut members: HashSet<MdId> = HashSet::new();
    for id in &ids {
        let Some(node) = module.metadata_node(*id) else {
            continue;
        };
        // A `distinct` node is its own node whatever it holds, which is
        // what keeps a subprogram's definition from merging onto the
        // declaration it shares a linkage name and a scope with.
        if node.is_distinct() {
            continue;
        }
        let Some((scope, key)) = node.odr_member_key() else {
            continue;
        };
        let scope = *canonical.get(&scope).unwrap_or(&scope);
        if !claimed.contains(&scope) {
            continue;
        }
        match by_member.get(&(scope, key.clone())) {
            Some(first) => {
                canonical.insert(*id, *first);
                members.insert(*id);
            }
            None => {
                by_member.insert((scope, key), *id);
                members.insert(*id);
            }
        }
    }

    loop {
        let mut representatives: HashMap<Metadata, MdId> = HashMap::new();
        let mut changed = false;
        for id in &ids {
            let Some(node) = module.metadata_node(*id) else {
                continue;
            };
            if node.is_distinct() || claimed.contains(id) || members.contains(id) {
                continue;
            }
            let key = substitute(node, &canonical);
            match representatives.get(&key) {
                // The lowest-numbered node of a group survives, which is the
                // one upstream would have constructed first.
                Some(representative) => {
                    if canonical.get(id) != Some(representative) {
                        canonical.insert(*id, *representative);
                        changed = true;
                    }
                }
                None => {
                    representatives.insert(key, *id);
                }
            }
        }
        if !changed {
            return (canonical, claimed);
        }
    }
}

fn resolve_in(canonical: &HashMap<MdId, MdId>, id: MdId) -> MdId {
    let mut current = id;
    let mut seen = HashSet::new();
    while let Some(next) = canonical.get(&current) {
        if *next == current || !seen.insert(current) {
            break;
        }
        current = *next;
    }
    current
}

/// The node with every reference replaced by its representative, which is the
/// key two nodes are compared by.
fn substitute(node: &Metadata, canonical: &HashMap<MdId, MdId>) -> Metadata {
    match node {
        Metadata::String(text) => Metadata::String(text.clone()),
        Metadata::Tuple { distinct, operands } => Metadata::Tuple {
            distinct: *distinct,
            operands: operands
                .iter()
                .map(|operand| substitute_operand(operand, canonical))
                .collect(),
        },
        Metadata::Specialized {
            distinct,
            tag,
            args,
        } => Metadata::Specialized {
            distinct: *distinct,
            tag: tag.clone(),
            args: match args {
                SpecializedArgs::Named(fields) => SpecializedArgs::Named(
                    fields
                        .iter()
                        .map(|(key, field)| (key.clone(), substitute_field(field, canonical)))
                        .collect(),
                ),
                SpecializedArgs::Positional(fields) => SpecializedArgs::Positional(
                    fields
                        .iter()
                        .map(|field| substitute_field(field, canonical))
                        .collect(),
                ),
            },
        },
    }
}

fn substitute_operand(operand: &MdOperand, canonical: &HashMap<MdId, MdId>) -> MdOperand {
    match operand {
        MdOperand::Ref(id) => MdOperand::Ref(resolve_in(canonical, *id)),
        MdOperand::Inline(inline) => MdOperand::Inline(Box::new(substitute(inline, canonical))),
        other => other.clone(),
    }
}

fn substitute_field(field: &MdField, canonical: &HashMap<MdId, MdId>) -> MdField {
    match field {
        MdField::Ref(id) => MdField::Ref(resolve_in(canonical, *id)),
        MdField::Inline(inline) => MdField::Inline(Box::new(substitute(inline, canonical))),
        other => other.clone(),
    }
}

/// Every node a node points at by number.
fn references(node: &Metadata) -> Vec<MdId> {
    let mut out = Vec::new();
    match node {
        Metadata::String(_) => {}
        Metadata::Tuple { operands, .. } => {
            for operand in operands {
                if let MdOperand::Ref(id) = operand {
                    out.push(*id);
                }
            }
        }
        Metadata::Specialized { tag, args, .. } => {
            for field in fields_in_numbering_order(tag, args) {
                if let MdField::Ref(id) = field {
                    out.push(*id);
                }
            }
        }
    }
    out
}

/// Every node a node holds in place, whose own references still count.
fn inline_nodes(node: &Metadata) -> Vec<Metadata> {
    let mut out = Vec::new();
    match node {
        Metadata::String(_) => {}
        Metadata::Tuple { operands, .. } => {
            for operand in operands {
                if let MdOperand::Inline(inline) = operand {
                    out.push((**inline).clone());
                }
            }
        }
        Metadata::Specialized { tag, args, .. } => {
            for field in fields_in_numbering_order(tag, args) {
                if let MdField::Inline(inline) = field {
                    out.push((**inline).clone());
                }
            }
        }
    }
    out
}

fn fields_of(args: &SpecializedArgs) -> Vec<&MdField> {
    match args {
        SpecializedArgs::Named(fields) => fields.iter().map(|(_, field)| field).collect(),
        SpecializedArgs::Positional(fields) => fields.iter().collect(),
    }
}

/// The fields of `tag` in the order upstream numbers what they reference,
/// where that differs from the order they are written in.
///
/// A specialized node holds its operands in a fixed order that the printer
/// does not follow: `DISubprogram` writes `scope` before `file` and stores
/// `file` first, so a subprogram whose file and scope are both new gives the
/// file the lower number. Numbering in written order would swap the two, and
/// every node after them, which is why this table exists.
///
/// Derived by asking `llvm-as | llvm-dis`, one probe per kind, each field
/// pointing at a node distinguishable by name. Kinds that are not here number
/// in written order, which was measured too and is what most of them do.
fn numbering_order(tag: &str) -> Option<&'static [&'static str]> {
    Some(match tag {
        "DISubprogram" => &[
            "file",
            "scope",
            "name",
            "linkageName",
            "type",
            "unit",
            "declaration",
            "retainedNodes",
            "containingType",
            "templateParams",
            "thrownTypes",
            "annotations",
            "targetFuncName",
        ],
        "DICompositeType" => &[
            "file",
            "scope",
            "name",
            "baseType",
            "elements",
            "vtableHolder",
            "templateParams",
            "discriminator",
            "dataLocation",
            "associated",
            "allocated",
            "rank",
            "annotations",
            "specification",
        ],
        "DIDerivedType" => &[
            "file",
            "scope",
            "name",
            "baseType",
            "extraData",
            "annotations",
        ],
        "DILexicalBlock" | "DILexicalBlockFile" | "DIModule" => &["file", "scope"],
        _ => return None,
    })
}

/// The fields of a specialized node in the order upstream numbers them: the
/// ones [`numbering_order`] names, in its order, then everything else as
/// written.
fn fields_in_numbering_order<'a>(tag: &str, args: &'a SpecializedArgs) -> Vec<&'a MdField> {
    let SpecializedArgs::Named(fields) = args else {
        return fields_of(args);
    };
    let Some(order) = numbering_order(tag) else {
        return fields_of(args);
    };
    let mut sorted: Vec<(usize, &MdField)> = fields
        .iter()
        .map(|(name, field)| {
            let rank = order
                .iter()
                .position(|known| known == name)
                .unwrap_or(order.len());
            (rank, field)
        })
        .collect();
    sorted.sort_by_key(|(rank, _)| *rank);
    sorted.into_iter().map(|(_, field)| field).collect()
}

/// The metadata a call passes as an argument, which upstream numbers before
/// the instruction's own attachments.
fn metadata_operands(
    module: &Module,
    instruction: &llvm_ir::instruction::Instruction,
) -> Vec<MdId> {
    use llvm_ir::instruction::InstKind;
    let mut out = Vec::new();
    let call = match &instruction.kind {
        InstKind::Call(call) | InstKind::Invoke { call, .. } | InstKind::CallBr { call, .. } => {
            call
        }
        InstKind::DebugRecord { operands, .. } => {
            for operand in operands {
                if let MdOperand::Ref(id) = operand {
                    out.push(*id);
                }
            }
            return out;
        }
        _ => return out,
    };
    for argument in &call.args {
        let Value::Constant(id) = argument.value else {
            continue;
        };
        if let llvm_ir::constant::Constant::Metadata { operand, .. } = module.ctx.constant(id)
            && let MdOperand::Ref(node) = **operand
        {
            out.push(node);
        }
    }
    out
}
