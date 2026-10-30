//! 字节码虚拟机后端。ZYX 函数之间的调用不占用 Rust 栈，调用帧保存在 `frames` 中；
//! 只有内置函数回调 ZYX 函数（如 `map`）时才重入 `execute`。

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::{BinOp, Pos};
use crate::builtins::{self, Io, Runtime};
use crate::bytecode::{self, Chunk, Fallback, Op};
use crate::error::{ErrorKind, Result};
use crate::ir::{Pattern, Program, Slot, UpSrc, Var};
use crate::ops::{self, err, type_err};
use crate::value::{Cell, Closure, StructType, Value, values_equal};

/// 调用深度上限（reference.md 5.4）。
const MAX_DEPTH: usize = 10_000;

struct Frame {
    closure: Rc<Closure>,
    /// 指向 `Vm::chunks` 中的代码块。代码块在虚拟机创建时生成，之后既不修改也不释放，
    /// 用裸指针可以省去每次调用、返回时的引用计数操作。
    chunk: *const Chunk,
    ip: usize,
    base: usize,
    /// 返回时栈截断到的位置（去掉被调用者和参数）
    ret_to: usize,
    cells: Vec<Cell>,
}

pub struct Vm {
    stack: Vec<Value>,
    frames: Vec<Frame>,
    globals: Vec<Value>,
    chunks: Vec<Rc<Chunk>>,
    protos: Vec<Rc<crate::ir::FnProto>>,
    io: Io,
    placeholder: Cell,
    /// 内置函数的参数缓冲区，反复使用以免每次调用都分配
    arg_buf: Vec<Value>,
}

impl Runtime for Vm {
    fn call(&mut self, f: &Value, args: Vec<Value>, pos: Pos) -> Result<Value> {
        self.call_value(f, args, None, pos)
    }

    fn io(&mut self) -> &mut Io {
        &mut self.io
    }
}

fn new_cell(v: Value) -> Cell {
    Rc::new(RefCell::new(v))
}

/// 覆盖一个值。旧值不持有堆资源时直接写入，省去一次（未内联的）析构调用。
#[inline(always)]
fn overwrite(dst: &mut Value, v: Value) {
    if matches!(dst, Value::Nil | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Builtin(_) | Value::Undef) {
        // SAFETY: 旧值没有需要析构的内容，跳过析构不会泄漏
        unsafe { std::ptr::write(dst, v) }
    } else {
        *dst = v;
    }
}

/// 丢弃一个值，标量直接忽略。
#[inline(always)]
fn discard(v: Value) {
    if matches!(v, Value::Nil | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Builtin(_) | Value::Undef) {
        std::mem::forget(v);
    }
}

/// SAFETY: 见 `Frame::chunk`，指针在虚拟机存活期间始终有效。
fn chunk_ref<'a>(p: *const Chunk) -> &'a Chunk {
    unsafe { &*p }
}

/// 两个整数的二元运算快速路径。返回 `None` 时走通用路径（溢出、除零等错误也由通用路径报告）。
#[inline(always)]
fn int_fast(op: BinOp, x: i64, y: i64) -> Option<Value> {
    Some(match op {
        BinOp::Add => Value::Int(x.checked_add(y)?),
        BinOp::Sub => Value::Int(x.checked_sub(y)?),
        BinOp::Mul => Value::Int(x.checked_mul(y)?),
        BinOp::Div if y != 0 => {
            let q = x.checked_div(y)?;
            Value::Int(if x % y != 0 && ((x < 0) != (y < 0)) { q - 1 } else { q })
        }
        BinOp::Mod if y != 0 => {
            let r = x.checked_rem(y)?;
            Value::Int(if r != 0 && ((r < 0) != (y < 0)) { r + y } else { r })
        }
        BinOp::Lt => Value::Bool(x < y),
        BinOp::Le => Value::Bool(x <= y),
        BinOp::Gt => Value::Bool(x > y),
        BinOp::Ge => Value::Bool(x >= y),
        BinOp::Eq => Value::Bool(x == y),
        BinOp::Ne => Value::Bool(x != y),
        _ => return None,
    })
}

