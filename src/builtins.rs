//! 内置函数（reference.md 第 8 节）。通过 `Runtime` 接口与执行后端交互，两个后端共用。

// 所有内置函数共用同一个函数指针类型，参数统一为 `&mut Vec<Value>`（部分函数会从中取走值）。
#![allow(clippy::ptr_arg)]

use std::cell::RefCell;
use std::io::{BufRead, BufWriter, Stdout, Write};
use std::rc::Rc;

use crate::ast::Pos;
use crate::error::{ErrorKind, Result};
use crate::ops::{self, err, norm_index, type_err};
use crate::value::{Instance, Value, compare, values_equal};

/// 执行后端需要为内置函数提供的能力。
pub trait Runtime {
    /// 调用一个 ZYX 函数值（闭包、内置函数或结构体构造器）。
    fn call(&mut self, f: &Value, args: Vec<Value>, pos: Pos) -> Result<Value>;
    fn io(&mut self) -> &mut Io;
}

pub struct Io {
    pub out: BufWriter<Stdout>,
    rng: u64,
}

impl Default for Io {
    fn default() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x2545_f491_4f6c_dd1d);
        Io { out: BufWriter::new(std::io::stdout()), rng: seed }
    }
}

impl Io {
    pub fn flush(&mut self) {
        let _ = self.out.flush();
    }

    /// splitmix64
    fn next_u64(&mut self) -> u64 {
        self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// 参数通过可复用的缓冲区传入，内置函数可以取走其中的值（调用方随后会清空它）。
type BuiltinFn = fn(&mut dyn Runtime, &mut Vec<Value>, Pos) -> Result<Value>;

pub struct Builtin {
    pub name: &'static str,
    pub min: usize,
    pub max: usize,
    pub func: BuiltinFn,
}

const ANY: usize = usize::MAX;

macro_rules! builtins {
    ($($name:literal, $min:expr, $max:expr, $f:ident;)*) => {
        pub static BUILTINS: &[Builtin] = &[$(Builtin { name: $name, min: $min, max: $max, func: $f },)*];
    };
}

builtins! {
    "print", 0, ANY, print;
    "write", 0, ANY, write;
    "input", 0, 1, input;
    "type", 1, 1, type_of;
    "int", 1, 1, to_int;
    "float", 1, 1, to_float;
    "str", 1, 1, to_str;
    "parse_int", 1, 1, parse_int;
    "len", 1, 1, len;
    "array", 2, 2, make_array;
    "copy", 1, 1, copy;
    "push", 2, 2, push;
    "pop", 1, 1, pop;
    "insert", 3, 3, insert;
    "remove", 2, 2, remove;
    "contains", 2, 2, contains;
    "index_of", 2, 2, index_of;
    "reverse", 1, 1, reverse;
    "sort", 1, 2, sort;
    "map", 2, 2, map;
    "filter", 2, 2, filter;
    "reduce", 3, 3, reduce;
    "sum", 1, 1, sum;
    "min", 1, ANY, min;
    "max", 1, ANY, max;
    "join", 2, 2, join;
    "split", 2, 2, split;
    "trim", 1, 1, trim;
    "upper", 1, 1, upper;
    "lower", 1, 1, lower;
    "replace", 3, 3, replace;
    "starts_with", 2, 2, starts_with;
    "ends_with", 2, 2, ends_with;
    "pad_left", 2, 3, pad_left;
    "pad_right", 2, 3, pad_right;
    "ord", 1, 1, ord;
    "chr", 1, 1, chr;
    "abs", 1, 1, abs;
    "sqrt", 1, 1, sqrt;
    "floor", 1, 1, floor;
    "ceil", 1, 1, ceil;
    "round", 1, 1, round;
    "random", 2, 2, random;
    "seed", 1, 1, seed;
    "time", 0, 0, time;
    "assert", 1, 2, assert;
    "assert_eq", 2, 2, assert_eq;
    "error", 1, 1, error;
    "exit", 0, 1, exit;
}

pub fn lookup(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name == name)
}

pub fn check_arity(b: &Builtin, n: usize, pos: Pos) -> Result<()> {
    if n >= b.min && n <= b.max {
        return Ok(());
    }
    let plural = |k: usize| if k == 1 { "argument" } else { "arguments" };
    let expected = if b.min == b.max {
        format!("{} {}", b.min, plural(b.min))
    } else if b.max == ANY {
        format!("at least {} {}", b.min, plural(b.min))
    } else {
        format!("{} to {} arguments", b.min, b.max)
    };
    Err(type_err(pos, format!("{}() expects {expected}, got {n}", b.name)))
}

// ---------- 参数辅助 ----------

fn int_arg(v: &Value, f: &str, pos: Pos) -> Result<i64> {
    match v {
        Value::Int(n) => Ok(*n),
        other => Err(type_err(pos, format!("{f}() expects int, got {}", other.type_name()))),
    }
}

fn str_arg<'a>(v: &'a Value, f: &str, pos: Pos) -> Result<&'a Rc<String>> {
    match v {
        Value::Str(s) => Ok(s),
        other => Err(type_err(pos, format!("{f}() expects str, got {}", other.type_name()))),
    }
}

