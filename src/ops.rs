//! 运算符、下标与字段访问的语义（reference.md 第 3 节），两个执行后端共用。

use std::cell::RefCell;
use std::rc::Rc;

use crate::ast::{BinOp, Pos, UnOp};
use crate::error::{ErrorKind, Result, ZyxError};
use crate::ir::FnProto;
use crate::value::{Closure, Instance, StructType, Value, compare, values_equal};

pub fn err(kind: ErrorKind, pos: Pos, msg: impl Into<String>) -> ZyxError {
    ZyxError::new(kind, pos.line, pos.col, msg)
}

pub fn type_err(pos: Pos, msg: impl Into<String>) -> ZyxError {
    err(ErrorKind::Type, pos, msg)
}

fn overflow(pos: Pos) -> ZyxError {
    err(ErrorKind::Runtime, pos, "integer overflow")
}

fn div_zero(pos: Pos) -> ZyxError {
    err(ErrorKind::Runtime, pos, "division by zero")
}

/// 条件必须是 bool（ADR-0007）。
pub fn expect_bool(v: &Value, pos: Pos, what: &str) -> Result<bool> {
    match v {
        Value::Bool(b) => Ok(*b),
        other => Err(type_err(pos, format!("{what} must be bool, got {}", other.type_name()))),
    }
}

/// 向下取整的整数除法。
pub fn floor_div(a: i64, b: i64, pos: Pos) -> Result<i64> {
    if b == 0 {
        return Err(div_zero(pos));
    }
    let q = a.checked_div(b).ok_or_else(|| overflow(pos))?;
    Ok(if a % b != 0 && ((a < 0) != (b < 0)) { q - 1 } else { q })
}

/// 结果符号与除数相同的取模。
pub fn floor_mod(a: i64, b: i64, pos: Pos) -> Result<i64> {
    if b == 0 {
        return Err(div_zero(pos));
    }
    let r = a.checked_rem(b).unwrap_or(0);
    Ok(if r != 0 && ((r < 0) != (b < 0)) { r + b } else { r })
}

fn float_mod(a: f64, b: f64) -> f64 {
    let r = a % b;
    if r != 0.0 && ((r < 0.0) != (b < 0.0)) { r + b } else { r }
}

fn as_float(v: &Value) -> Option<f64> {
    match v {
        Value::Int(n) => Some(*n as f64),
        Value::Float(x) => Some(*x),
        _ => None,
    }
}

fn bad_operands(op: BinOp, a: &Value, b: &Value, pos: Pos) -> ZyxError {
    type_err(pos, format!("unsupported operand types for '{}': {} and {}", op.symbol(), a.type_name(), b.type_name()))
}

pub fn binary(op: BinOp, a: &Value, b: &Value, pos: Pos) -> Result<Value> {
    use Value::{Float, Int, Str};
    let v = match (op, a, b) {
        (BinOp::Add, Int(x), Int(y)) => Int(x.checked_add(*y).ok_or_else(|| overflow(pos))?),
        (BinOp::Sub, Int(x), Int(y)) => Int(x.checked_sub(*y).ok_or_else(|| overflow(pos))?),
        (BinOp::Mul, Int(x), Int(y)) => Int(x.checked_mul(*y).ok_or_else(|| overflow(pos))?),
        (BinOp::Div, Int(x), Int(y)) => Int(floor_div(*x, *y, pos)?),
        (BinOp::Mod, Int(x), Int(y)) => Int(floor_mod(*x, *y, pos)?),
        (BinOp::Pow, Int(x), Int(y)) => {
            if *y < 0 {
                return Err(err(ErrorKind::Value, pos, "negative exponent in integer power, use a float base"));
            }
            let e = u32::try_from(*y).map_err(|_| overflow(pos))?;
            Int(x.checked_pow(e).ok_or_else(|| overflow(pos))?)
        }
        (BinOp::Add, Str(x), Str(y)) => {
            let mut s = String::with_capacity(x.len() + y.len());
            s.push_str(x);
            s.push_str(y);
            Value::string(s)
        }
        (BinOp::Mul, Str(s), Int(n)) => Value::string(s.repeat((*n).max(0) as usize)),
        (BinOp::Eq, _, _) => Value::Bool(values_equal(a, b)),
        (BinOp::Ne, _, _) => Value::Bool(!values_equal(a, b)),
        (BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge, _, _) => {
            let Some(ord) = compare(a, b) else {
                return Err(type_err(pos, format!("cannot compare {} and {}", a.type_name(), b.type_name())));
            };
            Value::Bool(match op {
                BinOp::Lt => ord.is_lt(),
                BinOp::Le => ord.is_le(),
                BinOp::Gt => ord.is_gt(),
                _ => ord.is_ge(),
            })
        }
        (BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow, _, _) => {
            let (Some(x), Some(y)) = (as_float(a), as_float(b)) else {
                return Err(bad_operands(op, a, b, pos));
            };
            match op {
                BinOp::Add => Float(x + y),
                BinOp::Sub => Float(x - y),
                BinOp::Mul => Float(x * y),
                BinOp::Div if y == 0.0 => return Err(div_zero(pos)),
                BinOp::Div => Float(x / y),
                BinOp::Mod if y == 0.0 => return Err(div_zero(pos)),
                BinOp::Mod => Float(float_mod(x, y)),
                _ => Float(x.powf(y)),
            }
        }
        (BinOp::And | BinOp::Or, _, _) => unreachable!("logical operators short-circuit in the backend"),
    };
    Ok(v)
}

