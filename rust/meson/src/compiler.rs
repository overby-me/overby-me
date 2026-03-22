/// Bytecode compiler: transforms AST into bytecode instructions for the VM.
use crate::ast::*;

#[derive(Debug, Clone, PartialEq)]
pub enum OpCode {
    /// Push a constant onto the stack
    Constant(usize),
    /// Push true
    True,
    /// Push false
    False,
    /// Pop and discard top of stack
    Pop,
    /// Load variable by name
    LoadVar(String),
    /// Store top of stack into variable
    StoreVar(String),
    /// Load + add + store (for +=)
    PlusAssignVar(String),
    /// Create array from N elements on stack
    MakeArray(usize),
    /// Create dict from N key-value pairs on stack
    MakeDict(usize),
    /// Binary operations
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Neq,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    In,
    NotIn,
    /// Unary operations
    Not,
    Negate,
    /// Call function with N args. Args are (name_or_none, value) pairs on stack.
    Call(usize),
    /// Call method with name and N args
    MethodCall(String, usize),
    /// Index into array/dict
    Index,
    /// Jump unconditionally
    Jump(usize),
    /// Jump if top of stack is false (pop condition)
    JumpIfFalse(usize),
    /// Jump if top of stack is true (pop condition)
    JumpIfTrue(usize),
    /// Push keyword argument name (None = positional)
    ArgName(Option<String>),
    /// Setup foreach: push iterator state
    IterSetup,
    /// Advance iterator, jump to end if exhausted
    IterNext(Vec<String>, usize),
    /// Break out of loop
    Break,
    /// Continue loop
    Continue,
    /// Format string with N substitutions
    FString(String),
    /// No-op placeholder
    Nop,
    /// Halt
    Halt,
}

#[derive(Debug, Clone)]
pub enum Constant {
    String(String),
    Int(i64),
    Bool(bool),
    None,
}

#[derive(Debug)]
pub struct Chunk {
    pub code: Vec<OpCode>,
    pub constants: Vec<Constant>,
    pub lines: Vec<usize>,
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            constants: Vec::new(),
            lines: Vec::new(),
        }
    }

    pub fn emit(&mut self, op: OpCode, line: usize) -> usize {
        let idx = self.code.len();
        self.code.push(op);
        self.lines.push(line);
        idx
    }

    pub fn add_constant(&mut self, c: Constant) -> usize {
        self.constants.push(c);
        self.constants.len() - 1
    }

    pub fn patch_jump(&mut self, idx: usize) {
        let target = self.code.len();
        match &mut self.code[idx] {
            OpCode::Jump(t)
            | OpCode::JumpIfFalse(t)
            | OpCode::JumpIfTrue(t)
            | OpCode::IterNext(_, t) => *t = target,
            _ => panic!("Cannot patch non-jump instruction"),
        }
    }
}