#[inline(always)]
fn bin(op: BinOp, a: &Value, b: &Value, pos: impl Fn() -> Pos) -> Result<Value> {
    if let (Value::Int(x), Value::Int(y)) = (a, b)
        && let Some(v) = int_fast(op, *x, *y)
    {
        return Ok(v);
    }
    ops::binary(op, a, b, pos())
}

/// 比较运算，结果一定是 bool。
#[inline(always)]
fn cmp(op: BinOp, a: &Value, b: &Value, pos: impl Fn() -> Pos) -> Result<bool> {
    if let (Value::Int(x), Value::Int(y)) = (a, b) {
        return Ok(match op {
            BinOp::Lt => x < y,
            BinOp::Le => x <= y,
            BinOp::Gt => x > y,
            BinOp::Ge => x >= y,
            BinOp::Eq => x == y,
            _ => x != y,
        });
    }
    Ok(matches!(ops::binary(op, a, b, pos())?, Value::Bool(true)))
}

/// 借助内联缓存找到字段下标：同一条指令连续遇到同一种结构体时不再按名字查找。
#[inline(always)]
fn cached_field(inst: &crate::value::Instance, name: &str, cache: &std::cell::Cell<(usize, u32)>) -> Option<usize> {
    let ty = Rc::as_ptr(&inst.ty) as usize;
    let (cached_ty, idx) = cache.get();
    if cached_ty == ty {
        return Some(idx as usize);
    }
    let idx = inst.ty.field_index(name)?;
    cache.set((ty, idx as u32));
    Some(idx)
}

fn uninit(name: &str, pos: Pos) -> crate::error::ZyxError {
    err(ErrorKind::Name, pos, format!("variable '{name}' is used before it is initialized"))
}

impl Vm {
    pub fn new(program: &Program) -> Self {
        Vm {
            stack: Vec::with_capacity(1024),
            frames: Vec::with_capacity(64),
            globals: vec![Value::Undef; program.n_globals],
            chunks: bytecode::compile(program),
            protos: program.protos.clone(),
            io: Io::default(),
            placeholder: new_cell(Value::Undef),
            arg_buf: Vec::with_capacity(8),
        }
    }

    pub fn flush(&mut self) {
        self.io.flush();
    }

    pub fn run(&mut self, program: &Program) -> Result<()> {
        let main = Rc::new(Closure { proto: program.main.clone(), upvals: Vec::new() });
        let r = self.push_frame(main, 0, 0, Pos { line: 1, col: 1 }).and_then(|_| self.execute(0));
        self.io.flush();
        r.map(|_| ())
    }

    /// 实参已在 `stack[base..]`，建立新帧。
    fn push_frame(&mut self, clo: Rc<Closure>, base: usize, ret_to: usize, pos: Pos) -> Result<()> {
        let p = &clo.proto;
        let argc = self.stack.len() - base;
        if argc != p.arity {
            return Err(ops::arity_error(p, argc, pos));
        }
        if self.frames.len() >= MAX_DEPTH {
            return Err(ops::stack_overflow(pos));
        }
        let chunk: *const Chunk = &*self.chunks[p.id];
        self.stack.resize(base + chunk_ref(chunk).n_slots, Value::Nil);
        let mut cells = if p.n_cells > 0 { vec![self.placeholder.clone(); p.n_cells] } else { Vec::new() };
        for &(i, c) in &p.param_cells {
            cells[c as usize] = new_cell(self.stack[base + i].clone());
        }
        self.frames.push(Frame { closure: clo, chunk, ip: 0, base, ret_to, cells });
        Ok(())
    }

