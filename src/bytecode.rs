//! 字节码与编译器：IR → 字节码，每个函数编译为一个 `Chunk`（ADR-0002）。
//!
//! 栈式虚拟机。帧布局：`stack[base..base + n_slots]` 是局部槽位（参数在最前），其上是临时值。
//! `for` 循环和 `match` 需要的隐藏状态放在 `n_locals` 之后的额外槽位里。

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::ast::{BinOp, Pos, UnOp};
use crate::ir::{Block, Expr, FnProto, Hoist, Pattern, Program, Slot, Stmt, StructProto, Target, Var};
use crate::value::Value;

/// 需要 bool 的位置，用于报错信息。
#[derive(Clone, Copy, Debug)]
pub enum BoolCtx {
    If,
    While,
    Guard,
    And,
    Or,
}

impl BoolCtx {
    pub fn what(self) -> &'static str {
        match self {
            BoolCtx::If => "if condition",
            BoolCtx::While => "while condition",
            BoolCtx::Guard => "match guard",
            BoolCtx::And => "operand of 'and'",
            BoolCtx::Or => "operand of 'or'",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Op {
    Const(u32),
    Nil,
    True,
    False,
    Pop,
    PopN(u32),
    Dup,
    Dup2,
    GetLocal(u32),
    SetLocal(u32),
    /// (单元格编号, 名字下标)；名字用于"尚未初始化"的报错
    GetCell(u32, u32),
    SetCell(u32),
    GetUpval(u32, u32),
    SetUpval(u32),
    GetGlobal(u32, u32),
    SetGlobal(u32),
    NewCell(u32),
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Neg,
    Not,
    /// 检查栈顶是 bool（不弹出）
    CheckBool(BoolCtx),
    /// 弹出 bool，为 false 时跳转
    JumpIfFalse(u32, BoolCtx),
    Jump(u32),
    Range(bool),
    Array(u32),
    Interp(u32),
    Index,
    SetIndex,
    /// (名字下标, 内联缓存下标)
    Field(u32, u32),
    SetField(u32, u32),
    /// 栈：callee, 参数...
    Call(u32),
    /// (参数个数, 名字列表下标)，用于结构体具名构造
    CallNamed(u32, u32),
    /// 方法调用描述下标；栈：对象, 参数...
    Method(u32),
    Closure(u32),
    Struct(u32),
    /// 弹出结构体类型，挂上方法
    Impl(u32),
    /// 弹出可迭代对象，存入隐藏槽位 (slot, slot + 1)
    IterPrep(u32),
    /// (隐藏槽位, 结束时跳转目标, 是否同时压入下标)
    ForNext(u32, u32, bool),
    /// (模式下标, 被匹配值所在槽位, 不匹配时跳转目标)
    Match(u32, u32, u32),
    NoMatch(u32),
    Return,

    // ---- 超级指令（窥孔优化生成），减少值栈的压入弹出 ----
    /// 栈顶 = 栈顶 op 局部变量
    BinLocal(BinOp, u32),
    /// 栈顶 = 栈顶 op 常量
    BinConst(BinOp, u32),
    /// 弹出右值，局部变量 = 局部变量 op 右值（复合赋值）
    OpAssignLocal(BinOp, u32),
    /// 局部变量 = 局部变量 op 常量，如 `i += 1`
    OpAssignLocalConst(BinOp, u32, u32),
    /// 弹出两个值比较，结果为 false 时跳转
    JumpIfNot(BinOp, u32),
    /// 弹出一个值与局部变量比较
    JumpIfNotLocal(BinOp, u32, u32),
    /// 弹出一个值与常量比较
    JumpIfNotConst(BinOp, u32, u32),
    /// 两个局部变量比较，如 `i < n`
    JumpIfNotLL(BinOp, u32, u32, u32),
    /// 局部变量与常量比较，如 `n < 2`
    JumpIfNotLC(BinOp, u32, u32, u32),
    /// 弹出下标，压入 局部变量[下标]
    IndexLocal(u32),
    /// 弹出值和下标，局部变量[下标] = 值
    SetIndexLocal(u32),
    /// 压入 局部变量.字段：(槽位, 名字下标, 内联缓存下标)
    FieldLocal(u32, u32, u32),
    /// 弹出值，局部变量.字段 = 值
    SetFieldLocal(u32, u32, u32),
}

/// 不含块、`if`、`match` 的表达式：求值过程中不会给普通局部变量赋值
/// （闭包只能修改被捕获的变量，而它们存放在单元格里）。
/// 只有这样，把读取局部变量的时机挪到这类表达式之后才不改变语义。
fn is_simple(e: &Expr) -> bool {
    match e {
        Expr::Const(_) | Expr::Var(..) | Expr::Lambda(_) => true,
        Expr::Unary(_, x, _) | Expr::Field(x, _, _) => is_simple(x),
        Expr::Binary(_, a, b, _) | Expr::Index(a, b, _) | Expr::Range(a, b, _, _) => is_simple(a) && is_simple(b),
        Expr::Interp(xs) | Expr::Array(xs) => xs.iter().all(is_simple),
        Expr::Call { callee, args, .. } => is_simple(callee) && args.iter().all(is_simple),
        Expr::MethodCall { obj, args, .. } => is_simple(obj) && args.iter().all(is_simple),
        Expr::If(..) | Expr::Match(..) | Expr::Block(_) => false,
    }
}

fn is_comparison(op: BinOp) -> bool {
    matches!(op, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge | BinOp::Eq | BinOp::Ne)
}

pub enum Fallback {
    None,
    Var(Var, u32),
    Const(Value),
}

pub struct MethodDesc {
    pub name: Rc<str>,
    pub argc: u32,
    pub fallback: Fallback,
}

pub struct ImplDesc {
    pub name: Rc<str>,
    pub methods: Vec<(Rc<str>, u32)>,
}

#[derive(Default)]
pub struct Chunk {
    pub code: Vec<Op>,
    /// 与 `code` 一一对应的源码位置
    pub pos: Vec<Pos>,
    pub consts: Vec<Value>,
    pub names: Vec<Rc<str>>,
    pub name_lists: Vec<Vec<Rc<str>>>,
    pub methods: Vec<MethodDesc>,
    pub structs: Vec<Rc<StructProto>>,
    pub impls: Vec<ImplDesc>,
    pub patterns: Vec<Pattern>,
    /// 字段访问的内联缓存：(上次遇到的结构体类型地址, 字段下标)
    pub field_caches: Vec<Cell<(usize, u32)>>,
    pub n_slots: usize,
}

/// 反汇编，用于 `zyx --dis` 调试与性能分析。
pub fn disassemble(prog: &Program, chunks: &[Rc<Chunk>]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let name = match prog.protos.get(i) {
            Some(p) => p.name.as_deref().map_or_else(|| format!("<fn #{i}>"), |n| format!("fn {n}")),
            None => "<script>".to_string(),
        };
        let _ = writeln!(out, "== {name} (slots: {}) ==", chunk.n_slots);
        for (ip, op) in chunk.code.iter().enumerate() {
            let _ = write!(out, "{ip:4}  {:>4}:{:<3} {op:?}", chunk.pos[ip].line, chunk.pos[ip].col);
            match op {
                Op::Const(c) => {
                    let _ = write!(out, "    ; {}", chunk.consts[*c as usize].repr());
                }
                Op::Method(d) => {
                    let _ = write!(out, "    ; .{}", chunk.methods[*d as usize].name);
                }
                _ => {}
            }
            out.push('\n');
        }
    }
    out
}

/// 编译整个程序。返回的下标与 `FnProto::id` 一致，最后一个是脚本顶层。
pub fn compile(prog: &Program) -> Vec<Rc<Chunk>> {
    let mut chunks: Vec<Rc<Chunk>> = prog.protos.iter().map(|p| Rc::new(compile_fn(p, true))).collect();
    chunks.push(Rc::new(compile_fn(&prog.main, false)));
    chunks
}

fn compile_fn(proto: &FnProto, returns_value: bool) -> Chunk {
    let mut c = Compiler {
        chunk: Chunk::default(),
        depth: 0,
        loops: Vec::new(),
        hidden_next: proto.n_locals as u32,
        hidden_max: proto.n_locals as u32,
        pos: Pos { line: 0, col: 0 },
        name_ids: HashMap::new(),
        label: 0,
    };
    c.block(&proto.body, returns_value);
    if !returns_value {
        c.emit(Op::Nil);
    }
    c.emit(Op::Return);
    c.chunk.n_slots = c.hidden_max as usize;
    c.chunk
}

struct Loop {
    continue_at: usize,
    breaks: Vec<usize>,
    depth: i32,
}

struct Compiler {
    chunk: Chunk,
    /// 当前临时值栈深度（相对帧槽位之上），用于 break / continue 时清理
    depth: i32,
    loops: Vec<Loop>,
    hidden_next: u32,
    hidden_max: u32,
    pos: Pos,
    name_ids: HashMap<Rc<str>, u32>,
    /// 最近一个跳转目标的位置。窥孔优化不能合并跨越跳转目标的指令。
    label: usize,
}

impl Compiler {
    fn at(&mut self, pos: Pos) -> &mut Self {
        self.pos = pos;
        self
    }

