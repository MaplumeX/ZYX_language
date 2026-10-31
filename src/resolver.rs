//! 槽位解析：AST → IR。
//!
//! - 未声明的变量在这里报 `NameError`，程序不会开始执行（reference.md 9.2）。
//! - 被闭包捕获的变量要放进单元格。解析一个函数前先做一遍"预扫描"：收集其内部嵌套函数中
//!   引用到的所有名字（去掉嵌套函数自己的参数），本函数中同名的局部变量一律放进单元格。
//!   这是保守的近似：可能多放，但不会漏放。
//! - 函数体（具名函数、匿名函数、结构体默认值）推迟到所在作用域结束时才解析，因此可以引用
//!   同一作用域中稍后声明的变量；查找时优先取函数出现位置之前可见的声明，其次取之后的第一个声明。

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::ast::{self, ExprKind, Pos, StmtKind};
use crate::builtins;
use crate::error::{ErrorKind, Result, ZyxError};
use crate::ir::*;
use crate::value::Value;

pub fn resolve(prog: &ast::Program) -> Result<Program> {
    let methods = Prescan::run(prog).methods;
    let mut r = Resolver {
        fns: Vec::new(),
        protos: Vec::new(),
        interner: HashMap::new(),
        n_globals: 0,
        methods,
    };
    r.fns.push(FnCtx::new(prescan(prog), true));
    let (body, _) = r.block(prog, &[])?;
    let ctx = r.fns.pop().expect("script context");
    let main = Rc::new(FnProto {
        id: r.protos.len(),
        name: None,
        arity: 0,
        n_locals: ctx.max_locals as usize,
        n_cells: ctx.n_cells as usize,
        param_cells: Vec::new(),
        upvals: Vec::new(),
        body,
    });
    let protos = r
        .protos
        .into_iter()
        .map(|p| p.expect("every function is resolved"))
        .collect();
    Ok(Program {
        main,
        protos,
        n_globals: r.n_globals as usize,
    })
}

struct Resolver {
    /// 函数上下文栈，`fns[0]` 是脚本顶层。
    fns: Vec<FnCtx>,
    protos: Vec<Option<Rc<FnProto>>>,
    interner: HashMap<String, Rc<str>>,
    n_globals: u32,
    /// 程序中所有 impl 块定义的方法名，用于在运行前发现拼错的方法名。
    methods: HashSet<String>,
}

struct FnCtx {
    scopes: Vec<Scope>,
    /// 预扫描得到的"可能被捕获"的名字。
    captured: HashSet<String>,
    is_script: bool,
    next_local: u32,
    max_locals: u32,
    n_cells: u32,
    upvals: Vec<UpSrc>,
    /// 正在解析推迟的嵌套函数时，记录它出现时最内层作用域可见的名字个数。
    limit: Option<(usize, usize)>,
}

struct Scope {
    names: Vec<(Rc<str>, Slot)>,
    cells: Vec<u32>,
    pending: Vec<Pending>,
    local_base: u32,
}

struct Pending {
    decl: Rc<ast::FnDecl>,
    idx: u32,
    visible: usize,
}

impl FnCtx {
    fn new(captured: HashSet<String>, is_script: bool) -> Self {
        FnCtx {
            scopes: Vec::new(),
            captured,
            is_script,
            next_local: 0,
            max_locals: 0,
            n_cells: 0,
            upvals: Vec::new(),
            limit: None,
        }
    }

    fn find(&self, name: &str) -> Option<Slot> {
        for (si, scope) in self.scopes.iter().enumerate().rev() {
            let hit = |(n, _): &&(Rc<str>, Slot)| &**n == name;
            match self.limit {
                Some((lim, visible)) if lim == si => {
                    let before = scope.names[..visible].iter().rev().find(hit);
                    if let Some((_, slot)) = before.or_else(|| scope.names[visible..].iter().find(hit)) {
                        return Some(*slot);
                    }
                }
                _ => {
                    if let Some((_, slot)) = scope.names.iter().rev().find(hit) {
                        return Some(*slot);
                    }
                }
            }
        }
        None
    }
}

fn name_err(pos: Pos, msg: String) -> ZyxError {
    ZyxError::new(ErrorKind::Name, pos.line, pos.col, msg)
}

