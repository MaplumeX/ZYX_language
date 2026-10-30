//! 运行时的值。数组和结构体实例是引用语义（reference.md 2.1）。

use std::cell::RefCell;
use std::cmp::Ordering;
use std::fmt::Write;
use std::rc::Rc;

use crate::builtins::Builtin;
use crate::ir::FnProto;

#[derive(Clone)]
pub enum Value {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    Array(Rc<RefCell<Vec<Value>>>),
    /// 左闭右开区间 `start..end`；`a..=b` 存为 `a..b+1`。
    Range(i64, i64),
    Func(Rc<Closure>),
    Builtin(&'static Builtin),
    StructType(Rc<StructType>),
    Struct(Rc<Instance>),
    /// 内部值：已分配但尚未执行到 `let` 的变量。用户代码永远拿不到它。
    Undef,
}

pub type Cell = Rc<RefCell<Value>>;

pub struct Closure {
    pub proto: Rc<FnProto>,
    pub upvals: Vec<Cell>,
}

pub struct StructType {
    pub name: Rc<str>,
    pub fields: Vec<Rc<str>>,
    /// 字段默认值，各自是一个无参闭包，每次构造时求值。
    pub defaults: Vec<Option<Rc<Closure>>>,
    pub methods: RefCell<Vec<(Rc<str>, Rc<Closure>)>>,
}

pub struct Instance {
    pub ty: Rc<StructType>,
    pub fields: RefCell<Vec<Value>>,
}

impl StructType {
    pub fn field_index(&self, name: &str) -> Option<usize> {
        // 名字经过驻留，先比指针，命中率最高
        self.fields
            .iter()
            .position(|f| std::ptr::eq(f.as_ptr(), name.as_ptr()) || &**f == name)
    }

    pub fn method(&self, name: &str) -> Option<Rc<Closure>> {
        self.methods.borrow().iter().find(|(n, _)| &**n == name).map(|(_, m)| m.clone())
    }
}

impl Value {
    pub fn str(s: &str) -> Value {
        Value::Str(Rc::from(s))
    }

    pub fn array(items: Vec<Value>) -> Value {
        Value::Array(Rc::new(RefCell::new(items)))
    }

    pub fn type_name(&self) -> String {
        match self {
            Value::Nil => "nil".into(),
            Value::Bool(_) => "bool".into(),
            Value::Int(_) => "int".into(),
            Value::Float(_) => "float".into(),
            Value::Str(_) => "str".into(),
            Value::Array(_) => "array".into(),
            Value::Range(..) => "range".into(),
            Value::Func(_) | Value::Builtin(_) => "fn".into(),
            Value::StructType(_) => "struct".into(),
            Value::Struct(inst) => inst.ty.name.to_string(),
            Value::Undef => "undefined".into(),
        }
    }

    /// `print`、插值和 `str()` 使用的形式：顶层字符串不加引号。
    pub fn to_display(&self) -> String {
        match self {
            Value::Str(s) => s.to_string(),
            _ => {
                let mut out = String::new();
                self.write_repr(&mut out);
                out
            }
        }
    }

    pub fn write_display(&self, out: &mut String) {
        match self {
            Value::Str(s) => out.push_str(s),
            _ => self.write_repr(out),
        }
    }

    /// 容器内部的形式：字符串加引号。
    pub fn repr(&self) -> String {
        let mut out = String::new();
        self.write_repr(&mut out);
        out
    }

    pub fn write_repr(&self, out: &mut String) {
        match self {
            Value::Nil => out.push_str("nil"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Int(n) => {
                let _ = write!(out, "{n}");
            }
            Value::Float(x) => out.push_str(&format_float(*x)),
            Value::Str(s) => {
                out.push('"');
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\t' => out.push_str("\\t"),
                        '\r' => out.push_str("\\r"),
                        _ => out.push(c),
                    }
                }
                out.push('"');
            }
            Value::Array(items) => {
                out.push('[');
                for (i, v) in items.borrow().iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    v.write_repr(out);
                }
                out.push(']');
            }
            Value::Range(a, b) => {
                let _ = write!(out, "{a}..{b}");
            }
            Value::Func(c) => match &c.proto.name {
                Some(name) => {
                    let _ = write!(out, "<fn {name}>");
                }
                None => out.push_str("<fn>"),
            },
            Value::Builtin(b) => {
                let _ = write!(out, "<fn {}>", b.name);
            }
            Value::StructType(t) => {
                let _ = write!(out, "<struct {}>", t.name);
            }
            Value::Struct(inst) => {
                out.push_str(&inst.ty.name);
                out.push('(');
                for (i, (name, v)) in inst.ty.fields.iter().zip(inst.fields.borrow().iter()).enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(name);
                    out.push_str(": ");
                    v.write_repr(out);
                }
                out.push(')');
            }
            Value::Undef => out.push_str("<undefined>"),
        }
    }
}

pub fn format_float(x: f64) -> String {
    if x.is_nan() {
        "nan".into()
    } else if x.is_infinite() {
        if x > 0.0 { "inf".into() } else { "-inf".into() }
    } else {
        // Debug 形式是最短的可回读表示，且整数值保留 ".0"
        format!("{x:?}")
    }
}

/// `==` 的语义（reference.md 2.3）：不同类型不相等，int 与 float 按数值比较，容器按内容比较。
pub fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Nil, Value::Nil) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Int(x), Value::Float(y)) | (Value::Float(y), Value::Int(x)) => (*x as f64) == *y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => {
            if Rc::ptr_eq(x, y) {
                return true;
            }
            let (x, y) = (x.borrow(), y.borrow());
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(a, b)| values_equal(a, b))
        }
        (Value::Range(a1, b1), Value::Range(a2, b2)) => a1 == a2 && b1 == b2,
        (Value::Func(x), Value::Func(y)) => Rc::ptr_eq(x, y),
        (Value::Builtin(x), Value::Builtin(y)) => std::ptr::eq(*x, *y),
        (Value::StructType(x), Value::StructType(y)) => Rc::ptr_eq(x, y),
        (Value::Struct(x), Value::Struct(y)) => {
            if Rc::ptr_eq(x, y) {
                return true;
            }
            Rc::ptr_eq(&x.ty, &y.ty) && {
                let (fx, fy) = (x.fields.borrow(), y.fields.borrow());
                fx.iter().zip(fy.iter()).all(|(a, b)| values_equal(a, b))
            }
        }
        _ => false,
    }
}

/// `<` 等比较：只支持两个数或两个字符串，其余返回 `None`。
pub fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Some(x.cmp(y)),
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y),
        (Value::Int(x), Value::Float(y)) => (*x as f64).partial_cmp(y),
        (Value::Float(x), Value::Int(y)) => x.partial_cmp(&(*y as f64)),
        (Value::Str(x), Value::Str(y)) => Some(x.cmp(y)),
        _ => None,
    }
}