    fn effect(&self, op: Op) -> i32 {
        match op {
            Op::Const(_)
            | Op::Nil
            | Op::True
            | Op::False
            | Op::Dup
            | Op::GetLocal(_)
            | Op::GetCell(..)
            | Op::GetUpval(..)
            | Op::GetGlobal(..)
            | Op::Closure(_)
            | Op::Struct(_) => 1,
            Op::Dup2 => 2,
            Op::Pop
            | Op::SetLocal(_)
            | Op::SetCell(_)
            | Op::SetUpval(_)
            | Op::SetGlobal(_)
            | Op::Add
            | Op::Sub
            | Op::Mul
            | Op::Div
            | Op::Mod
            | Op::Pow
            | Op::Eq
            | Op::Ne
            | Op::Lt
            | Op::Le
            | Op::Gt
            | Op::Ge
            | Op::JumpIfFalse(..)
            | Op::Range(_)
            | Op::Index
            | Op::Impl(_)
            | Op::IterPrep(_)
            | Op::Return => -1,
            Op::PopN(n) => -(n as i32),
            Op::NewCell(_)
            | Op::Neg
            | Op::Not
            | Op::CheckBool(_)
            | Op::Jump(_)
            | Op::Field(..)
            | Op::Match(..)
            | Op::NoMatch(_) => 0,
            Op::Array(n) | Op::Interp(n) => 1 - n as i32,
            Op::SetIndex => -3,
            Op::SetField(..) => -2,
            Op::Call(argc) | Op::CallNamed(argc, _) => -(argc as i32),
            Op::Method(d) => -(self.chunk.methods[d as usize].argc as i32),
            Op::ForNext(_, _, with_index) => {
                if with_index {
                    2
                } else {
                    1
                }
            }
            Op::BinLocal(..) | Op::BinConst(..) | Op::OpAssignLocalConst(..) | Op::IndexLocal(_) => 0,
            Op::JumpIfNotLL(..) | Op::JumpIfNotLC(..) => 0,
            Op::OpAssignLocal(..) | Op::JumpIfNotLocal(..) | Op::JumpIfNotConst(..) | Op::SetFieldLocal(..) => -1,
            Op::JumpIfNot(..) | Op::SetIndexLocal(_) => -2,
            Op::FieldLocal(..) => 1,
        }
    }