fn array_arg<'a>(v: &'a Value, f: &str, pos: Pos) -> Result<&'a Rc<RefCell<Vec<Value>>>> {
    match v {
        Value::Array(a) => Ok(a),
        other => Err(type_err(pos, format!("{f}() expects array, got {}", other.type_name()))),
    }
}

fn num_arg(v: &Value, f: &str, pos: Pos) -> Result<f64> {
    match v {
        Value::Int(n) => Ok(*n as f64),
        Value::Float(x) => Ok(*x),
        other => Err(type_err(pos, format!("{f}() expects a number, got {}", other.type_name()))),
    }
}

/// 数组、区间、字符串展开为元素列表（快照，回调中修改原数组不影响遍历）。
fn items(v: &Value, f: &str, pos: Pos) -> Result<Vec<Value>> {
    match v {
        Value::Array(a) => Ok(a.borrow().clone()),
        Value::Range(r) => Ok((r.0..r.1).map(Value::Int).collect()),
        Value::Str(s) => Ok(s.chars().map(|c| Value::str(c.encode_utf8(&mut [0; 4]))).collect()),
        other => Err(type_err(pos, format!("{f}() expects array, range or str, got {}", other.type_name()))),
    }
}

fn float_to_int(x: f64, f: &str, pos: Pos) -> Result<i64> {
    if x.is_finite() && x >= i64::MIN as f64 && x < i64::MAX as f64 {
        Ok(x as i64)
    } else {
        Err(err(ErrorKind::Value, pos, format!("{f}() cannot convert {} to int", crate::value::format_float(x))))
    }
}

// ---------- 输入输出 ----------

fn joined(args: &[Value]) -> String {
    let mut s = String::new();
    for (i, v) in args.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        v.write_display(&mut s);
    }
    s
}

fn print(rt: &mut dyn Runtime, args: &mut Vec<Value>, _: Pos) -> Result<Value> {
    let mut s = joined(args);
    s.push('\n');
    let _ = rt.io().out.write_all(s.as_bytes());
    Ok(Value::Nil)
}

fn write(rt: &mut dyn Runtime, args: &mut Vec<Value>, _: Pos) -> Result<Value> {
    let _ = rt.io().out.write_all(joined(args).as_bytes());
    Ok(Value::Nil)
}

fn input(rt: &mut dyn Runtime, args: &mut Vec<Value>, _: Pos) -> Result<Value> {
    let io = rt.io();
    if let Some(p) = args.first() {
        let _ = io.out.write_all(p.to_display().as_bytes());
    }
    io.flush();
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => Ok(Value::Nil),
        Ok(_) => {
            let trimmed = line.strip_suffix('\n').unwrap_or(&line);
            Ok(Value::str(trimmed.strip_suffix('\r').unwrap_or(trimmed)))
        }
    }
}