impl Resolver {
    fn intern(&mut self, s: &str) -> Rc<str> {
        if let Some(r) = self.interner.get(s) {
            return r.clone();
        }
        let r: Rc<str> = Rc::from(s);
        self.interner.insert(s.to_string(), r.clone());
        r
    }

    fn ctx(&mut self) -> &mut FnCtx {
        self.fns.last_mut().expect("function context")
    }

    fn scope(&mut self) -> &mut Scope {
        self.ctx().scopes.last_mut().expect("scope")
    }

    // ---------- 作用域与声明 ----------

    fn push_scope(&mut self) {
        let base = self.ctx().next_local;
        self.ctx().scopes.push(Scope {
            names: Vec::new(),
            cells: Vec::new(),
            pending: Vec::new(),
            local_base: base,
        });
    }

    /// 先解析推迟到本作用域结束的函数体，再弹出作用域，返回需要在进入时分配的单元格。
    fn pop_scope(&mut self) -> Result<Vec<u32>> {
        let k = self.fns.len() - 1;
        let si = self.fns[k].scopes.len() - 1;
        let pending = std::mem::take(&mut self.fns[k].scopes[si].pending);
        for p in pending {
            let saved = self.fns[k].limit.replace((si, p.visible));
            let mut proto = self.function(&p.decl)?;
            proto.id = p.idx as usize;
            self.fns[k].limit = saved;
            self.protos[p.idx as usize] = Some(Rc::new(proto));
        }
        let ctx = &mut self.fns[k];
        let scope = ctx.scopes.pop().expect("scope");
        ctx.next_local = scope.local_base;
        Ok(scope.cells)
    }

    fn declare(&mut self, name: &str) -> Slot {
        let name = self.intern(name);
        let ctx = self.fns.last_mut().expect("function context");
        let slot = if ctx.is_script && ctx.scopes.len() == 1 {
            self.n_globals += 1;
            Slot::Global(self.n_globals - 1)
        } else if ctx.captured.contains(&*name) {
            ctx.n_cells += 1;
            let c = ctx.n_cells - 1;
            ctx.scopes.last_mut().expect("scope").cells.push(c);
            Slot::Cell(c)
        } else {
            ctx.next_local += 1;
            ctx.max_locals = ctx.max_locals.max(ctx.next_local);
            Slot::Local(ctx.next_local - 1)
        };
        ctx.scopes.last_mut().expect("scope").names.push((name, slot));
        slot
    }

    fn placeholder(&mut self) -> u32 {
        self.protos.push(None);
        (self.protos.len() - 1) as u32
    }

    /// 登记一个推迟到当前作用域结束时解析的函数体。
    fn defer(&mut self, decl: Rc<ast::FnDecl>, idx: u32) {
        let scope = self.scope();
        let visible = scope.names.len();
        scope.pending.push(Pending { decl, idx, visible });
    }

    // ---------- 名字查找 ----------

    fn lookup(&mut self, name: &str) -> Option<Var> {
        let k = self.fns.len() - 1;
        self.lookup_in(k, name)
    }

    fn lookup_in(&mut self, k: usize, name: &str) -> Option<Var> {
        if let Some(slot) = self.fns[k].find(name) {
            return Some(slot.into());
        }
        if k == 0 {
            return None;
        }
        let src = match self.lookup_in(k - 1, name)? {
            Var::Global(g) => return Some(Var::Global(g)),
            Var::Cell(c) => UpSrc::Cell(c),
            Var::Upval(u) => UpSrc::Upval(u),
            Var::Local(_) => unreachable!("prescan missed captured variable '{name}'"),
        };
        let upvals = &mut self.fns[k].upvals;
        let i = match upvals.iter().position(|u| *u == src) {
            Some(i) => i,
            None => {
                upvals.push(src);
                upvals.len() - 1
            }
        };
        Some(Var::Upval(i as u32))
    }

    /// 读取一个名字：用户变量优先，其次是内置函数。
    fn name_expr(&mut self, name: &str, pos: Pos) -> Option<Expr> {
        if let Some(var) = self.lookup(name) {
            return Some(Expr::Var(var, self.intern(name), pos));
        }
        builtins::lookup(name).map(|b| Expr::Const(Value::Builtin(b)))
    }

    // ---------- 函数与块 ----------

