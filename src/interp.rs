//! 树遍历执行后端：直接执行 IR。
//!
//! 所有调用帧共用一个值栈，帧内局部变量位于 `stack[base + slot]`；被捕获的变量在帧的单元格数组中。

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::{BinOp, Pos};
use crate::builtins::{self, Io, Runtime};
use crate::error::{ErrorKind, Result, ZyxError};
use crate::ir::*;
use crate::ops::{self, err, expect_bool, type_err};
use crate::value::{Cell, Closure, StructType, Value, values_equal};

/// 调用深度上限（reference.md 5.4）。
const MAX_DEPTH: usize = 10_000;

/// 控制流与错误统一走 `Err` 通道，便于用 `?` 传播。
enum Ctrl {
    Break,
    Continue,
    Return(Value),
    Error(Box<ZyxError>),
}

impl From<ZyxError> for Ctrl {
    fn from(e: ZyxError) -> Self {
        Ctrl::Error(Box::new(e))
    }
}

type R<T> = std::result::Result<T, Ctrl>;

struct Frame<'a> {
    base: usize,
    cells: Vec<Cell>,
    upvals: &'a [Cell],
}

pub struct Interp {
    stack: Vec<Value>,
    globals: Vec<Value>,
    protos: Vec<Rc<FnProto>>,
    depth: usize,
    io: Io,
    /// 帧单元格数组的占位值，进入块时替换为新单元格。
    placeholder: Cell,
}

impl Runtime for Interp {
    fn call(&mut self, f: &Value, args: Vec<Value>, pos: Pos) -> Result<Value> {
        self.call_value(f, args, None, pos).map_err(into_error)
    }

    fn io(&mut self) -> &mut Io {
        &mut self.io
    }
}

fn into_error(c: Ctrl) -> ZyxError {
    match c {
        Ctrl::Error(e) => *e,
        // break / continue / return 不会越过函数边界（语法分析已检查）
        _ => ZyxError::new(
            ErrorKind::Runtime,
            0,
            0,
            "internal error: control flow escaped a function",
        ),
    }
}

fn new_cell(v: Value) -> Cell {
    Rc::new(RefCell::new(v))
}

impl Interp {
    pub fn new(program: &Program) -> Self {
        Interp {
            stack: Vec::with_capacity(1024),
            globals: vec![Value::Undef; program.n_globals],
            protos: program.protos.clone(),
            depth: 0,
            io: Io::default(),
            placeholder: new_cell(Value::Undef),
        }
    }

    pub fn run(&mut self, program: &Program) -> Result<()> {
        let main = &program.main;
        self.stack.resize(main.n_locals, Value::Nil);
        let mut frame = Frame {
            base: 0,
            cells: vec![self.placeholder.clone(); main.n_cells],
            upvals: &[],
        };
        let r = self.exec_block(&main.body, &mut frame);
        self.io.flush();
        r.map(|_| ()).map_err(into_error)
    }

    pub fn flush(&mut self) {
        self.io.flush();
    }

    // ---------- 变量 ----------

    fn load(&self, var: Var, name: &str, pos: Pos, f: &Frame) -> R<Value> {
        let v = match var {
            Var::Local(s) => return Ok(self.stack[f.base + s as usize].clone()),
            Var::Global(g) => self.globals[g as usize].clone(),
            Var::Cell(c) => f.cells[c as usize].borrow().clone(),
            Var::Upval(u) => f.upvals[u as usize].borrow().clone(),
        };
        if let Value::Undef = v {
            return Err(err(
                ErrorKind::Name,
                pos,
                format!("variable '{name}' is used before it is initialized"),
            )
            .into());
        }
        Ok(v)
    }

    fn store_var(&mut self, var: Var, v: Value, f: &Frame) {
        match var {
            Var::Local(s) => self.stack[f.base + s as usize] = v,
            Var::Global(g) => self.globals[g as usize] = v,
            Var::Cell(c) => *f.cells[c as usize].borrow_mut() = v,
            Var::Upval(u) => *f.upvals[u as usize].borrow_mut() = v,
        }
    }