pub struct Compiler {
    pub chunk: Chunk,
    loop_breaks: Vec<Vec<usize>>,
    loop_continues: Vec<usize>,
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            chunk: Chunk::new(),
            loop_breaks: Vec::new(),
            loop_continues: Vec::new(),
        }
    }

    pub fn compile(&mut self, program: &Program) -> Result<(), String> {
        for stmt in &program.statements {
            self.compile_statement(stmt)?;
        }
        self.chunk.emit(OpCode::Halt, 0);
        Ok(())
    }

    fn compile_statement(&mut self, stmt: &Statement) -> Result<(), String> {
        match stmt {
            Statement::Expression(expr) => {
                let line = expr.loc().line;
                self.compile_expression(expr)?;
                self.chunk.emit(OpCode::Pop, line);
            }
            Statement::Assignment(a) => {
                self.compile_expression(&a.value)?;
                self.chunk
                    .emit(OpCode::StoreVar(a.name.clone()), a.loc.line);
            }
            Statement::PlusAssignment(a) => {
                self.compile_expression(&a.value)?;
                self.chunk
                    .emit(OpCode::PlusAssignVar(a.name.clone()), a.loc.line);
            }
            Statement::If(if_stmt) => self.compile_if(if_stmt)?,
            Statement::Foreach(foreach) => self.compile_foreach(foreach)?,
            Statement::Break(loc) => {
                let idx = self.chunk.emit(OpCode::Jump(0), loc.line);
                if let Some(breaks) = self.loop_breaks.last_mut() {
                    breaks.push(idx);
                }
            }
            Statement::Continue(loc) => {
                if let Some(&continue_target) = self.loop_continues.last() {
                    self.chunk.emit(OpCode::Jump(continue_target), loc.line);
                }
            }
        }
        Ok(())
    }

    fn compile_if(&mut self, if_stmt: &IfStatement) -> Result<(), String> {
        let line = if_stmt.loc.line;

        // Compile condition
        self.compile_expression(&if_stmt.condition)?;
        let false_jump = self.chunk.emit(OpCode::JumpIfFalse(0), line);

        // Compile body
        for stmt in &if_stmt.body {
            self.compile_statement(stmt)?;
        }

        // Jump over elif/else
        let mut end_jumps = vec![self.chunk.emit(OpCode::Jump(0), line)];

        // Patch false jump to here
        self.chunk.patch_jump(false_jump);

        // Compile elif clauses
        for (cond, body) in &if_stmt.elif_clauses {
            self.compile_expression(cond)?;
            let elif_false = self.chunk.emit(OpCode::JumpIfFalse(0), line);
            for stmt in body {
                self.compile_statement(stmt)?;
            }
            end_jumps.push(self.chunk.emit(OpCode::Jump(0), line));
            self.chunk.patch_jump(elif_false);
        }

        // Compile else
        if let Some(else_body) = &if_stmt.else_body {
            for stmt in else_body {
                self.compile_statement(stmt)?;
            }
        }

        // Patch all end jumps
        for j in end_jumps {
            self.chunk.patch_jump(j);
        }

        Ok(())
    }

    fn compile_foreach(&mut self, foreach: &ForeachStatement) -> Result<(), String> {
        let line = foreach.loc.line;

        // Compile iterable
        self.compile_expression(&foreach.iterable)?;
        self.chunk.emit(OpCode::IterSetup, line);

        // Loop start
        let loop_start = self.chunk.code.len();
        self.loop_continues.push(loop_start);
        self.loop_breaks.push(Vec::new());

        // Iterator advance
        let iter_next = self
            .chunk
            .emit(OpCode::IterNext(foreach.varnames.clone(), 0), line);

        // Compile body
        for stmt in &foreach.body {
            self.compile_statement(stmt)?;
        }

        // Jump back to loop start
        self.chunk.emit(OpCode::Jump(loop_start), line);

        // Patch iterator end jump
        self.chunk.patch_jump(iter_next);

        // Patch break jumps
        let breaks = self.loop_breaks.pop().unwrap();
        for b in breaks {
            self.chunk.patch_jump(b);
        }
        self.loop_continues.pop();

        Ok(())
    }

    fn compile_expression(&mut self, expr: &Expression) -> Result<(), String> {
        let line = expr.loc().line;
        match expr {
            Expression::StringLiteral(s, _) => {
                let idx = self.chunk.add_constant(Constant::String(s.clone()));
                self.chunk.emit(OpCode::Constant(idx), line);
            }
            Expression::MultilineStringLiteral(s, _) => {
                let idx = self.chunk.add_constant(Constant::String(s.clone()));
                self.chunk.emit(OpCode::Constant(idx), line);
            }
            Expression::FStringLiteral(s, _) => {
                self.chunk.emit(OpCode::FString(s.clone()), line);
            }
            Expression::IntLiteral(n, _) => {
                let idx = self.chunk.add_constant(Constant::Int(*n));
                self.chunk.emit(OpCode::Constant(idx), line);
            }
            Expression::BoolLiteral(b, _) => {
                if *b {
                    self.chunk.emit(OpCode::True, line);
                } else {
                    self.chunk.emit(OpCode::False, line);
                }
            }
            Expression::Identifier(name, _) => {
                self.chunk.emit(OpCode::LoadVar(name.clone()), line);
            }
            Expression::Array(elements, _) => {
                for e in elements {
                    self.compile_expression(e)?;
                }
                self.chunk.emit(OpCode::MakeArray(elements.len()), line);
            }
            Expression::Dict(entries, _) => {
                for (k, v) in entries {
                    self.compile_expression(k)?;
                    self.compile_expression(v)?;
                }
                self.chunk.emit(OpCode::MakeDict(entries.len()), line);
            }
            Expression::UnaryOp(op, operand, _) => {
                self.compile_expression(operand)?;
                match op {
                    UnaryOp::Not => self.chunk.emit(OpCode::Not, line),
                    UnaryOp::Negate => self.chunk.emit(OpCode::Negate, line),
                };
            }
            Expression::BinaryOp(op, left, right, _) => {
                // Short-circuit for and/or
                match op {
                    BinaryOp::And => {
                        self.compile_expression(left)?;
                        let jump = self.chunk.emit(OpCode::JumpIfFalse(0), line);
                        self.chunk.emit(OpCode::Pop, line);
                        self.compile_expression(right)?;
                        self.chunk.patch_jump(jump);
                        return Ok(());
                    }
                    BinaryOp::Or => {
                        self.compile_expression(left)?;
                        let jump = self.chunk.emit(OpCode::JumpIfTrue(0), line);
                        self.chunk.emit(OpCode::Pop, line);
                        self.compile_expression(right)?;
                        self.chunk.patch_jump(jump);
                        return Ok(());
                    }
                    _ => {}
                }
                self.compile_expression(left)?;
                self.compile_expression(right)?;
                let opcode = match op {
                    BinaryOp::Add => OpCode::Add,
                    BinaryOp::Sub => OpCode::Sub,
                    BinaryOp::Mul => OpCode::Mul,
                    BinaryOp::Div => OpCode::Div,
                    BinaryOp::Mod => OpCode::Mod,
                    BinaryOp::Eq => OpCode::Eq,
                    BinaryOp::Neq => OpCode::Neq,
                    BinaryOp::Lt => OpCode::Lt,
                    BinaryOp::Gt => OpCode::Gt,
                    BinaryOp::Le => OpCode::Le,
                    BinaryOp::Ge => OpCode::Ge,
                    BinaryOp::In => OpCode::In,
                    BinaryOp::NotIn => OpCode::NotIn,
                    BinaryOp::And | BinaryOp::Or => unreachable!(),
                };
                self.chunk.emit(opcode, line);
            }
            Expression::FunctionCall(func, args, _) => {
                // Push the function reference
                self.compile_expression(func)?;
                // Push arguments
                for arg in args {
                    self.chunk.emit(OpCode::ArgName(arg.name.clone()), line);
                    self.compile_expression(&arg.value)?;
                }
                self.chunk.emit(OpCode::Call(args.len()), line);
            }
            Expression::MethodCall(obj, method, args, _) => {
                self.compile_expression(obj)?;
                for arg in args {
                    self.chunk.emit(OpCode::ArgName(arg.name.clone()), line);
                    self.compile_expression(&arg.value)?;
                }
                self.chunk
                    .emit(OpCode::MethodCall(method.clone(), args.len()), line);
            }
            Expression::Index(obj, index, _) => {
                self.compile_expression(obj)?;
                self.compile_expression(index)?;
                self.chunk.emit(OpCode::Index, line);
            }
            Expression::Ternary(cond, true_val, false_val, _) => {
                self.compile_expression(cond)?;
                let false_jump = self.chunk.emit(OpCode::JumpIfFalse(0), line);
                self.compile_expression(true_val)?;
                let end_jump = self.chunk.emit(OpCode::Jump(0), line);
                self.chunk.patch_jump(false_jump);
                self.compile_expression(false_val)?;
                self.chunk.patch_jump(end_jump);
            }
        }
        Ok(())
    }
}