// ---------- 类型与转换 ----------

fn type_of(_: &mut dyn Runtime, args: &mut Vec<Value>, _: Pos) -> Result<Value> {
    Ok(Value::str(&args[0].type_name()))
}

fn parse_int_str(s: &str) -> Option<i64> {
    s.trim().parse::<i64>().ok()
}

fn to_int(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    Ok(Value::Int(match &args[0] {
        Value::Int(n) => *n,
        Value::Float(x) => float_to_int(*x, "int", pos)?,
        Value::Bool(b) => *b as i64,
        Value::Str(s) => parse_int_str(s)
            .ok_or_else(|| err(ErrorKind::Value, pos, format!("invalid integer literal: {}", Value::Str(s.clone()).repr())))?,
        other => return Err(type_err(pos, format!("int() cannot convert {}", other.type_name()))),
    }))
}

fn to_float(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    Ok(Value::Float(match &args[0] {
        Value::Int(n) => *n as f64,
        Value::Float(x) => *x,
        Value::Bool(b) => *b as i64 as f64,
        Value::Str(s) => s
            .trim()
            .parse::<f64>()
            .map_err(|_| err(ErrorKind::Value, pos, format!("invalid float literal: {}", Value::Str(s.clone()).repr())))?,
        other => return Err(type_err(pos, format!("float() cannot convert {}", other.type_name()))),
    }))
}

fn to_str(_: &mut dyn Runtime, args: &mut Vec<Value>, _: Pos) -> Result<Value> {
    Ok(match &args[0] {
        Value::Str(s) => Value::Str(s.clone()),
        other => Value::str(&other.to_display()),
    })
}

fn parse_int(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let s = str_arg(&args[0], "parse_int", pos)?;
    Ok(parse_int_str(s).map(Value::Int).unwrap_or(Value::Nil))
}

// ---------- 数组与通用 ----------

fn len(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let n = match &args[0] {
        Value::Array(a) => a.borrow().len() as i64,
        Value::Str(s) => {
            if s.is_ascii() {
                s.len() as i64
            } else {
                s.chars().count() as i64
            }
        }
        Value::Range(r) => (r.1 - r.0).max(0),
        other => return Err(type_err(pos, format!("len() of {}", other.type_name()))),
    };
    Ok(Value::Int(n))
}

fn shallow_copy(v: &Value) -> Value {
    match v {
        Value::Array(a) => Value::array(a.borrow().clone()),
        Value::Struct(inst) => Value::Struct(Rc::new(Instance { ty: inst.ty.clone(), fields: RefCell::new(inst.fields.borrow().clone()) })),
        other => other.clone(),
    }
}

fn make_array(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let n = int_arg(&args[0], "array", pos)?;
    if n < 0 {
        return Err(err(ErrorKind::Value, pos, "array() length must not be negative"));
    }
    let items = (0..n).map(|_| shallow_copy(&args[1])).collect();
    Ok(Value::array(items))
}

fn copy(_: &mut dyn Runtime, args: &mut Vec<Value>, _: Pos) -> Result<Value> {
    Ok(shallow_copy(&args[0]))
}

fn push(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let x = args.pop().expect("arity");
    array_arg(&args[0], "push", pos)?.borrow_mut().push(x);
    Ok(Value::Nil)
}

fn pop(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    array_arg(&args[0], "pop", pos)?
        .borrow_mut()
        .pop()
        .ok_or_else(|| err(ErrorKind::Index, pos, "pop from empty array"))
}

fn insert(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let x = args.pop().expect("arity");
    let i = int_arg(&args[1], "insert", pos)?;
    let mut a = array_arg(&args[0], "insert", pos)?.borrow_mut();
    let len = a.len();
    let k = if i < 0 { i + len as i64 } else { i };
    if k < 0 || k as usize > len {
        return Err(err(ErrorKind::Index, pos, format!("insert index {i} out of range for array of length {len}")));
    }
    a.insert(k as usize, x);
    Ok(Value::Nil)
}