    /// 末尾 `k` 条指令可以和即将发出的指令合并：没有跳转落在它们中间或即将发出的位置上
    /// （落在这组指令的第一条上没关系，合并后的指令就在那里）。
    fn can_fuse(&self, k: usize) -> bool {
        let n = self.chunk.code.len();
        n >= k && self.label <= n - k
    }

    fn last_op(&self, back: usize) -> Option<Op> {
        let n = self.chunk.code.len();
        if n > back { Some(self.chunk.code[n - 1 - back]) } else { None }
    }

    /// 删除最后一条指令，撤销它对栈深度的影响，返回它和它的位置。
    fn pop_op(&mut self) -> (Op, Pos) {
        let op = self.chunk.code.pop().expect("op");
        let pos = self.chunk.pos.pop().expect("pos");
        self.depth -= self.effect(op);
        (op, pos)
    }

    fn emit(&mut self, op: Op) -> usize {
        self.depth += self.effect(op);
        self.chunk.code.push(op);
        self.chunk.pos.push(self.pos);
        self.chunk.code.len() - 1
    }

    fn here(&self) -> u32 {
        self.chunk.code.len() as u32
    }

    /// 把跳转指令的目标改为当前位置。
    fn patch(&mut self, at: usize) {
        let target = self.here();
        self.label = target as usize;
        self.chunk.code[at] = match self.chunk.code[at] {
            Op::Jump(_) => Op::Jump(target),
            Op::JumpIfFalse(_, ctx) => Op::JumpIfFalse(target, ctx),
            Op::JumpIfNot(op, _) => Op::JumpIfNot(op, target),
            Op::JumpIfNotLocal(op, s, _) => Op::JumpIfNotLocal(op, s, target),
            Op::JumpIfNotConst(op, c, _) => Op::JumpIfNotConst(op, c, target),
            Op::JumpIfNotLL(op, a, b, _) => Op::JumpIfNotLL(op, a, b, target),
            Op::JumpIfNotLC(op, a, c, _) => Op::JumpIfNotLC(op, a, c, target),
            Op::ForNext(s, _, i) => Op::ForNext(s, target, i),
            Op::Match(p, s, _) => Op::Match(p, s, target),
            other => unreachable!("not a jump: {other:?}"),
        };
    }