    /// 从内置函数或结构体构造中调用一个函数值，运行到它返回为止。
    fn call_value(&mut self, f: &Value, args: Vec<Value>, names: Option<&[Rc<str>]>, pos: Pos) -> Result<Value> {
        match f {
            Value::StructType(ty) => {
                let mut call_default = |d: &Rc<Closure>| self.call_closure(d, Vec::new(), pos);
                ops::construct(ty, args, names, pos, &mut call_default)
            }
            _ if names.is_some() => Err(ops::named_args_error(pos)),
            Value::Func(clo) => self.call_closure(clo, args, pos),
            Value::Builtin(b) => {
                builtins::check_arity(b, args.len(), pos)?;
                let mut args = args;
                (b.func)(self, &mut args, pos)
            }
            other => Err(ops::not_callable(other, pos)),
        }
    }

    /// 调用内置函数，参数是 `stack[start..]`（调用后从栈上移除）。
    fn call_builtin(&mut self, b: &'static builtins::Builtin, start: usize, pos: Pos) -> Result<Value> {
        let mut args = std::mem::take(&mut self.arg_buf);
        args.extend(self.stack.drain(start..));
        let r = builtins::check_arity(b, args.len(), pos).and_then(|_| (b.func)(self, &mut args, pos));
        args.clear();
        self.arg_buf = args;
        r
    }

    fn call_closure(&mut self, clo: &Rc<Closure>, args: Vec<Value>, pos: Pos) -> Result<Value> {
        let base = self.stack.len();
        self.stack.extend(args);
        self.push_frame(clo.clone(), base, base, pos)?;
        let stop = self.frames.len() - 1;
        self.execute(stop)
    }

    fn make_closure(&self, idx: u32) -> Rc<Closure> {
        let f = self.frames.last().expect("frame");
        let proto = self.protos[idx as usize].clone();
        let upvals = proto
            .upvals
            .iter()
            .map(|src| match *src {
                UpSrc::Cell(c) => f.cells[c as usize].clone(),
                UpSrc::Upval(u) => f.closure.upvals[u as usize].clone(),
            })
            .collect();
        Rc::new(Closure { proto, upvals })
    }

    fn read_var(&self, var: Var, name: &str, base: usize, pos: Pos) -> Result<Value> {
        let f = self.frames.last().expect("frame");
        let v = match var {
            Var::Local(s) => return Ok(self.stack[base + s as usize].clone()),
            Var::Global(g) => self.globals[g as usize].clone(),
            Var::Cell(c) => f.cells[c as usize].borrow().clone(),
            Var::Upval(u) => f.closure.upvals[u as usize].borrow().clone(),
        };
        if let Value::Undef = v {
            return Err(uninit(name, pos));
        }
        Ok(v)
    }

    fn store_slot(&mut self, slot: Slot, v: Value, base: usize) {
        match slot {
            Slot::Local(s) => self.stack[base + s as usize] = v,
            Slot::Global(g) => self.globals[g as usize] = v,
            Slot::Cell(c) => *self.frames.last().expect("frame").cells[c as usize].borrow_mut() = v,
        }
    }

    fn match_pattern(&mut self, p: &Pattern, v: &Value, base: usize) -> bool {
        match p {
            Pattern::Wildcard => true,
            Pattern::Bind(slot) => {
                self.store_slot(*slot, v.clone(), base);
                true
            }
            Pattern::Lit(lit) => values_equal(v, lit),
            Pattern::Range { lo, hi, inclusive } => match v {
                Value::Int(n) => {
                    lo.is_none_or(|l| *n >= l) && hi.is_none_or(|h| if *inclusive { *n <= h } else { *n < h })
                }
                _ => false,
            },
            Pattern::Or(ps) => ps.iter().any(|p| self.match_pattern(p, v, base)),
        }
    }

    fn pop(&mut self) -> Value {
        self.stack.pop().expect("stack underflow")
    }