    fn store(&mut self, slot: Slot, v: Value, f: &Frame) {
        self.store_var(slot.into(), v, f);
    }

    fn make_closure(&self, idx: u32, f: &Frame) -> Rc<Closure> {
        let proto = self.protos[idx as usize].clone();
        let upvals = proto
            .upvals
            .iter()
            .map(|src| match *src {
                UpSrc::Cell(c) => f.cells[c as usize].clone(),
                UpSrc::Upval(u) => f.upvals[u as usize].clone(),
            })
            .collect();
        Rc::new(Closure { proto, upvals })
    }

    // ---------- 块与语句 ----------

    fn alloc_cells(cells: &[u32], f: &mut Frame) {
        for &c in cells {
            f.cells[c as usize] = new_cell(Value::Undef);
        }
    }

    fn run_hoists(&mut self, hoists: &[Hoist], f: &mut Frame) -> R<()> {
        for h in hoists {
            match h {
                Hoist::Struct(slot, proto) => {
                    let defaults = proto
                        .defaults
                        .iter()
                        .map(|d| d.map(|idx| self.make_closure(idx, f)))
                        .collect();
                    let ty = StructType {
                        name: proto.name.clone(),
                        fields: proto.fields.clone(),
                        defaults,
                        methods: RefCell::new(Vec::new()),
                    };
                    self.store(*slot, Value::StructType(Rc::new(ty)), f);
                }
                Hoist::Fn(slot, idx) => {
                    let c = self.make_closure(*idx, f);
                    self.store(*slot, Value::Func(c), f);
                }
                Hoist::Impl {
                    target,
                    name,
                    methods,
                    pos,
                } => {
                    let ty = self.load(*target, name, *pos, f)?;
                    let ms = methods
                        .iter()
                        .map(|(n, idx)| (n.clone(), self.make_closure(*idx, f)))
                        .collect();
                    ops::add_methods(&ty, name, ms, *pos)?;
                }
            }
        }
        Ok(())
    }

    fn exec_block(&mut self, b: &Block, f: &mut Frame) -> R<Value> {
        Self::alloc_cells(&b.cells, f);
        self.run_hoists(&b.hoists, f)?;
        self.run_stmts(&b.stmts, f)
    }

    /// 执行语句序列，返回最后一条表达式语句的值。
    fn run_stmts(&mut self, stmts: &[Stmt], f: &mut Frame) -> R<Value> {
        let Some((last, init)) = stmts.split_last() else {
            return Ok(Value::Nil);
        };
        for s in init {
            self.exec(s, f)?;
        }
        match last {
            Stmt::Expr(e) => self.eval(e, f),
            s => {
                self.exec(s, f)?;
                Ok(Value::Nil)
            }
        }
    }

    fn exec(&mut self, s: &Stmt, f: &mut Frame) -> R<()> {
        match s {
            Stmt::Let(slot, e) => {
                let v = self.eval(e, f)?;
                self.store(*slot, v, f);
            }
            Stmt::Assign(target, op, e, pos) => self.assign(target, *op, e, *pos, f)?,
            Stmt::Expr(e) => {
                self.eval(e, f)?;
            }
            Stmt::While(cond, body, pos) => loop {
                let c = self.eval(cond, f)?;
                if !expect_bool(&c, *pos, "while condition")? {
                    break;
                }
                match self.exec_block(body, f) {
                    Ok(_) | Err(Ctrl::Continue) => {}
                    Err(Ctrl::Break) => break,
                    Err(e) => return Err(e),
                }
            },
            Stmt::For {
                index,
                var,
                iter,
                body,
                pos,
            } => self.exec_for(*index, *var, iter, body, *pos, f)?,
            Stmt::Break => return Err(Ctrl::Break),
            Stmt::Continue => return Err(Ctrl::Continue),
            Stmt::Return(e) => {
                let v = match e {
                    Some(e) => self.eval(e, f)?,
                    None => Value::Nil,
                };
                return Err(Ctrl::Return(v));
            }
        }
        Ok(())
    }