    fn function(&mut self, decl: &ast::FnDecl) -> Result<FnProto> {
        self.fns.push(FnCtx::new(prescan(&decl.body.stmts), false));
        self.push_scope();
        let mut param_cells = Vec::new();
        for (i, p) in decl.params.iter().enumerate() {
            let name = self.intern(p);
            let ctx = self.ctx();
            let slot = if ctx.captured.contains(p) {
                ctx.n_cells += 1;
                param_cells.push((i, ctx.n_cells - 1));
                Slot::Cell(ctx.n_cells - 1)
            } else {
                Slot::Local(i as u32)
            };
            ctx.next_local = i as u32 + 1;
            ctx.max_locals = ctx.next_local;
            ctx.scopes.last_mut().expect("scope").names.push((name, slot));
        }
        let (body, _) = self.block(&decl.body.stmts, &[])?;
        self.pop_scope()?;
        let ctx = self.fns.pop().expect("function context");
        Ok(FnProto {
            id: 0,
            name: decl.name.as_deref().map(|n| self.intern(n)),
            arity: decl.params.len(),
            n_locals: ctx.max_locals as usize,
            n_cells: ctx.n_cells as usize,
            param_cells,
            upvals: ctx.upvals,
            body,
        })
    }

    /// 解析一个块。`pre` 是在块作用域开头预先声明的名字（`for` 的循环变量）。
    fn block(&mut self, stmts: &[ast::Stmt], pre: &[&str]) -> Result<(Block, Vec<Slot>)> {
        self.push_scope();
        let pre_slots: Vec<Slot> = pre.iter().map(|n| self.declare(n)).collect();

        // 提升：先声明本块所有结构体和函数的名字
        let mut structs = Vec::new();
        let mut fn_hoists = Vec::new();
        let mut fn_idx = vec![0u32; stmts.len()];
        for (i, s) in stmts.iter().enumerate() {
            match &s.kind {
                StmtKind::Struct(d) => {
                    let slot = self.declare(&d.name);
                    structs.push((slot, d.clone()));
                }
                StmtKind::Fn(f) => {
                    let slot = self.declare(f.name.as_deref().expect("named fn"));
                    let idx = self.placeholder();
                    fn_idx[i] = idx;
                    fn_hoists.push(Hoist::Fn(slot, idx));
                }
                _ => {}
            }
        }

        let mut hoists = Vec::new();
        for (slot, d) in structs {
            let mut defaults = Vec::new();
            for (_, default) in &d.fields {
                defaults.push(default.as_ref().map(|e| {
                    let decl = Rc::new(ast::FnDecl {
                        name: None,
                        params: Vec::new(),
                        body: ast::Block {
                            stmts: vec![ast::Stmt {
                                kind: StmtKind::Expr(e.clone()),
                                pos: e.pos,
                            }],
                            pos: e.pos,
                        },
                        pos: e.pos,
                    });
                    let idx = self.placeholder();
                    self.defer(decl, idx);
                    idx
                }));
            }
            let fields = d.fields.iter().map(|(f, _)| self.intern(f)).collect();
            let name = self.intern(&d.name);
            hoists.push(Hoist::Struct(slot, Rc::new(StructProto { name, fields, defaults })));
        }
        hoists.extend(fn_hoists);

        let mut impl_idx: Vec<Vec<u32>> = vec![Vec::new(); stmts.len()];
        for (i, s) in stmts.iter().enumerate() {
            if let StmtKind::Impl { name, methods } = &s.kind {
                let Some(target) = self.lookup(name) else {
                    return Err(name_err(s.pos, format!("undefined struct '{name}'")));
                };
                let mut ms = Vec::new();
                for m in methods {
                    let idx = self.placeholder();
                    impl_idx[i].push(idx);
                    ms.push((self.intern(m.name.as_deref().expect("named method")), idx));
                }
                hoists.push(Hoist::Impl {
                    target,
                    name: self.intern(name),
                    methods: ms,
                    pos: s.pos,
                });
            }
        }

        // 按顺序解析语句；声明在此登记推迟解析的函数体，记录它们出现时的可见范围
        let mut out = Vec::new();
        for (i, s) in stmts.iter().enumerate() {
            match &s.kind {
                StmtKind::Fn(f) => self.defer(f.clone(), fn_idx[i]),
                StmtKind::Struct(_) => {}
                StmtKind::Impl { methods, .. } => {
                    for (m, idx) in methods.iter().zip(&impl_idx[i]) {
                        self.defer(m.clone(), *idx);
                    }
                }
                _ => out.push(self.stmt(s)?),
            }
        }
        // 块的值是最后一条语句的值；最后一条是声明时值为 nil
        if stmts
            .last()
            .is_some_and(|s| matches!(s.kind, StmtKind::Fn(_) | StmtKind::Struct(_) | StmtKind::Impl { .. }))
        {
            out.push(Stmt::Expr(Expr::Const(Value::Nil)));
        }

        let cells = self.pop_scope()?;
        Ok((
            Block {
                cells,
                hoists,
                stmts: out,
            },
            pre_slots,
        ))
    }