fn remove(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let i = int_arg(&args[1], "remove", pos)?;
    let mut a = array_arg(&args[0], "remove", pos)?.borrow_mut();
    let len = a.len();
    match norm_index(i, len) {
        Some(k) => Ok(a.remove(k)),
        None => Err(err(ErrorKind::Index, pos, format!("index {i} out of range for array of length {len}"))),
    }
}

fn char_index(hay: &str, byte_idx: usize) -> i64 {
    hay[..byte_idx].chars().count() as i64
}

fn contains(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    Ok(Value::Bool(match &args[0] {
        Value::Array(a) => a.borrow().iter().any(|v| values_equal(v, &args[1])),
        Value::Str(s) => s.contains(&**str_arg(&args[1], "contains", pos)?),
        Value::Range(r) => matches!(args[1], Value::Int(n) if n >= r.0 && n < r.1),
        other => return Err(type_err(pos, format!("contains() expects array, str or range, got {}", other.type_name()))),
    }))
}

fn index_of(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    Ok(Value::Int(match &args[0] {
        Value::Array(a) => a.borrow().iter().position(|v| values_equal(v, &args[1])).map_or(-1, |i| i as i64),
        Value::Str(s) => s.find(&**str_arg(&args[1], "index_of", pos)?).map_or(-1, |b| char_index(s, b)),
        other => return Err(type_err(pos, format!("index_of() expects array or str, got {}", other.type_name()))),
    }))
}

fn reverse(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    match &args[0] {
        Value::Array(a) => Ok(Value::array(a.borrow().iter().rev().cloned().collect())),
        Value::Str(s) => Ok(Value::string(s.chars().rev().collect())),
        other => Err(type_err(pos, format!("reverse() expects array or str, got {}", other.type_name()))),
    }
}

/// 稳定排序；遇到无法比较的元素时报 TypeError。
fn sort_values(pairs: &mut [(Value, Value)], pos: Pos) -> Result<()> {
    let mut bad = None;
    pairs.sort_by(|(a, _), (b, _)| {
        compare(a, b).unwrap_or_else(|| {
            bad.get_or_insert_with(|| (a.type_name(), b.type_name()));
            std::cmp::Ordering::Equal
        })
    });
    match bad {
        Some((a, b)) => Err(type_err(pos, format!("sort() cannot compare {a} and {b}"))),
        None => Ok(()),
    }
}

fn sort(rt: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let xs = items(&args[0], "sort", pos)?;
    let mut pairs = Vec::with_capacity(xs.len());
    for x in xs {
        let key = match args.get(1) {
            Some(f) => rt.call(f, vec![x.clone()], pos)?,
            None => x.clone(),
        };
        pairs.push((key, x));
    }
    sort_values(&mut pairs, pos)?;
    Ok(Value::array(pairs.into_iter().map(|(_, v)| v).collect()))
}

fn map(rt: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let xs = items(&args[0], "map", pos)?;
    let mut out = Vec::with_capacity(xs.len());
    for x in xs {
        out.push(rt.call(&args[1], vec![x], pos)?);
    }
    Ok(Value::array(out))
}

fn filter(rt: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let xs = items(&args[0], "filter", pos)?;
    let mut out = Vec::new();
    for x in xs {
        let keep = rt.call(&args[1], vec![x.clone()], pos)?;
        if ops::expect_bool(&keep, pos, "filter() function result")? {
            out.push(x);
        }
    }
    Ok(Value::array(out))
}

fn reduce(rt: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let f = args.pop().expect("arity");
    let mut acc = args.pop().expect("arity");
    for x in items(&args[0], "reduce", pos)? {
        acc = rt.call(&f, vec![acc, x], pos)?;
    }
    Ok(acc)
}