    fn assign(&mut self, target: &Target, op: Option<BinOp>, e: &Expr, pos: Pos, f: &mut Frame) -> R<()> {
        match target {
            Target::Var(var, name, vpos) => {
                let v = match op {
                    Some(op) => {
                        let cur = self.load(*var, name, *vpos, f)?;
                        let rhs = self.eval(e, f)?;
                        ops::binary(op, &cur, &rhs, pos)?
                    }
                    None => self.eval(e, f)?,
                };
                self.store_var(*var, v, f);
            }
            Target::Index(c, i, ipos) => {
                let c = self.eval(c, f)?;
                let i = self.eval(i, f)?;
                let v = match op {
                    Some(op) => {
                        let cur = ops::index_get(&c, &i, *ipos)?;
                        let rhs = self.eval(e, f)?;
                        ops::binary(op, &cur, &rhs, pos)?
                    }
                    None => self.eval(e, f)?,
                };
                ops::index_set(&c, &i, v, *ipos)?;
            }
            Target::Field(obj, name, fpos) => {
                let obj = self.eval(obj, f)?;
                let v = match op {
                    Some(op) => {
                        let cur = ops::field_get(&obj, name, *fpos)?;
                        let rhs = self.eval(e, f)?;
                        ops::binary(op, &cur, &rhs, pos)?
                    }
                    None => self.eval(e, f)?,
                };
                ops::field_set(&obj, name, v, *fpos)?;
            }
        }
        Ok(())
    }

    /// 执行一轮循环体；返回 `false` 表示 `break`。
    fn loop_iteration(
        &mut self,
        index: Option<(Slot, i64)>,
        var: Slot,
        item: Value,
        body: &Block,
        f: &mut Frame,
    ) -> R<bool> {
        Self::alloc_cells(&body.cells, f);
        if let Some((slot, i)) = index {
            self.store(slot, Value::Int(i), f);
        }
        self.store(var, item, f);
        let r = self
            .run_hoists(&body.hoists, f)
            .and_then(|_| self.run_stmts(&body.stmts, f));
        match r {
            Ok(_) | Err(Ctrl::Continue) => Ok(true),
            Err(Ctrl::Break) => Ok(false),
            Err(e) => Err(e),
        }
    }

    fn exec_for(
        &mut self,
        index: Option<Slot>,
        var: Slot,
        iter: &Expr,
        body: &Block,
        pos: Pos,
        f: &mut Frame,
    ) -> R<()> {
        let it = self.eval(iter, f)?;
        let idx = |i: i64| index.map(|s| (s, i));
        match it {
            Value::Range(r) => {
                let (start, end) = *r;
                for n in start..end {
                    if !self.loop_iteration(idx(n - start), var, Value::Int(n), body, f)? {
                        break;
                    }
                }
            }
            Value::Array(a) => {
                let mut i = 0;
                loop {
                    let item = match a.borrow().get(i) {
                        Some(v) => v.clone(),
                        None => break,
                    };
                    if !self.loop_iteration(idx(i as i64), var, item, body, f)? {
                        break;
                    }
                    i += 1;
                }
            }
            Value::Str(s) => {
                for (i, c) in s.chars().enumerate() {
                    let item = Value::str(c.encode_utf8(&mut [0; 4]));
                    if !self.loop_iteration(idx(i as i64), var, item, body, f)? {
                        break;
                    }
                }
            }
            other => return Err(type_err(pos, format!("cannot iterate over {}", other.type_name())).into()),
        }
        Ok(())
    }

    // ---------- 表达式 ----------