    // ---------- 语句 ----------

    fn stmt(&mut self, s: &ast::Stmt) -> Result<Stmt> {
        Ok(match &s.kind {
            StmtKind::Let { name, init } => {
                let init = match init {
                    Some(e) => self.expr(e)?,
                    None => Expr::Const(Value::Nil),
                };
                Stmt::Let(self.declare(name), init)
            }
            StmtKind::Assign { target, op, value } => {
                let t = match &target.kind {
                    ExprKind::Ident(name) => match self.lookup(name) {
                        Some(var) => Target::Var(var, self.intern(name), target.pos),
                        None => return Err(name_err(target.pos, format!("undefined variable '{name}'"))),
                    },
                    ExprKind::Index(a, i) => Target::Index(self.expr(a)?, self.expr(i)?, target.pos),
                    ExprKind::Field(a, f) => Target::Field(self.expr(a)?, self.intern(f), target.pos),
                    _ => unreachable!("parser only produces valid assignment targets"),
                };
                Stmt::Assign(t, *op, self.expr(value)?, target.pos)
            }
            StmtKind::Expr(e) => Stmt::Expr(self.expr(e)?),
            StmtKind::While { cond, body } => {
                let c = self.expr(cond)?;
                Stmt::While(c, self.block(&body.stmts, &[])?.0, cond.pos)
            }
            StmtKind::For { index, var, iter, body } => {
                let it = self.expr(iter)?;
                let mut pre = Vec::new();
                if let Some(i) = index {
                    pre.push(i.as_str());
                }
                pre.push(var.as_str());
                let (b, slots) = self.block(&body.stmts, &pre)?;
                let (index, var) = if index.is_some() {
                    (Some(slots[0]), slots[1])
                } else {
                    (None, slots[0])
                };
                Stmt::For {
                    index,
                    var,
                    iter: it,
                    body: b,
                    pos: iter.pos,
                }
            }
            StmtKind::Break => Stmt::Break,
            StmtKind::Continue => Stmt::Continue,
            StmtKind::Return(e) => Stmt::Return(e.as_ref().map(|e| self.expr(e)).transpose()?),
            StmtKind::Fn(_) | StmtKind::Struct(_) | StmtKind::Impl { .. } => unreachable!("declarations are hoisted"),
        })
    }

    // ---------- 表达式 ----------

    fn exprs(&mut self, es: &[ast::Expr]) -> Result<Vec<Expr>> {
        es.iter().map(|e| self.expr(e)).collect()
    }

    fn boxed(&mut self, e: &ast::Expr) -> Result<Box<Expr>> {
        Ok(Box::new(self.expr(e)?))
    }