    /// 读帧内槽位，不做边界检查。
    /// SAFETY: 槽位编号由编译器生成且小于 `n_slots`，进入帧时栈已扩展到 `base + n_slots`，
    /// 帧存活期间栈不会缩到这之下。调试构建中用断言核对。
    #[inline(always)]
    fn slot(&self, i: usize) -> &Value {
        debug_assert!(i < self.stack.len());
        unsafe { self.stack.get_unchecked(i) }
    }

    #[inline(always)]
    fn slot_mut(&mut self, i: usize) -> &mut Value {
        debug_assert!(i < self.stack.len());
        unsafe { self.stack.get_unchecked_mut(i) }
    }

    /// 主循环：执行栈顶帧，直到帧数回落到 `stop` 时返回该帧的返回值。
    fn execute(&mut self, stop: usize) -> Result<Value> {
        let (mut chunk, mut ip, mut base) = {
            let f = self.frames.last().expect("frame");
            (chunk_ref(f.chunk), f.ip, f.base)
        };

        macro_rules! pos {
            () => {
                chunk.pos[ip - 1]
            };
        }
        macro_rules! reload {
            () => {{
                let f = self.frames.last().expect("frame");
                chunk = chunk_ref(f.chunk);
                ip = f.ip;
                base = f.base;
            }};
        }
        // SAFETY: 常量下标由编译器生成，必在常量表内。
        macro_rules! konst {
            ($c:expr) => {{
                debug_assert!(($c as usize) < chunk.consts.len());
                unsafe { chunk.consts.get_unchecked($c as usize) }
            }};
        }
        macro_rules! save_ip {
            () => {
                self.frames.last_mut().expect("frame").ip = ip
            };
        }
        // 整数快速路径，其余交给 ops::binary
        macro_rules! arith {
            ($op:expr, $checked:ident) => {{
                let b = self.pop();
                let a = self.stack.last_mut().expect("operand");
                match (&*a, &b) {
                    (Value::Int(x), Value::Int(y)) => match x.$checked(*y) {
                        Some(r) => *a = Value::Int(r),
                        None => return Err(err(ErrorKind::Runtime, pos!(), "integer overflow")),
                    },
                    _ => *a = ops::binary($op, a, &b, pos!())?,
                }
            }};
        }
        macro_rules! compare {
            ($op:expr, $cmp:tt) => {{
                let b = self.pop();
                let a = self.stack.last_mut().expect("operand");
                match (&*a, &b) {
                    (Value::Int(x), Value::Int(y)) => *a = Value::Bool(*x $cmp *y),
                    _ => *a = ops::binary($op, a, &b, pos!())?,
                }
            }};
        }

        loop {
            // SAFETY: 每个代码块都以 Return 结尾，跳转目标都由编译器回填为块内位置，ip 不会越界。
            debug_assert!(ip < chunk.code.len());
            let op = unsafe { *chunk.code.get_unchecked(ip) };
            ip += 1;
            match op {
                Op::Const(i) => self.stack.push(konst!(i).clone()),
                Op::Nil => self.stack.push(Value::Nil),
                Op::True => self.stack.push(Value::Bool(true)),
                Op::False => self.stack.push(Value::Bool(false)),
                Op::Pop => {
                    self.stack.pop();
                }
                Op::PopN(n) => {
                    let len = self.stack.len();
                    self.stack.truncate(len - n as usize);
                }
                Op::Dup => {
                    let v = self.stack.last().expect("operand").clone();
                    self.stack.push(v);
                }
                Op::Dup2 => {
                    let len = self.stack.len();
                    let (a, b) = (self.stack[len - 2].clone(), self.stack[len - 1].clone());
                    self.stack.push(a);
                    self.stack.push(b);
                }
                Op::GetLocal(s) => {
                    let v = self.slot(base + s as usize).clone();
                    self.stack.push(v);
                }
                Op::SetLocal(s) => {
                    let v = self.pop();
                    overwrite(self.slot_mut(base + s as usize), v);
                }
                Op::GetCell(c, n) => {
                    let v = self.frames.last().expect("frame").cells[c as usize].borrow().clone();
                    if let Value::Undef = v {
                        return Err(uninit(&chunk.names[n as usize], pos!()));
                    }
                    self.stack.push(v);
                }
                Op::SetCell(c) => {
                    let v = self.pop();
                    *self.frames.last().expect("frame").cells[c as usize].borrow_mut() = v;
                }
                Op::GetUpval(u, n) => {
                    let v = self.frames.last().expect("frame").closure.upvals[u as usize].borrow().clone();
                    if let Value::Undef = v {
                        return Err(uninit(&chunk.names[n as usize], pos!()));
                    }
                    self.stack.push(v);
                }
                Op::SetUpval(u) => {
                    let v = self.pop();
                    *self.frames.last().expect("frame").closure.upvals[u as usize].borrow_mut() = v;
                }
                Op::GetGlobal(g, n) => {
                    let v = &self.globals[g as usize];
                    if let Value::Undef = v {
                        return Err(uninit(&chunk.names[n as usize], pos!()));
                    }
                    let v = v.clone();
                    self.stack.push(v);
                }
                Op::SetGlobal(g) => {
                    let v = self.pop();
                    self.globals[g as usize] = v;
                }
                Op::NewCell(c) => {
                    self.frames.last_mut().expect("frame").cells[c as usize] = new_cell(Value::Undef);
                }
                Op::Add => arith!(BinOp::Add, checked_add),
                Op::Sub => arith!(BinOp::Sub, checked_sub),
                Op::Mul => arith!(BinOp::Mul, checked_mul),
                Op::Div => {
                    let b = self.pop();
                    let a = self.stack.last_mut().expect("operand");
                    *a = match (&*a, &b) {
                        (Value::Int(x), Value::Int(y)) => Value::Int(ops::floor_div(*x, *y, pos!())?),
                        _ => ops::binary(BinOp::Div, a, &b, pos!())?,
                    };
                }
                Op::Mod => {
                    let b = self.pop();
                    let a = self.stack.last_mut().expect("operand");
                    *a = match (&*a, &b) {
                        (Value::Int(x), Value::Int(y)) => Value::Int(ops::floor_mod(*x, *y, pos!())?),
                        _ => ops::binary(BinOp::Mod, a, &b, pos!())?,
                    };
                }
                Op::Pow => {
                    let b = self.pop();
                    let a = self.stack.last_mut().expect("operand");
                    *a = ops::binary(BinOp::Pow, a, &b, pos!())?;
                }
                Op::Eq => {
                    let b = self.pop();
                    let a = self.stack.last_mut().expect("operand");
                    *a = Value::Bool(values_equal(a, &b));
                }
                Op::Ne => {
                    let b = self.pop();
                    let a = self.stack.last_mut().expect("operand");
                    *a = Value::Bool(!values_equal(a, &b));
                }
                Op::Lt => compare!(BinOp::Lt, <),
                Op::Le => compare!(BinOp::Le, <=),
                Op::Gt => compare!(BinOp::Gt, >),
                Op::Ge => compare!(BinOp::Ge, >=),
                Op::Neg => {
                    let a = self.stack.last_mut().expect("operand");
                    *a = ops::unary(crate::ast::UnOp::Neg, a, pos!())?;
                }
                Op::Not => {
                    let a = self.stack.last_mut().expect("operand");
                    *a = ops::unary(crate::ast::UnOp::Not, a, pos!())?;
                }
                Op::CheckBool(ctx) => {
                    let v = self.stack.last().expect("operand");
                    ops::expect_bool(v, pos!(), ctx.what())?;
                }
                Op::JumpIfFalse(target, ctx) => match self.pop() {
                    Value::Bool(true) => {}
                    Value::Bool(false) => ip = target as usize,
                    other => return Err(type_err(pos!(), format!("{} must be bool, got {}", ctx.what(), other.type_name()))),
                },
                Op::Jump(target) => ip = target as usize,
                Op::Range(inclusive) => {
                    let b = self.pop();
                    let a = self.stack.last_mut().expect("operand");
                    *a = ops::make_range(a, &b, inclusive, pos!())?;
                }
                Op::Array(n) => {
                    let items = self.stack.split_off(self.stack.len() - n as usize);
                    self.stack.push(Value::array(items));
                }
                Op::Interp(n) => {
                    let start = self.stack.len() - n as usize;
                    let mut s = String::new();
                    for v in &self.stack[start..] {
                        v.write_display(&mut s);
                    }
                    self.stack.truncate(start);
                    self.stack.push(Value::string(s));
                }
                Op::Index => {
                    let i = self.pop();
                    let c = self.stack.last_mut().expect("operand");
                    *c = ops::index_get(c, &i, pos!())?;
                }
                Op::SetIndex => {
                    let v = self.pop();
                    let i = self.pop();
                    let c = self.pop();
                    ops::index_set(&c, &i, v, pos!())?;
                }
                Op::Field(n, k) => {
                    let o = self.stack.last_mut().expect("operand");
                    let name = &chunk.names[n as usize];
                    let fast = match &*o {
                        Value::Struct(inst) => cached_field(inst, name, &chunk.field_caches[k as usize]).map(|i| inst.fields.borrow()[i].clone()),
                        _ => None,
                    };
                    *o = match fast {
                        Some(v) => v,
                        None => ops::field_get(o, name, pos!())?,
                    };
                }
                Op::SetField(n, k) => {
                    let v = self.pop();
                    let o = self.pop();
                    let name = &chunk.names[n as usize];
                    match &o {
                        Value::Struct(inst) if let Some(i) = cached_field(inst, name, &chunk.field_caches[k as usize]) => {
                            inst.fields.borrow_mut()[i] = v;
                        }
                        _ => ops::field_set(&o, name, v, pos!())?,
                    }
                }
                Op::Call(argc) => {
                    let callee_at = self.stack.len() - argc as usize - 1;
                    if let Value::Func(_) = &self.stack[callee_at] {
                        // 被调用者所在的槽位在返回时会被丢弃，直接移出，省一次引用计数增减
                        let Value::Func(clo) = std::mem::replace(&mut self.stack[callee_at], Value::Nil) else {
                            unreachable!()
                        };
                        save_ip!();
                        self.push_frame(clo, callee_at + 1, callee_at, pos!())?;
                        reload!();
                    } else if let Value::Builtin(b) = self.stack[callee_at] {
                        save_ip!();
                        let r = self.call_builtin(b, callee_at + 1, pos!())?;
                        *self.stack.last_mut().expect("callee") = r;
                    } else {
                        let args = self.stack.split_off(callee_at + 1);
                        let callee = self.pop();
                        save_ip!();
                        let r = self.call_value(&callee, args, None, pos!())?;
                        self.stack.push(r);
                    }
                }
                Op::CallNamed(argc, names) => {
                    let args = self.stack.split_off(self.stack.len() - argc as usize);
                    let callee = self.pop();
                    save_ip!();
                    let r = self.call_value(&callee, args, Some(&chunk.name_lists[names as usize]), pos!())?;
                    self.stack.push(r);
                }
                Op::Method(d) => {
                    let desc = &chunk.methods[d as usize];
                    let obj_at = self.stack.len() - desc.argc as usize - 1;
                    let callee = match ops::find_method(&self.stack[obj_at], &desc.name) {
                        Some(m) => Value::Func(m),
                        None => match &desc.fallback {
                            Fallback::Var(var, n) => self.read_var(*var, &chunk.names[*n as usize], base, pos!())?,
                            Fallback::Const(v) => v.clone(),
                            Fallback::None => return Err(ops::no_method_error(&self.stack[obj_at], &desc.name, pos!())),
                        },
                    };
                    save_ip!();
                    if let Value::Func(clo) = callee {
                        self.push_frame(clo, obj_at, obj_at, pos!())?;
                        reload!();
                    } else if let Value::Builtin(b) = callee {
                        let r = self.call_builtin(b, obj_at, pos!())?;
                        self.stack.push(r);
                    } else {
                        let args = self.stack.split_off(obj_at);
                        let r = self.call_value(&callee, args, None, pos!())?;
                        self.stack.push(r);
                    }
                }
                Op::Closure(idx) => {
                    let c = self.make_closure(idx);
                    self.stack.push(Value::Func(c));
                }
                Op::Struct(idx) => {
                    let proto = &chunk.structs[idx as usize];
                    let defaults = proto.defaults.iter().map(|d| d.map(|i| self.make_closure(i))).collect();
                    let ty = StructType {
                        name: proto.name.clone(),
                        fields: proto.fields.clone(),
                        defaults,
                        methods: RefCell::new(Vec::new()),
                    };
                    self.stack.push(Value::StructType(Rc::new(ty)));
                }
                Op::Impl(idx) => {
                    let target = self.pop();
                    let desc = &chunk.impls[idx as usize];
                    let methods = desc.methods.iter().map(|(n, i)| (n.clone(), self.make_closure(*i))).collect();
                    ops::add_methods(&target, &desc.name, methods, pos!())?;
                }
                Op::IterPrep(h) => {
                    let it = match self.pop() {
                        v @ (Value::Range(..) | Value::Array(_)) => v,
                        Value::Str(s) => Value::array(s.chars().map(|c| Value::str(c.encode_utf8(&mut [0; 4]))).collect()),
                        other => return Err(type_err(pos!(), format!("cannot iterate over {}", other.type_name()))),
                    };
                    self.stack[base + h as usize] = it;
                    self.stack[base + h as usize + 1] = Value::Int(0);
                }
                Op::ForNext(h, exit, with_index) => {
                    let h = base + h as usize;
                    let Value::Int(i) = self.stack[h + 1] else { unreachable!("loop counter") };
                    let item = match &self.stack[h] {
                        Value::Range(r) => (r.0 + i < r.1).then(|| Value::Int(r.0 + i)),
                        Value::Array(a) => a.borrow().get(i as usize).cloned(),
                        _ => unreachable!("iterable checked by IterPrep"),
                    };
                    match item {
                        None => ip = exit as usize,
                        Some(v) => {
                            self.stack[h + 1] = Value::Int(i + 1);
                            if with_index {
                                self.stack.push(Value::Int(i));
                            }
                            self.stack.push(v);
                        }
                    }
                }
                Op::Match(p, h, fail) => {
                    let v = self.stack[base + h as usize].clone();
                    if !self.match_pattern(&chunk.patterns[p as usize], &v, base) {
                        ip = fail as usize;
                    }
                }
                Op::NoMatch(h) => {
                    let v = &self.stack[base + h as usize];
                    return Err(err(ErrorKind::Runtime, pos!(), format!("no match arm for value {}", v.repr())));
                }
                Op::BinLocal(op, s) => {
                    let top = self.stack.len() - 1;
                    let r = bin(op, self.slot(top), self.slot(base + s as usize), || pos!())?;
                    overwrite(self.slot_mut(top), r);
                }
                Op::BinConst(op, c) => {
                    let top = self.stack.len() - 1;
                    let r = bin(op, self.slot(top), konst!(c), || pos!())?;
                    overwrite(self.slot_mut(top), r);
                }
                Op::OpAssignLocal(op, s) => {
                    let rhs = self.pop();
                    let slot = base + s as usize;
                    // 字符串追加：局部变量独占该字符串时原地追加，避免 `s += x` 在循环中退化为 O(n²)
                    if let (BinOp::Add, Value::Str(r)) = (op, &rhs)
                        && let Value::Str(l) = self.slot_mut(slot)
                        && let Some(buf) = Rc::get_mut(l)
                    {
                        buf.push_str(r);
                        continue;
                    }
                    let r = bin(op, self.slot(slot), &rhs, || pos!())?;
                    overwrite(self.slot_mut(slot), r);
                    discard(rhs);
                }
                Op::OpAssignLocalConst(op, s, c) => {
                    let slot = base + s as usize;
                    if let (BinOp::Add, Value::Str(r)) = (op, konst!(c))
                        && let Value::Str(l) = self.slot_mut(slot)
                        && let Some(buf) = Rc::get_mut(l)
                    {
                        buf.push_str(r);
                        continue;
                    }
                    let r = bin(op, self.slot(slot), konst!(c), || pos!())?;
                    overwrite(self.slot_mut(slot), r);
                }
                Op::JumpIfNot(op, target) => {
                    let b = self.pop();
                    let a = self.pop();
                    if !cmp(op, &a, &b, || pos!())? {
                        ip = target as usize;
                    }
                }
                Op::JumpIfNotLocal(op, s, target) => {
                    let a = self.pop();
                    if !cmp(op, &a, self.slot(base + s as usize), || pos!())? {
                        ip = target as usize;
                    }
                    discard(a);
                }
                Op::JumpIfNotConst(op, c, target) => {
                    let a = self.pop();
                    if !cmp(op, &a, konst!(c), || pos!())? {
                        ip = target as usize;
                    }
                    discard(a);
                }
                Op::JumpIfNotLL(op, a, b, target) => {
                    if !cmp(op, self.slot(base + a as usize), self.slot(base + b as usize), || pos!())? {
                        ip = target as usize;
                    }
                }
                Op::JumpIfNotLC(op, a, c, target) => {
                    if !cmp(op, self.slot(base + a as usize), konst!(c), || pos!())? {
                        ip = target as usize;
                    }
                }
                Op::IndexLocal(s) => {
                    let top = self.stack.len() - 1;
                    // 快速路径：数组 + 合法的非负下标
                    let fast = match (&self.stack[base + s as usize], &self.stack[top]) {
                        (Value::Array(a), Value::Int(i)) if *i >= 0 => a.borrow().get(*i as usize).cloned(),
                        _ => None,
                    };
                    let v = match fast {
                        Some(v) => v,
                        None => ops::index_get(&self.stack[base + s as usize], &self.stack[top], pos!())?,
                    };
                    self.stack[top] = v;
                }
                Op::SetIndexLocal(s) => {
                    let v = self.pop();
                    let i = self.pop();
                    let c = &self.stack[base + s as usize];
                    // 快速路径：数组 + 合法的非负下标
                    if let (Value::Array(a), Value::Int(k)) = (c, &i)
                        && *k >= 0
                        && let Some(slot) = a.borrow_mut().get_mut(*k as usize)
                    {
                        *slot = v;
                        continue;
                    }
                    ops::index_set(c, &i, v, pos!())?;
                }
                Op::FieldLocal(s, n, k) => {
                    let obj = &self.stack[base + s as usize];
                    let name = &chunk.names[n as usize];
                    let v = match obj {
                        Value::Struct(inst) if let Some(i) = cached_field(inst, name, &chunk.field_caches[k as usize]) => {
                            inst.fields.borrow()[i].clone()
                        }
                        _ => ops::field_get(obj, name, pos!())?,
                    };
                    self.stack.push(v);
                }
                Op::SetFieldLocal(s, n, k) => {
                    let v = self.pop();
                    let obj = &self.stack[base + s as usize];
                    let name = &chunk.names[n as usize];
                    match obj {
                        Value::Struct(inst) if let Some(i) = cached_field(inst, name, &chunk.field_caches[k as usize]) => {
                            inst.fields.borrow_mut()[i] = v;
                        }
                        _ => ops::field_set(obj, name, v, pos!())?,
                    }
                }
                Op::Return => {
                    let v = self.pop();
                    let f = self.frames.pop().expect("frame");
                    self.stack.truncate(f.ret_to);
                    if self.frames.len() == stop {
                        return Ok(v);
                    }
                    self.stack.push(v);
                    reload!();
                }
            }
        }
    }
}