fn sum(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let mut acc = Value::Int(0);
    for x in items(&args[0], "sum", pos)? {
        if !matches!(x, Value::Int(_) | Value::Float(_)) {
            return Err(type_err(pos, format!("sum() expects numbers, got {}", x.type_name())));
        }
        acc = ops::binary(crate::ast::BinOp::Add, &acc, &x, pos)?;
    }
    Ok(acc)
}

fn extreme(args: Vec<Value>, f: &str, want: std::cmp::Ordering, pos: Pos) -> Result<Value> {
    let xs = match args.as_slice() {
        [single @ (Value::Array(_) | Value::Range(..))] => items(single, f, pos)?,
        _ => args,
    };
    let mut it = xs.into_iter();
    let Some(mut best) = it.next() else {
        return Err(err(ErrorKind::Value, pos, format!("{f}() of empty sequence")));
    };
    for x in it {
        match compare(&x, &best) {
            Some(o) if o == want => best = x,
            Some(_) => {}
            None => return Err(type_err(pos, format!("{f}() cannot compare {} and {}", x.type_name(), best.type_name()))),
        }
    }
    Ok(best)
}

fn min(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    extreme(std::mem::take(args), "min", std::cmp::Ordering::Less, pos)
}

fn max(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    extreme(std::mem::take(args), "max", std::cmp::Ordering::Greater, pos)
}

fn join(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let sep = str_arg(&args[1], "join", pos)?;
    let mut s = String::new();
    for (i, x) in items(&args[0], "join", pos)?.iter().enumerate() {
        if i > 0 {
            s.push_str(sep);
        }
        x.write_display(&mut s);
    }
    Ok(Value::string(s))
}

// ---------- 字符串 ----------

fn split(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let s = str_arg(&args[0], "split", pos)?;
    let sep = str_arg(&args[1], "split", pos)?;
    let parts = if sep.is_empty() {
        s.chars().map(|c| Value::str(c.encode_utf8(&mut [0; 4]))).collect()
    } else {
        s.split(&**sep).map(Value::str).collect()
    };
    Ok(Value::array(parts))
}

fn trim(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    Ok(Value::str(str_arg(&args[0], "trim", pos)?.trim()))
}

fn upper(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    Ok(Value::str(&str_arg(&args[0], "upper", pos)?.to_uppercase()))
}

fn lower(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    Ok(Value::str(&str_arg(&args[0], "lower", pos)?.to_lowercase()))
}

fn replace(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let s = str_arg(&args[0], "replace", pos)?;
    let from = str_arg(&args[1], "replace", pos)?;
    let to = str_arg(&args[2], "replace", pos)?;
    if from.is_empty() {
        return Err(err(ErrorKind::Value, pos, "replace() pattern must not be empty"));
    }
    Ok(Value::str(&s.replace(&**from, to)))
}

fn starts_with(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    Ok(Value::Bool(str_arg(&args[0], "starts_with", pos)?.starts_with(&**str_arg(&args[1], "starts_with", pos)?)))
}

fn ends_with(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    Ok(Value::Bool(str_arg(&args[0], "ends_with", pos)?.ends_with(&**str_arg(&args[1], "ends_with", pos)?)))
}

fn pad(args: &[Value], f: &str, left: bool, pos: Pos) -> Result<Value> {
    let s = args[0].to_display();
    let width = int_arg(&args[1], f, pos)?;
    let fill = match args.get(2) {
        None => ' ',
        Some(v) => {
            let c = str_arg(v, f, pos)?;
            let mut chars = c.chars();
            match (chars.next(), chars.next()) {
                (Some(ch), None) => ch,
                _ => return Err(err(ErrorKind::Value, pos, format!("{f}() fill must be a single character"))),
            }
        }
    };
    let n = s.chars().count() as i64;
    if n >= width {
        return Ok(Value::str(&s));
    }
    let padding: String = std::iter::repeat_n(fill, (width - n) as usize).collect();
    Ok(Value::str(&if left { padding + &s } else { s + &padding }))
}