    fn expr(&mut self, e: &ast::Expr) -> Result<Expr> {
        let pos = e.pos;
        Ok(match &e.kind {
            ExprKind::Int(n) => Expr::Const(Value::Int(*n)),
            ExprKind::Float(x) => Expr::Const(Value::Float(*x)),
            ExprKind::Str(s) => Expr::Const(Value::str(s)),
            ExprKind::Bool(b) => Expr::Const(Value::Bool(*b)),
            ExprKind::Nil => Expr::Const(Value::Nil),
            ExprKind::Interp(parts) => {
                let mut out = Vec::new();
                for p in parts {
                    out.push(match p {
                        ast::InterpPart::Lit(s) => Expr::Const(Value::str(s)),
                        ast::InterpPart::Expr(e) => self.expr(e)?,
                    });
                }
                Expr::Interp(out)
            }
            ExprKind::Ident(name) => match self.name_expr(name, pos) {
                Some(e) => e,
                None => return Err(name_err(pos, format!("undefined variable '{name}'"))),
            },
            ExprKind::Array(items) => Expr::Array(self.exprs(items)?),
            ExprKind::Unary(op, x) => Expr::Unary(*op, self.boxed(x)?, pos),
            ExprKind::Binary(op, a, b) => Expr::Binary(*op, self.boxed(a)?, self.boxed(b)?, pos),
            ExprKind::Range { start, end, inclusive } => {
                Expr::Range(self.boxed(start)?, self.boxed(end)?, *inclusive, pos)
            }
            ExprKind::Call { callee, args } => {
                let names = if args.iter().any(|a| a.name.is_some()) {
                    Some(
                        args.iter()
                            .map(|a| self.intern(a.name.as_deref().unwrap_or_default()))
                            .collect(),
                    )
                } else {
                    None
                };
                let callee = self.boxed(callee)?;
                let values: Vec<ast::Expr> = args.iter().map(|a| a.value.clone()).collect();
                Expr::Call {
                    callee,
                    args: self.exprs(&values)?,
                    names,
                    pos,
                }
            }
            ExprKind::MethodCall { obj, name, args } => {
                if args.iter().any(|a| a.name.is_some()) {
                    return Err(ZyxError::new(
                        ErrorKind::Type,
                        pos.line,
                        pos.col,
                        "named arguments are only allowed when constructing a struct",
                    ));
                }
                let obj = self.boxed(obj)?;
                let fallback = self.name_expr(name, pos).map(Box::new);
                if fallback.is_none() && !self.methods.contains(name) {
                    return Err(name_err(pos, format!("undefined function or method '{name}'")));
                }
                let values: Vec<ast::Expr> = args.iter().map(|a| a.value.clone()).collect();
                Expr::MethodCall {
                    obj,
                    name: self.intern(name),
                    fallback,
                    args: self.exprs(&values)?,
                    pos,
                }
            }
            ExprKind::Index(a, i) => Expr::Index(self.boxed(a)?, self.boxed(i)?, pos),
            ExprKind::Field(a, f) => Expr::Field(self.boxed(a)?, self.intern(f), pos),
            ExprKind::Lambda(f) => {
                let idx = self.placeholder();
                self.defer(f.clone(), idx);
                Expr::Lambda(idx)
            }
            ExprKind::If { cond, then, else_ } => {
                let c = self.boxed(cond)?;
                let t = self.block(&then.stmts, &[])?.0;
                let e = match else_ {
                    Some(e) => Some(self.boxed(e)?),
                    None => None,
                };
                Expr::If(c, t, e, cond.pos)
            }
            ExprKind::Match { subject, arms } => {
                let subj = self.boxed(subject)?;
                let mut out = Vec::new();
                for arm in arms {
                    self.push_scope();
                    let pattern = self.pattern(&arm.pattern);
                    let guard = match &arm.guard {
                        Some(g) => Some((self.expr(g)?, g.pos)),
                        None => None,
                    };
                    let body = self.expr(&arm.body)?;
                    let cells = self.pop_scope()?;
                    out.push(Arm {
                        cells,
                        pattern,
                        guard,
                        body,
                    });
                }
                Expr::Match(subj, out, pos)
            }
            ExprKind::Block(b) => Expr::Block(self.block(&b.stmts, &[])?.0),
        })
    }

    fn pattern(&mut self, p: &ast::Pattern) -> Pattern {
        match p {
            ast::Pattern::Wildcard => Pattern::Wildcard,
            ast::Pattern::Bind(name) => Pattern::Bind(self.declare(name)),
            ast::Pattern::Int(n) => Pattern::Lit(Value::Int(*n)),
            ast::Pattern::Float(x) => Pattern::Lit(Value::Float(*x)),
            ast::Pattern::Str(s) => Pattern::Lit(Value::str(s)),
            ast::Pattern::Bool(b) => Pattern::Lit(Value::Bool(*b)),
            ast::Pattern::Nil => Pattern::Lit(Value::Nil),
            ast::Pattern::Range { lo, hi, inclusive } => Pattern::Range {
                lo: *lo,
                hi: *hi,
                inclusive: *inclusive,
            },
            ast::Pattern::Or(ps) => Pattern::Or(ps.iter().map(|p| self.pattern(p)).collect()),
        }
    }
}