    fn alloc_hidden(&mut self, n: u32) -> u32 {
        let s = self.hidden_next;
        self.hidden_next += n;
        self.hidden_max = self.hidden_max.max(self.hidden_next);
        s
    }

    fn free_hidden(&mut self, n: u32) {
        self.hidden_next -= n;
    }

    fn field_cache(&mut self) -> u32 {
        self.chunk.field_caches.push(Cell::new((0, 0)));
        (self.chunk.field_caches.len() - 1) as u32
    }

    /// 编译条件表达式，返回条件为假时需要回填的跳转。`and` / `or` 直接编译为短路跳转。
    fn cond(&mut self, e: &Expr, ctx: BoolCtx, pos: Pos) -> Vec<usize> {
        match e {
            Expr::Binary(BinOp::And, a, b, p) => {
                let mut fails = self.cond(a, BoolCtx::And, *p);
                fails.extend(self.cond(b, BoolCtx::And, *p));
                fails
            }
            Expr::Binary(BinOp::Or, a, b, p) => {
                let a_fails = self.cond(a, BoolCtx::Or, *p);
                let to_body = self.emit(Op::Jump(0));
                for j in a_fails {
                    self.patch(j);
                }
                let fails = self.cond(b, BoolCtx::Or, *p);
                self.patch(to_body);
                fails
            }
            _ => {
                self.expr(e);
                vec![self.jump_if_false(ctx, pos)]
            }
        }
    }

    fn name(&mut self, name: &Rc<str>) -> u32 {
        if let Some(&i) = self.name_ids.get(name) {
            return i;
        }
        self.chunk.names.push(name.clone());
        let i = (self.chunk.names.len() - 1) as u32;
        self.name_ids.insert(name.clone(), i);
        i
    }

    fn constant(&mut self, v: &Value) {
        let op = match v {
            Value::Nil => Op::Nil,
            Value::Bool(true) => Op::True,
            Value::Bool(false) => Op::False,
            _ => {
                self.chunk.consts.push(v.clone());
                Op::Const((self.chunk.consts.len() - 1) as u32)
            }
        };
        self.emit(op);
    }

    fn get(&mut self, var: Var, name: &Rc<str>, pos: Pos) {
        let n = self.name(name);
        let op = match var {
            Var::Local(s) => Op::GetLocal(s),
            Var::Cell(c) => Op::GetCell(c, n),
            Var::Upval(u) => Op::GetUpval(u, n),
            Var::Global(g) => Op::GetGlobal(g, n),
        };
        self.at(pos).emit(op);
    }

    fn set_var(&mut self, var: Var) {
        let op = match var {
            Var::Local(s) => Op::SetLocal(s),
            Var::Cell(c) => Op::SetCell(c),
            Var::Upval(u) => Op::SetUpval(u),
            Var::Global(g) => Op::SetGlobal(g),
        };
        self.emit(op);
    }

    fn set(&mut self, slot: Slot) {
        self.set_var(slot.into());
    }