pub fn unary(op: UnOp, v: &Value, pos: Pos) -> Result<Value> {
    match (op, v) {
        (UnOp::Neg, Value::Int(n)) => Ok(Value::Int(n.checked_neg().ok_or_else(|| overflow(pos))?)),
        (UnOp::Neg, Value::Float(x)) => Ok(Value::Float(-x)),
        (UnOp::Neg, other) => Err(type_err(pos, format!("cannot negate {}", other.type_name()))),
        (UnOp::Not, other) => Ok(Value::Bool(!expect_bool(other, pos, "operand of 'not'")?)),
    }
}

pub fn make_range(a: &Value, b: &Value, inclusive: bool, pos: Pos) -> Result<Value> {
    let (Value::Int(start), Value::Int(end)) = (a, b) else {
        return Err(type_err(pos, format!("range bounds must be int, got {} and {}", a.type_name(), b.type_name())));
    };
    let end = if inclusive { end.checked_add(1).ok_or_else(|| overflow(pos))? } else { *end };
    Ok(Value::range(*start, end))
}

/// 负数下标从末尾倒数。
pub fn norm_index(i: i64, len: usize) -> Option<usize> {
    let i = if i < 0 { i + len as i64 } else { i };
    if i >= 0 && (i as usize) < len { Some(i as usize) } else { None }
}

fn index_err(i: i64, what: &str, len: usize, pos: Pos) -> ZyxError {
    err(ErrorKind::Index, pos, format!("index {i} out of range for {what} of length {len}"))
}

/// 切片端点：负数从末尾倒数，再截断到 `0..=len`。
fn clamp_slice(start: i64, end: i64, len: usize) -> (usize, usize) {
    let clamp = |x: i64| (if x < 0 { x + len as i64 } else { x }).clamp(0, len as i64) as usize;
    let (s, e) = (clamp(start), clamp(end));
    (s, e.max(s))
}

pub fn index_get(c: &Value, i: &Value, pos: Pos) -> Result<Value> {
    match (c, i) {
        (Value::Array(a), Value::Int(n)) => {
            let a = a.borrow();
            match norm_index(*n, a.len()) {
                Some(k) => Ok(a[k].clone()),
                None => Err(index_err(*n, "array", a.len(), pos)),
            }
        }
        (Value::Array(a), Value::Range(r)) => {
            let a = a.borrow();
            let (s, e) = clamp_slice(r.0, r.1, a.len());
            Ok(Value::array(a[s..e].to_vec()))
        }
        (Value::Str(s), Value::Int(n)) => {
            if s.is_ascii() {
                return match norm_index(*n, s.len()) {
                    Some(k) => Ok(Value::str(&s[k..k + 1])),
                    None => Err(index_err(*n, "string", s.len(), pos)),
                };
            }
            let len = s.chars().count();
            match norm_index(*n, len) {
                Some(k) => Ok(Value::str(s.chars().nth(k).expect("in range").encode_utf8(&mut [0; 4]))),
                None => Err(index_err(*n, "string", len, pos)),
            }
        }
        (Value::Str(s), Value::Range(r)) => {
            if s.is_ascii() {
                let (a, b) = clamp_slice(r.0, r.1, s.len());
                return Ok(Value::str(&s[a..b]));
            }
            let len = s.chars().count();
            let (a, b) = clamp_slice(r.0, r.1, len);
            Ok(Value::string(s.chars().skip(a).take(b - a).collect()))
        }
        (Value::Array(_) | Value::Str(_), other) => {
            Err(type_err(pos, format!("index must be int or range, got {}", other.type_name())))
        }
        (other, _) => Err(type_err(pos, format!("{} does not support indexing", other.type_name()))),
    }
}

pub fn index_set(c: &Value, i: &Value, v: Value, pos: Pos) -> Result<()> {
    match (c, i) {
        (Value::Array(a), Value::Int(n)) => {
            let mut a = a.borrow_mut();
            let len = a.len();
            match norm_index(*n, len) {
                Some(k) => {
                    a[k] = v;
                    Ok(())
                }
                None => Err(index_err(*n, "array", len, pos)),
            }
        }
        (Value::Array(_), other) => Err(type_err(pos, format!("array index must be int, got {}", other.type_name()))),
        (Value::Str(_), _) => Err(type_err(pos, "strings are immutable")),
        (other, _) => Err(type_err(pos, format!("{} does not support index assignment", other.type_name()))),
    }
}

