//! In-memory build graph: rules, edges, nodes, file-scope variable bindings.

use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct State {
    pub bindings: HashMap<String, String>,
    pub rules: HashMap<String, Rule>,
    pub edges: Vec<Edge>,
    /// Map output path → edge index that produces it.
    pub producers: HashMap<String, usize>,
    /// Map target/alias → edge index for `default` resolution.
    pub defaults: Vec<String>,
    /// Pool depth limits, keyed by pool name. The implicit `console`
    /// pool (depth 1, special terminal-locking semantics) is always
    /// present; user-declared pools are merged in by the parser.
    pub pools: HashMap<String, usize>,
}

#[derive(Debug, Default, Clone)]
pub struct Rule {
    pub name: String,
    pub bindings: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct Edge {
    pub rule: String,
    pub outputs: Vec<String>,
    pub implicit_outputs: Vec<String>,
    pub inputs: Vec<String>,
    pub implicit_inputs: Vec<String>,
    pub order_only_inputs: Vec<String>,
    pub bindings: HashMap<String, String>,
    /// Path to a dyndep file. When set, the file is treated as an
    /// order-only input and parsed before the edge is dispatched so its
    /// extra outputs/inputs are merged into the edge.
    pub dyndep: Option<String>,
}

impl Edge {
    /// Phony rule: the canonical no-op rule with no command.
    pub fn is_phony(&self) -> bool {
        self.rule == "phony"
    }
}