    fn binop(&mut self, binop: BinOp, pos: Pos) {
        let op = match binop {
            BinOp::Add => Op::Add,
            BinOp::Sub => Op::Sub,
            BinOp::Mul => Op::Mul,
            BinOp::Div => Op::Div,
            BinOp::Mod => Op::Mod,
            BinOp::Pow => Op::Pow,
            BinOp::Eq => Op::Eq,
            BinOp::Ne => Op::Ne,
            BinOp::Lt => Op::Lt,
            BinOp::Le => Op::Le,
            BinOp::Gt => Op::Gt,
            BinOp::Ge => Op::Ge,
            BinOp::And | BinOp::Or => unreachable!("logical operators are compiled to jumps"),
        };
        if self.can_fuse(1) {
            match self.last_op(0) {
                Some(Op::GetLocal(s)) => {
                    self.pop_op();
                    self.at(pos).emit(Op::BinLocal(binop, s));
                    return;
                }
                Some(Op::Const(c)) => {
                    self.pop_op();
                    self.at(pos).emit(Op::BinConst(binop, c));
                    return;
                }
                _ => {}
            }
        }
        self.at(pos).emit(op);
    }

    /// 条件跳转：前一条是比较时合并为比较跳转。返回需要回填的指令位置。
    fn jump_if_false(&mut self, ctx: BoolCtx, pos: Pos) -> usize {
        if self.can_fuse(1) {
            let fused = match self.last_op(0) {
                Some(Op::Lt) => Some(BinOp::Lt),
                Some(Op::Le) => Some(BinOp::Le),
                Some(Op::Gt) => Some(BinOp::Gt),
                Some(Op::Ge) => Some(BinOp::Ge),
                Some(Op::Eq) => Some(BinOp::Eq),
                Some(Op::Ne) => Some(BinOp::Ne),
                _ => None,
            };
            if let Some(cmp) = fused {
                let (_, p) = self.pop_op();
                return self.at(p).emit(Op::JumpIfNot(cmp, 0));
            }
            match self.last_op(0) {
                Some(Op::BinLocal(cmp, b)) if is_comparison(cmp) => {
                    let (_, p) = self.pop_op();
                    if let (true, Some(Op::GetLocal(a))) = (self.can_fuse(1), self.last_op(0)) {
                        self.pop_op();
                        return self.at(p).emit(Op::JumpIfNotLL(cmp, a, b, 0));
                    }
                    return self.at(p).emit(Op::JumpIfNotLocal(cmp, b, 0));
                }
                Some(Op::BinConst(cmp, c)) if is_comparison(cmp) => {
                    let (_, p) = self.pop_op();
                    if let (true, Some(Op::GetLocal(a))) = (self.can_fuse(1), self.last_op(0)) {
                        self.pop_op();
                        return self.at(p).emit(Op::JumpIfNotLC(cmp, a, c, 0));
                    }
                    return self.at(p).emit(Op::JumpIfNotConst(cmp, c, 0));
                }
                _ => {}
            }
        }
        self.at(pos).emit(Op::JumpIfFalse(0, ctx))
    }

    // ---------- 块与语句 ----------

    fn hoists(&mut self, hoists: &[Hoist]) {
        for h in hoists {
            match h {
                Hoist::Struct(slot, proto) => {
                    self.chunk.structs.push(proto.clone());
                    self.emit(Op::Struct((self.chunk.structs.len() - 1) as u32));
                    self.set(*slot);
                }
                Hoist::Fn(slot, idx) => {
                    self.emit(Op::Closure(*idx));
                    self.set(*slot);
                }
                Hoist::Impl { target, name, methods, pos } => {
                    self.get(*target, name, *pos);
                    self.chunk.impls.push(ImplDesc { name: name.clone(), methods: methods.clone() });
                    let idx = (self.chunk.impls.len() - 1) as u32;
                    self.at(*pos).emit(Op::Impl(idx));
                }
            }
        }
    }

    /// `want` 为真时在栈上留下块的值。
    fn block(&mut self, b: &Block, want: bool) {
        for &c in &b.cells {
            self.emit(Op::NewCell(c));
        }
        self.hoists(&b.hoists);
        self.stmts(&b.stmts, want);
    }