pub fn field_get(obj: &Value, name: &str, pos: Pos) -> Result<Value> {
    match obj {
        Value::Struct(inst) => match inst.ty.field_index(name) {
            Some(k) => Ok(inst.fields.borrow()[k].clone()),
            None => Err(type_err(pos, format!("{} has no field '{name}'", inst.ty.name))),
        },
        other => Err(type_err(pos, format!("{} has no field '{name}'", other.type_name()))),
    }
}

pub fn field_set(obj: &Value, name: &str, v: Value, pos: Pos) -> Result<()> {
    match obj {
        Value::Struct(inst) => match inst.ty.field_index(name) {
            Some(k) => {
                inst.fields.borrow_mut()[k] = v;
                Ok(())
            }
            None => Err(type_err(pos, format!("{} has no field '{name}'", inst.ty.name))),
        },
        other => Err(type_err(pos, format!("{} has no field '{name}'", other.type_name()))),
    }
}

// ---------- 调用相关（两个后端共用） ----------

pub fn arity_error(proto: &FnProto, argc: usize, pos: Pos) -> ZyxError {
    let name = proto.name.as_deref().map_or_else(|| "anonymous function".to_string(), |n| format!("function '{n}'"));
    let s = if proto.arity == 1 { "" } else { "s" };
    type_err(pos, format!("{name} expects {} argument{s}, got {argc}", proto.arity))
}

pub fn named_args_error(pos: Pos) -> ZyxError {
    type_err(pos, "named arguments are only allowed when constructing a struct")
}

pub fn not_callable(v: &Value, pos: Pos) -> ZyxError {
    type_err(pos, format!("{} is not callable", v.type_name()))
}

pub fn no_method_error(obj: &Value, name: &str, pos: Pos) -> ZyxError {
    err(ErrorKind::Name, pos, format!("{} has no method '{name}' and no function '{name}' is defined", obj.type_name()))
}

pub fn stack_overflow(pos: Pos) -> ZyxError {
    err(ErrorKind::Runtime, pos, "stack overflow")
}

/// 统一调用语法的第一步：对象是结构体且定义了该方法时返回方法。
pub fn find_method(obj: &Value, name: &str) -> Option<Rc<Closure>> {
    match obj {
        Value::Struct(inst) => inst.ty.method(name),
        _ => None,
    }
}

/// 执行 `impl`：把方法挂到结构体类型上，同名方法后定义的覆盖先定义的。
pub fn add_methods(target: &Value, name: &str, methods: Vec<(Rc<str>, Rc<Closure>)>, pos: Pos) -> Result<()> {
    let Value::StructType(ty) = target else {
        return Err(type_err(pos, format!("impl target '{name}' is not a struct")));
    };
    let mut ms = ty.methods.borrow_mut();
    for (mname, m) in methods {
        match ms.iter_mut().find(|(n, _)| *n == mname) {
            Some(entry) => entry.1 = m,
            None => ms.push((mname, m)),
        }
    }
    Ok(())
}

/// 构造结构体实例。`call_default` 用于求值字段默认值（无参闭包）。
pub fn construct(
    ty: &Rc<StructType>,
    args: Vec<Value>,
    names: Option<&[Rc<str>]>,
    pos: Pos,
    call_default: &mut dyn FnMut(&Rc<Closure>) -> Result<Value>,
) -> Result<Value> {
    let n = ty.fields.len();
    let mut vals: Vec<Option<Value>> = vec![None; n];
    match names {
        None => {
            if args.len() > n {
                return Err(type_err(pos, format!("{} has {n} fields, got {} arguments", ty.name, args.len())));
            }
            for (slot, v) in vals.iter_mut().zip(args) {
                *slot = Some(v);
            }
        }
        Some(names) => {
            for (name, v) in names.iter().zip(args) {
                let Some(k) = ty.field_index(name) else {
                    return Err(type_err(pos, format!("{} has no field '{name}'", ty.name)));
                };
                vals[k] = Some(v);
            }
        }
    }
    let mut fields = Vec::with_capacity(n);
    for (k, v) in vals.into_iter().enumerate() {
        fields.push(match (v, &ty.defaults[k]) {
            (Some(v), _) => v,
            (None, Some(d)) => call_default(d)?,
            (None, None) => {
                return Err(type_err(pos, format!("missing field '{}' when constructing {}", ty.fields[k], ty.name)));
            }
        });
    }
    Ok(Value::Struct(Rc::new(Instance { ty: ty.clone(), fields: RefCell::new(fields) })))
}