fn pad_left(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    pad(args, "pad_left", true, pos)
}

fn pad_right(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    pad(args, "pad_right", false, pos)
}

fn ord(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let s = str_arg(&args[0], "ord", pos)?;
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok(Value::Int(c as i64)),
        _ => Err(err(ErrorKind::Value, pos, "ord() expects a single character")),
    }
}

fn chr(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let n = int_arg(&args[0], "chr", pos)?;
    u32::try_from(n)
        .ok()
        .and_then(char::from_u32)
        .map(|c| Value::str(c.encode_utf8(&mut [0; 4])))
        .ok_or_else(|| err(ErrorKind::Value, pos, format!("chr() argument {n} is not a valid code point")))
}

// ---------- 数学 ----------

fn abs(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    match &args[0] {
        Value::Int(n) => n.checked_abs().map(Value::Int).ok_or_else(|| err(ErrorKind::Runtime, pos, "integer overflow")),
        Value::Float(x) => Ok(Value::Float(x.abs())),
        other => Err(type_err(pos, format!("abs() expects a number, got {}", other.type_name()))),
    }
}

fn sqrt(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let x = num_arg(&args[0], "sqrt", pos)?;
    if x < 0.0 {
        return Err(err(ErrorKind::Value, pos, "sqrt() of a negative number"));
    }
    Ok(Value::Float(x.sqrt()))
}

fn rounding(args: &[Value], f: &str, op: fn(f64) -> f64, pos: Pos) -> Result<Value> {
    match &args[0] {
        Value::Int(n) => Ok(Value::Int(*n)),
        Value::Float(x) => Ok(Value::Int(float_to_int(op(*x), f, pos)?)),
        other => Err(type_err(pos, format!("{f}() expects a number, got {}", other.type_name()))),
    }
}

fn floor(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    rounding(args, "floor", f64::floor, pos)
}

fn ceil(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    rounding(args, "ceil", f64::ceil, pos)
}

fn round(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    rounding(args, "round", f64::round, pos)
}

// ---------- 其他 ----------

fn random(rt: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let a = int_arg(&args[0], "random", pos)?;
    let b = int_arg(&args[1], "random", pos)?;
    if a > b {
        return Err(err(ErrorKind::Value, pos, format!("random() lower bound {a} is greater than upper bound {b}")));
    }
    let span = (b as i128 - a as i128 + 1) as u128;
    let r = (rt.io().next_u64() as u128 % span) as i128;
    Ok(Value::Int((a as i128 + r) as i64))
}

fn seed(rt: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    rt.io().rng = int_arg(&args[0], "seed", pos)? as u64;
    Ok(Value::Nil)
}

fn time(_: &mut dyn Runtime, _: &mut Vec<Value>, _: Pos) -> Result<Value> {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    Ok(Value::Float(t))
}

fn assert(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    if ops::expect_bool(&args[0], pos, "assert() condition")? {
        return Ok(Value::Nil);
    }
    let msg = args.get(1).map_or_else(|| "assertion failed".to_string(), Value::to_display);
    Err(err(ErrorKind::Assertion, pos, msg))
}

fn assert_eq(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    if values_equal(&args[0], &args[1]) {
        return Ok(Value::Nil);
    }
    Err(err(ErrorKind::Assertion, pos, format!("{} != {}", args[0].repr(), args[1].repr())))
}

fn error(_: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    Err(err(ErrorKind::Runtime, pos, args[0].to_display()))
}

fn exit(rt: &mut dyn Runtime, args: &mut Vec<Value>, pos: Pos) -> Result<Value> {
    let code = match args.first() {
        Some(v) => int_arg(v, "exit", pos)?,
        None => 0,
    };
    rt.io().flush();
    std::process::exit(code as i32)
}