    fn eval(&mut self, e: &Expr, f: &mut Frame) -> R<Value> {
        match e {
            Expr::Const(v) => Ok(v.clone()),
            Expr::Var(var, name, pos) => self.load(*var, name, *pos, f),
            Expr::Interp(parts) => {
                let mut s = String::new();
                for p in parts {
                    self.eval(p, f)?.write_display(&mut s);
                }
                Ok(Value::string(s))
            }
            Expr::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.eval(item, f)?);
                }
                Ok(Value::array(out))
            }
            Expr::Unary(op, x, pos) => {
                let v = self.eval(x, f)?;
                Ok(ops::unary(*op, &v, *pos)?)
            }
            Expr::Binary(BinOp::And, a, b, pos) => {
                let l = self.eval(a, f)?;
                if !expect_bool(&l, *pos, "operand of 'and'")? {
                    return Ok(Value::Bool(false));
                }
                let r = self.eval(b, f)?;
                Ok(Value::Bool(expect_bool(&r, *pos, "operand of 'and'")?))
            }
            Expr::Binary(BinOp::Or, a, b, pos) => {
                let l = self.eval(a, f)?;
                if expect_bool(&l, *pos, "operand of 'or'")? {
                    return Ok(Value::Bool(true));
                }
                let r = self.eval(b, f)?;
                Ok(Value::Bool(expect_bool(&r, *pos, "operand of 'or'")?))
            }
            Expr::Binary(op, a, b, pos) => {
                let l = self.eval(a, f)?;
                let r = self.eval(b, f)?;
                Ok(ops::binary(*op, &l, &r, *pos)?)
            }
            Expr::Range(a, b, inclusive, pos) => {
                let a = self.eval(a, f)?;
                let b = self.eval(b, f)?;
                Ok(ops::make_range(&a, &b, *inclusive, *pos)?)
            }
            Expr::Call {
                callee,
                args,
                names,
                pos,
            } => {
                let fv = self.eval(callee, f)?;
                if let Value::Func(clo) = &fv {
                    if names.is_some() {
                        return Err(ops::named_args_error(*pos).into());
                    }
                    let base = self.stack.len();
                    for a in args {
                        let v = self.eval(a, f)?;
                        self.stack.push(v);
                    }
                    return self.invoke(clo, base, *pos);
                }
                let vals = self.eval_args(args, f)?;
                self.call_value(&fv, vals, names.as_deref(), *pos)
            }
            Expr::MethodCall {
                obj,
                name,
                fallback,
                args,
                pos,
            } => {
                let o = self.eval(obj, f)?;
                let callee = match (ops::find_method(&o, name), fallback) {
                    (Some(m), _) => Value::Func(m),
                    (None, Some(fb)) => self.eval(fb, f)?,
                    (None, None) => return Err(ops::no_method_error(&o, name, *pos).into()),
                };
                if let Value::Func(clo) = &callee {
                    let base = self.stack.len();
                    self.stack.push(o);
                    for a in args {
                        let v = self.eval(a, f)?;
                        self.stack.push(v);
                    }
                    return self.invoke(clo, base, *pos);
                }
                let mut vals = Vec::with_capacity(args.len() + 1);
                vals.push(o);
                for a in args {
                    vals.push(self.eval(a, f)?);
                }
                self.call_value(&callee, vals, None, *pos)
            }
            Expr::Index(c, i, pos) => {
                let c = self.eval(c, f)?;
                let i = self.eval(i, f)?;
                Ok(ops::index_get(&c, &i, *pos)?)
            }
            Expr::Field(obj, name, pos) => {
                let o = self.eval(obj, f)?;
                Ok(ops::field_get(&o, name, *pos)?)
            }
            Expr::Lambda(idx) => Ok(Value::Func(self.make_closure(*idx, f))),
            Expr::If(cond, then, else_, pos) => {
                let c = self.eval(cond, f)?;
                if expect_bool(&c, *pos, "if condition")? {
                    self.exec_block(then, f)
                } else if let Some(e) = else_ {
                    self.eval(e, f)
                } else {
                    Ok(Value::Nil)
                }
            }
            Expr::Match(subject, arms, pos) => {
                let v = self.eval(subject, f)?;
                for arm in arms {
                    Self::alloc_cells(&arm.cells, f);
                    if !self.match_pattern(&arm.pattern, &v, f) {
                        continue;
                    }
                    if let Some((g, gpos)) = &arm.guard {
                        let ok = self.eval(g, f)?;
                        if !expect_bool(&ok, *gpos, "match guard")? {
                            continue;
                        }
                    }
                    return self.eval(&arm.body, f);
                }
                Err(err(ErrorKind::Runtime, *pos, format!("no match arm for value {}", v.repr())).into())
            }
            Expr::Block(b) => self.exec_block(b, f),
        }
    }

    fn eval_args(&mut self, args: &[Expr], f: &mut Frame) -> R<Vec<Value>> {
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            vals.push(self.eval(a, f)?);
        }
        Ok(vals)
    }

    fn match_pattern(&mut self, p: &Pattern, v: &Value, f: &mut Frame) -> bool {
        match p {
            Pattern::Wildcard => true,
            Pattern::Bind(slot) => {
                self.store(*slot, v.clone(), f);
                true
            }
            Pattern::Lit(lit) => values_equal(v, lit),
            Pattern::Range { lo, hi, inclusive } => match v {
                Value::Int(n) => {
                    lo.is_none_or(|l| *n >= l) && hi.is_none_or(|h| if *inclusive { *n <= h } else { *n < h })
                }
                _ => false,
            },
            Pattern::Or(ps) => ps.iter().any(|p| self.match_pattern(p, v, f)),
        }
    }

    // ---------- 调用 ----------

    /// 调用闭包；实参已压在 `stack[base..]`。
    fn invoke(&mut self, clo: &Rc<Closure>, base: usize, pos: Pos) -> R<Value> {
        let p = &clo.proto;
        let argc = self.stack.len() - base;
        if argc != p.arity {
            self.stack.truncate(base);
            return Err(ops::arity_error(p, argc, pos).into());
        }
        if self.depth >= MAX_DEPTH {
            return Err(ops::stack_overflow(pos).into());
        }
        self.depth += 1;
        self.stack.resize(base + p.n_locals, Value::Nil);
        let mut cells = if p.n_cells > 0 {
            vec![self.placeholder.clone(); p.n_cells]
        } else {
            Vec::new()
        };
        for &(i, c) in &p.param_cells {
            cells[c as usize] = new_cell(self.stack[base + i].clone());
        }
        let mut frame = Frame {
            base,
            cells,
            upvals: &clo.upvals,
        };
        let r = self.exec_block(&p.body, &mut frame);
        self.stack.truncate(base);
        self.depth -= 1;
        match r {
            Ok(v) | Err(Ctrl::Return(v)) => Ok(v),
            Err(e) => Err(e),
        }
    }

    fn call_value(&mut self, fv: &Value, args: Vec<Value>, names: Option<&[Rc<str>]>, pos: Pos) -> R<Value> {
        match fv {
            Value::StructType(ty) => {
                let mut call_default = |d: &Rc<Closure>| {
                    let base = self.stack.len();
                    self.invoke(d, base, pos).map_err(into_error)
                };
                Ok(ops::construct(ty, args, names, pos, &mut call_default)?)
            }
            _ if names.is_some() => Err(ops::named_args_error(pos).into()),
            Value::Func(clo) => {
                let base = self.stack.len();
                self.stack.extend(args);
                self.invoke(clo, base, pos)
            }
            Value::Builtin(b) => {
                builtins::check_arity(b, args.len(), pos)?;
                let mut args = args;
                Ok((b.func)(self, &mut args, pos)?)
            }
            other => Err(ops::not_callable(other, pos).into()),
        }
    }
}
