//! 中间表示：槽位解析后的语法树，变量已定位为槽位编号。两个执行后端共用（ADR-0002）。
//!
//! 变量的存放位置：
//! - `Local`：当前帧值栈上的普通槽位；
//! - `Cell`：被闭包捕获的局部变量，存放在共享单元格中，每次进入声明它的块时重新分配；
//! - `Upval`：当前闭包捕获的外层单元格；
//! - `Global`：脚本顶层作用域的变量。

use std::rc::Rc;

use crate::ast::{BinOp, Pos, UnOp};
use crate::value::Value;

/// 声明的目标位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Local(u32),
    Cell(u32),
    Global(u32),
}

/// 读写变量的位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Var {
    Local(u32),
    Cell(u32),
    Upval(u32),
    Global(u32),
}

impl From<Slot> for Var {
    fn from(s: Slot) -> Var {
        match s {
            Slot::Local(i) => Var::Local(i),
            Slot::Cell(i) => Var::Cell(i),
            Slot::Global(i) => Var::Global(i),
        }
    }
}

/// 创建闭包时，每个捕获变量从哪里来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpSrc {
    /// 外层函数帧中的单元格
    Cell(u32),
    /// 外层闭包自己捕获的单元格
    Upval(u32),
}

pub struct FnProto {
    /// 在 `Program::protos` 中的下标；脚本顶层为 `protos.len()`。字节码后端据此找到对应的代码块。
    pub id: usize,
    pub name: Option<Rc<str>>,
    pub arity: usize,
    /// 参数占用局部槽位 `0..arity`。
    pub n_locals: usize,
    pub n_cells: usize,
    /// 被捕获的参数：(参数序号, 单元格编号)
    pub param_cells: Vec<(usize, u32)>,
    pub upvals: Vec<UpSrc>,
    pub body: Block,
}

pub struct StructProto {
    pub name: Rc<str>,
    pub fields: Vec<Rc<str>>,
    /// 默认值编译为无参函数，值为 `Program::protos` 的下标。
    pub defaults: Vec<Option<u32>>,
}

pub struct Block {
    /// 进入块时需要新分配的单元格。
    pub cells: Vec<u32>,
    /// 声明提升：进入块时依次执行（结构体、函数、impl）。
    pub hoists: Vec<Hoist>,
    pub stmts: Vec<Stmt>,
}

pub enum Hoist {
    Struct(Slot, Rc<StructProto>),
    Fn(Slot, u32),
    Impl { target: Var, name: Rc<str>, methods: Vec<(Rc<str>, u32)>, pos: Pos },
}

pub enum Stmt {
    Let(Slot, Expr),
    Assign(Target, Option<BinOp>, Expr, Pos),
    Expr(Expr),
    While(Expr, Block, Pos),
    For { index: Option<Slot>, var: Slot, iter: Expr, body: Block, pos: Pos },
    Break,
    Continue,
    Return(Option<Expr>),
}

pub enum Target {
    Var(Var, Rc<str>, Pos),
    Index(Expr, Expr, Pos),
    Field(Expr, Rc<str>, Pos),
}

pub enum Expr {
    Const(Value),
    /// 变量读取；名字用于"尚未初始化"的报错。
    Var(Var, Rc<str>, Pos),
    Interp(Vec<Expr>),
    Array(Vec<Expr>),
    Unary(UnOp, Box<Expr>, Pos),
    Binary(BinOp, Box<Expr>, Box<Expr>, Pos),
    Range(Box<Expr>, Box<Expr>, bool, Pos),
    Call { callee: Box<Expr>, args: Vec<Expr>, names: Option<Vec<Rc<str>>>, pos: Pos },
    /// 统一调用语法：先找结构体方法，找不到再调用 `fallback`（解析期确定的同名函数）。
    MethodCall { obj: Box<Expr>, name: Rc<str>, fallback: Option<Box<Expr>>, args: Vec<Expr>, pos: Pos },
    Index(Box<Expr>, Box<Expr>, Pos),
    Field(Box<Expr>, Rc<str>, Pos),
    /// 创建闭包，值为 `Program::protos` 的下标。
    Lambda(u32),
    If(Box<Expr>, Block, Option<Box<Expr>>, Pos),
    Match(Box<Expr>, Vec<Arm>, Pos),
    Block(Block),
}

pub struct Arm {
    pub cells: Vec<u32>,
    pub pattern: Pattern,
    pub guard: Option<(Expr, Pos)>,
    pub body: Expr,
}

#[derive(Clone)]
pub enum Pattern {
    Wildcard,
    Bind(Slot),
    Lit(Value),
    Range { lo: Option<i64>, hi: Option<i64>, inclusive: bool },
    Or(Vec<Pattern>),
}

pub struct Program {
    pub main: Rc<FnProto>,
    pub protos: Vec<Rc<FnProto>>,
    pub n_globals: usize,
}