    fn stmts(&mut self, stmts: &[Stmt], want: bool) {
        let Some((last, init)) = stmts.split_last() else {
            if want {
                self.emit(Op::Nil);
            }
            return;
        };
        for s in init {
            self.stmt(s);
        }
        match last {
            Stmt::Expr(e) if want => self.expr(e),
            Stmt::Expr(e) => self.expr_discard(e),
            s => {
                self.stmt(s);
                if want {
                    self.emit(Op::Nil);
                }
            }
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let(slot, e) => {
                self.expr(e);
                self.set(*slot);
            }
            Stmt::Assign(target, op, e, pos) => self.assign(target, *op, e, *pos),
            Stmt::Expr(e) => self.expr_discard(e),
            Stmt::While(cond, body, pos) => {
                let start = self.chunk.code.len();
                self.label = start;
                let exits = self.cond(cond, BoolCtx::While, *pos);
                self.loops.push(Loop { continue_at: start, breaks: Vec::new(), depth: self.depth });
                self.block(body, false);
                self.emit(Op::Jump(start as u32));
                for j in exits {
                    self.patch(j);
                }
                self.end_loop();
            }
            Stmt::For { index, var, iter, body, pos } => {
                self.expr(iter);
                let h = self.alloc_hidden(2);
                self.at(*pos).emit(Op::IterPrep(h));
                let base_depth = self.depth;
                let start = self.chunk.code.len();
                self.label = start;
                let next = self.emit(Op::ForNext(h, 0, index.is_some()));
                for &c in &body.cells {
                    self.emit(Op::NewCell(c));
                }
                self.set(*var);
                if let Some(i) = index {
                    self.set(*i);
                }
                self.loops.push(Loop { continue_at: start, breaks: Vec::new(), depth: base_depth });
                self.hoists(&body.hoists);
                self.stmts(&body.stmts, false);
                self.emit(Op::Jump(start as u32));
                self.patch(next);
                self.depth = base_depth;
                self.end_loop();
                self.free_hidden(2);
            }
            Stmt::Break | Stmt::Continue => {
                let lp = self.loops.last().expect("break/continue inside loop");
                let (extra, continue_at) = (self.depth - lp.depth, lp.continue_at);
                if extra > 0 {
                    self.emit(Op::PopN(extra as u32));
                }
                if matches!(s, Stmt::Break) {
                    let j = self.emit(Op::Jump(0));
                    self.loops.last_mut().expect("loop").breaks.push(j);
                } else {
                    self.emit(Op::Jump(continue_at as u32));
                }
                // 之后的代码不可达，深度按跳转前计
                self.depth += extra;
            }
            Stmt::Return(e) => {
                match e {
                    Some(e) => self.expr(e),
                    None => {
                        self.emit(Op::Nil);
                    }
                }
                // Return 弹出返回值后深度回到语句开始时，与正常执行完该语句一致
                self.emit(Op::Return);
            }
        }
    }

    fn end_loop(&mut self) {
        let lp = self.loops.pop().expect("loop");
        for j in lp.breaks {
            self.patch(j);
        }
    }