// ---------- 预扫描：找出可能被嵌套函数捕获的名字 ----------

/// 返回 `stmts`（一个函数体）内部所有嵌套函数中引用到的名字，嵌套函数自己的参数除外。
fn prescan(stmts: &[ast::Stmt]) -> HashSet<String> {
    Prescan::run(stmts).out
}

struct Prescan {
    out: HashSet<String>,
    /// 顺带收集遇到的所有 impl 方法名
    methods: HashSet<String>,
    /// 当前所在嵌套函数（及其外层嵌套函数）的参数名
    excluded: Vec<String>,
    depth: usize,
}

impl Prescan {
    fn run(stmts: &[ast::Stmt]) -> Self {
        let mut p = Prescan {
            out: HashSet::new(),
            methods: HashSet::new(),
            excluded: Vec::new(),
            depth: 0,
        };
        p.stmts(stmts);
        p
    }

    fn note(&mut self, name: &str) {
        if self.depth > 0 && !self.excluded.iter().any(|e| e == name) {
            self.out.insert(name.to_string());
        }
    }

    fn nested(&mut self, params: &[String], body: impl FnOnce(&mut Self)) {
        self.depth += 1;
        let n = self.excluded.len();
        self.excluded.extend(params.iter().cloned());
        body(self);
        self.excluded.truncate(n);
        self.depth -= 1;
    }

    fn func(&mut self, f: &ast::FnDecl) {
        self.nested(&f.params, |p| p.stmts(&f.body.stmts));
    }

    fn stmts(&mut self, stmts: &[ast::Stmt]) {
        for s in stmts {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &ast::Stmt) {
        match &s.kind {
            StmtKind::Let { init, .. } => {
                if let Some(e) = init {
                    self.expr(e);
                }
            }
            StmtKind::Assign { target, value, .. } => {
                self.expr(target);
                self.expr(value);
            }
            StmtKind::Expr(e) => self.expr(e),
            StmtKind::Fn(f) => self.func(f),
            StmtKind::Struct(d) => {
                for e in d.fields.iter().filter_map(|(_, d)| d.as_ref()) {
                    self.nested(&[], |p| p.expr(e));
                }
            }
            StmtKind::Impl { name, methods } => {
                self.note(name);
                for m in methods {
                    self.methods.extend(m.name.clone());
                    self.func(m);
                }
            }
            StmtKind::While { cond, body } => {
                self.expr(cond);
                self.stmts(&body.stmts);
            }
            StmtKind::For { iter, body, .. } => {
                self.expr(iter);
                self.stmts(&body.stmts);
            }
            StmtKind::Return(e) => {
                if let Some(e) = e {
                    self.expr(e);
                }
            }
            StmtKind::Break | StmtKind::Continue => {}
        }
    }

    fn expr(&mut self, e: &ast::Expr) {
        match &e.kind {
            ExprKind::Ident(name) => self.note(name),
            ExprKind::Interp(parts) => {
                for p in parts {
                    if let ast::InterpPart::Expr(e) = p {
                        self.expr(e);
                    }
                }
            }
            ExprKind::Array(items) => items.iter().for_each(|e| self.expr(e)),
            ExprKind::Unary(_, x) => self.expr(x),
            ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            ExprKind::Range { start, end, .. } => {
                self.expr(start);
                self.expr(end);
            }
            ExprKind::Call { callee, args } => {
                self.expr(callee);
                args.iter().for_each(|a| self.expr(&a.value));
            }
            ExprKind::MethodCall { obj, name, args } => {
                self.note(name);
                self.expr(obj);
                args.iter().for_each(|a| self.expr(&a.value));
            }
            ExprKind::Field(a, _) => self.expr(a),
            ExprKind::Lambda(f) => self.func(f),
            ExprKind::If { cond, then, else_ } => {
                self.expr(cond);
                self.stmts(&then.stmts);
                if let Some(e) = else_ {
                    self.expr(e);
                }
            }
            ExprKind::Match { subject, arms } => {
                self.expr(subject);
                for arm in arms {
                    if let Some(g) = &arm.guard {
                        self.expr(g);
                    }
                    self.expr(&arm.body);
                }
            }
            ExprKind::Block(b) => self.stmts(&b.stmts),
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Str(_) | ExprKind::Bool(_) | ExprKind::Nil => {}
        }
    }
}