    fn assign(&mut self, target: &Target, op: Option<BinOp>, e: &Expr, pos: Pos) {
        match target {
            Target::Var(Var::Local(s), _, _) if op.is_some() && is_simple(e) => {
                let op = op.expect("compound");
                self.expr(e);
                if let (true, Some(Op::Const(c))) = (self.can_fuse(1), self.last_op(0)) {
                    self.pop_op();
                    self.at(pos).emit(Op::OpAssignLocalConst(op, *s, c));
                } else {
                    self.at(pos).emit(Op::OpAssignLocal(op, *s));
                }
            }
            Target::Index(Expr::Var(Var::Local(s), ..), i, ipos) if is_simple(i) && is_simple(e) => {
                self.expr(i);
                if let Some(op) = op {
                    self.emit(Op::Dup);
                    self.at(*ipos).emit(Op::IndexLocal(*s));
                    self.expr(e);
                    self.binop(op, pos);
                } else {
                    self.expr(e);
                }
                self.at(*ipos).emit(Op::SetIndexLocal(*s));
            }
            Target::Field(Expr::Var(Var::Local(s), ..), name, fpos) if is_simple(e) => {
                let n = self.name(name);
                let k = self.field_cache();
                if let Some(op) = op {
                    self.at(*fpos).emit(Op::FieldLocal(*s, n, k));
                    self.expr(e);
                    self.binop(op, pos);
                } else {
                    self.expr(e);
                }
                self.at(*fpos).emit(Op::SetFieldLocal(*s, n, k));
            }
            // `x = x op e` 与 `x op= e` 等价（e 为简单表达式时求值顺序不受影响）
            Target::Var(Var::Local(s), _, _)
                if op.is_none()
                    && matches!(e, Expr::Binary(bop, l, r, _)
                        if !matches!(bop, BinOp::And | BinOp::Or)
                            && matches!(**l, Expr::Var(Var::Local(t), ..) if t == *s)
                            && is_simple(r)) =>
            {
                let Expr::Binary(bop, _, r, bpos) = e else { unreachable!() };
                self.expr(r);
                if let (true, Some(Op::Const(c))) = (self.can_fuse(1), self.last_op(0)) {
                    self.pop_op();
                    self.at(*bpos).emit(Op::OpAssignLocalConst(*bop, *s, c));
                } else {
                    self.at(*bpos).emit(Op::OpAssignLocal(*bop, *s));
                }
            }
            Target::Var(var, name, vpos) => {
                if let Some(op) = op {
                    self.get(*var, name, *vpos);
                    self.expr(e);
                    self.binop(op, pos);
                } else {
                    self.expr(e);
                }
                self.set_var(*var);
            }
            Target::Index(c, i, ipos) => {
                self.expr(c);
                self.expr(i);
                if let Some(op) = op {
                    self.emit(Op::Dup2);
                    self.at(*ipos).emit(Op::Index);
                    self.expr(e);
                    self.binop(op, pos);
                } else {
                    self.expr(e);
                }
                self.at(*ipos).emit(Op::SetIndex);
            }
            Target::Field(obj, name, fpos) => {
                let n = self.name(name);
                let k = self.field_cache();
                self.expr(obj);
                if let Some(op) = op {
                    self.emit(Op::Dup);
                    self.at(*fpos).emit(Op::Field(n, k));
                    self.expr(e);
                    self.binop(op, pos);
                } else {
                    self.expr(e);
                }
                self.at(*fpos).emit(Op::SetField(n, k));
            }
        }
    }

    // ---------- 表达式 ----------

    /// 求值但不需要结果：语句位置的 `if` 和块不产生值，省去压入再弹出。
    fn expr_discard(&mut self, e: &Expr) {
        match e {
            Expr::If(cond, then, else_, pos) => {
                let to_else = self.cond(cond, BoolCtx::If, *pos);
                self.block(then, false);
                match else_ {
                    Some(e) => {
                        let end = self.emit(Op::Jump(0));
                        for j in to_else {
                            self.patch(j);
                        }
                        self.expr_discard(e);
                        self.patch(end);
                    }
                    None => {
                        for j in to_else {
                            self.patch(j);
                        }
                    }
                }
            }
            Expr::Block(b) => self.block(b, false),
            _ => {
                self.expr(e);
                self.emit(Op::Pop);
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Const(v) => self.constant(v),
            Expr::Var(var, name, pos) => self.get(*var, name, *pos),
            Expr::Interp(parts) => {
                for p in parts {
                    self.expr(p);
                }
                self.emit(Op::Interp(parts.len() as u32));
            }
            Expr::Array(items) => {
                for item in items {
                    self.expr(item);
                }
                self.emit(Op::Array(items.len() as u32));
            }
            Expr::Unary(op, x, pos) => {
                self.expr(x);
                self.at(*pos).emit(match op {
                    UnOp::Neg => Op::Neg,
                    UnOp::Not => Op::Not,
                });
            }
            Expr::Binary(BinOp::And, a, b, pos) => {
                self.expr(a);
                let short = self.at(*pos).emit(Op::JumpIfFalse(0, BoolCtx::And));
                self.expr(b);
                self.at(*pos).emit(Op::CheckBool(BoolCtx::And));
                let end = self.emit(Op::Jump(0));
                self.patch(short);
                self.depth -= 1;
                self.emit(Op::False);
                self.patch(end);
            }
            Expr::Binary(BinOp::Or, a, b, pos) => {
                self.expr(a);
                let rhs = self.at(*pos).emit(Op::JumpIfFalse(0, BoolCtx::Or));
                self.emit(Op::True);
                let end = self.emit(Op::Jump(0));
                self.patch(rhs);
                self.depth -= 1;
                self.expr(b);
                self.at(*pos).emit(Op::CheckBool(BoolCtx::Or));
                self.patch(end);
            }
            Expr::Binary(op, a, b, pos) => {
                self.expr(a);
                self.expr(b);
                self.binop(*op, *pos);
            }
            Expr::Range(a, b, inclusive, pos) => {
                self.expr(a);
                self.expr(b);
                self.at(*pos).emit(Op::Range(*inclusive));
            }
            Expr::Call { callee, args, names, pos } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
                let argc = args.len() as u32;
                match names {
                    Some(names) => {
                        self.chunk.name_lists.push(names.clone());
                        let idx = (self.chunk.name_lists.len() - 1) as u32;
                        self.at(*pos).emit(Op::CallNamed(argc, idx));
                    }
                    None => {
                        self.at(*pos).emit(Op::Call(argc));
                    }
                }
            }
            Expr::MethodCall { obj, name, fallback, args, pos } => {
                self.expr(obj);
                for a in args {
                    self.expr(a);
                }
                let fallback = match fallback.as_deref() {
                    None => Fallback::None,
                    Some(Expr::Var(var, n, _)) => Fallback::Var(*var, self.name(n)),
                    Some(Expr::Const(v)) => Fallback::Const(v.clone()),
                    Some(_) => unreachable!("fallback is a variable or a builtin"),
                };
                self.chunk.methods.push(MethodDesc { name: name.clone(), argc: args.len() as u32, fallback });
                let idx = (self.chunk.methods.len() - 1) as u32;
                self.at(*pos).emit(Op::Method(idx));
            }
            Expr::Index(c, i, pos) => {
                if let (Expr::Var(Var::Local(s), ..), true) = (&**c, is_simple(i)) {
                    let s = *s;
                    self.expr(i);
                    self.at(*pos).emit(Op::IndexLocal(s));
                    return;
                }
                self.expr(c);
                self.expr(i);
                self.at(*pos).emit(Op::Index);
            }
            Expr::Field(obj, name, pos) => {
                let n = self.name(name);
                let k = self.field_cache();
                if let Expr::Var(Var::Local(s), ..) = **obj {
                    self.at(*pos).emit(Op::FieldLocal(s, n, k));
                    return;
                }
                self.expr(obj);
                self.at(*pos).emit(Op::Field(n, k));
            }
            Expr::Lambda(idx) => {
                self.emit(Op::Closure(*idx));
            }
            Expr::If(cond, then, else_, pos) => {
                let to_else = self.cond(cond, BoolCtx::If, *pos);
                self.block(then, true);
                let end = self.emit(Op::Jump(0));
                for j in to_else {
                    self.patch(j);
                }
                self.depth -= 1;
                match else_ {
                    Some(e) => self.expr(e),
                    None => {
                        self.emit(Op::Nil);
                    }
                }
                self.patch(end);
            }
            Expr::Match(subject, arms, pos) => {
                self.expr(subject);
                let h = self.alloc_hidden(1);
                self.emit(Op::SetLocal(h));
                let mut ends = Vec::new();
                for arm in arms {
                    for &c in &arm.cells {
                        self.emit(Op::NewCell(c));
                    }
                    self.chunk.patterns.push(arm.pattern.clone());
                    let pidx = (self.chunk.patterns.len() - 1) as u32;
                    let mut fails = vec![self.at(*pos).emit(Op::Match(pidx, h, 0))];
                    if let Some((g, gpos)) = &arm.guard {
                        fails.extend(self.cond(g, BoolCtx::Guard, *gpos));
                    }
                    self.expr(&arm.body);
                    ends.push(self.emit(Op::Jump(0)));
                    self.depth -= 1;
                    for f in fails {
                        self.patch(f);
                    }
                }
                self.at(*pos).emit(Op::NoMatch(h));
                self.depth += 1;
                for j in ends {
                    self.patch(j);
                }
                self.free_hidden(1);
            }
            Expr::Block(b) => self.block(b, true),
        }
    }
}
