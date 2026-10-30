//! 抽象语法树，以及用于调试和测试的 S 表达式打印（`zyx --ast`）。
//!
//! 管道 `x |> f(a)` 在语法分析时已脱糖为 `f(x, a)`，AST 中不再出现管道节点。

use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
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
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Int(i64),
    Float(f64),
    Str(String),
    Interp(Vec<InterpPart>),
    Bool(bool),
    Nil,
    Ident(String),
    Array(Vec<Expr>),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Range { start: Box<Expr>, end: Box<Expr>, inclusive: bool },
    Call { callee: Box<Expr>, args: Vec<Arg> },
    /// `obj.name(args)`，按统一调用语法解析（ADR-0005）。
    MethodCall { obj: Box<Expr>, name: String, args: Vec<Arg> },
    Index(Box<Expr>, Box<Expr>),
    Field(Box<Expr>, String),
    Lambda(Rc<FnDecl>),
    If { cond: Box<Expr>, then: Block, else_: Option<Box<Expr>> },
    Match { subject: Box<Expr>, arms: Vec<Arm> },
    Block(Block),
}

#[derive(Debug, Clone, PartialEq)]
pub enum InterpPart {
    Lit(String),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    /// 具名参数 `name: expr`，只对结构体构造有效。
    pub name: Option<String>,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnDecl {
    /// 匿名函数为 `None`。
    pub name: Option<String>,
    pub params: Vec<String>,
    pub body: Block,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDecl {
    pub name: String,
    pub fields: Vec<(String, Option<Expr>)>,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arm {
    pub pattern: Pattern,
    pub guard: Option<Expr>,
    pub body: Expr,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Wildcard,
    Bind(String),
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Nil,
    /// `lo..hi`、`lo..=hi`、`lo..`、`..hi`、`..=hi`
    Range { lo: Option<i64>, hi: Option<i64>, inclusive: bool },
    Or(Vec<Pattern>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub pos: Pos,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    Let { name: String, init: Option<Expr> },
    /// `op` 为复合赋值的运算符，普通赋值为 `None`。目标只能是标识符、下标或字段。
    Assign { target: Expr, op: Option<BinOp>, value: Expr },
    Expr(Expr),
    Fn(Rc<FnDecl>),
    Struct(Rc<StructDecl>),
    Impl { name: String, methods: Vec<Rc<FnDecl>> },
    While { cond: Expr, body: Block },
    For { index: Option<String>, var: String, iter: Expr, body: Block },
    Break,
    Continue,
    Return(Option<Expr>),
}

pub type Program = Vec<Stmt>;

impl BinOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::Pow => "**",
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::And => "and",
            BinOp::Or => "or",
        }
    }
}

// ---------- S 表达式打印 ----------

pub fn sexpr_program(prog: &Program) -> String {
    prog.iter().map(sexpr_stmt).collect::<Vec<_>>().join("\n")
}

fn list(head: &str, items: impl IntoIterator<Item = String>) -> String {
    let mut s = format!("({head}");
    for item in items {
        s.push(' ');
        s.push_str(&item);
    }
    s.push(')');
    s
}

fn sexpr_block(b: &Block) -> String {
    list("block", b.stmts.iter().map(sexpr_stmt))
}

fn sexpr_fn(f: &FnDecl) -> String {
    let params = format!("({})", f.params.join(" "));
    match &f.name {
        Some(name) => list("fn", [name.clone(), params, sexpr_block(&f.body)]),
        None => list("fn", [params, sexpr_block(&f.body)]),
    }
}

fn sexpr_args(args: &[Arg]) -> impl Iterator<Item = String> + '_ {
    args.iter().map(|a| match &a.name {
        Some(n) => format!("({n}: {})", sexpr_expr(&a.value)),
        None => sexpr_expr(&a.value),
    })
}

pub fn sexpr_stmt(s: &Stmt) -> String {
    match &s.kind {
        StmtKind::Let { name, init } => match init {
            Some(e) => list("let", [name.clone(), sexpr_expr(e)]),
            None => list("let", [name.clone()]),
        },
        StmtKind::Assign { target, op, value } => {
            let head = match op {
                Some(op) => format!("{}=", op.symbol()),
                None => "=".to_string(),
            };
            list(&head, [sexpr_expr(target), sexpr_expr(value)])
        }
        StmtKind::Expr(e) => sexpr_expr(e),
        StmtKind::Fn(f) => sexpr_fn(f),
        StmtKind::Struct(d) => {
            let fields = d.fields.iter().map(|(name, default)| match default {
                Some(e) => format!("({name} {})", sexpr_expr(e)),
                None => name.clone(),
            });
            list("struct", std::iter::once(d.name.clone()).chain(fields))
        }
        StmtKind::Impl { name, methods } => {
            list("impl", std::iter::once(name.clone()).chain(methods.iter().map(|m| sexpr_fn(m))))
        }
        StmtKind::While { cond, body } => list("while", [sexpr_expr(cond), sexpr_block(body)]),
        StmtKind::For { index, var, iter, body } => {
            let vars = match index {
                Some(i) => format!("({i} {var})"),
                None => var.clone(),
            };
            list("for", [vars, sexpr_expr(iter), sexpr_block(body)])
        }
        StmtKind::Break => "(break)".to_string(),
        StmtKind::Continue => "(continue)".to_string(),
        StmtKind::Return(e) => list("return", e.iter().map(sexpr_expr)),
    }
}

pub fn sexpr_expr(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Int(n) => n.to_string(),
        ExprKind::Float(x) => format!("{x:?}"),
        ExprKind::Str(s) => format!("{s:?}"),
        ExprKind::Interp(parts) => list(
            "interp",
            parts.iter().map(|p| match p {
                InterpPart::Lit(s) => format!("{s:?}"),
                InterpPart::Expr(e) => sexpr_expr(e),
            }),
        ),
        ExprKind::Bool(b) => b.to_string(),
        ExprKind::Nil => "nil".to_string(),
        ExprKind::Ident(name) => name.clone(),
        ExprKind::Array(items) => list("array", items.iter().map(sexpr_expr)),
        ExprKind::Unary(op, x) => {
            let head = match op {
                UnOp::Neg => "-",
                UnOp::Not => "not",
            };
            list(head, [sexpr_expr(x)])
        }
        ExprKind::Binary(op, a, b) => list(op.symbol(), [sexpr_expr(a), sexpr_expr(b)]),
        ExprKind::Range { start, end, inclusive } => {
            list(if *inclusive { "..=" } else { ".." }, [sexpr_expr(start), sexpr_expr(end)])
        }
        ExprKind::Call { callee, args } => list("call", std::iter::once(sexpr_expr(callee)).chain(sexpr_args(args))),
        ExprKind::MethodCall { obj, name, args } => {
            list("method", [sexpr_expr(obj), name.clone()].into_iter().chain(sexpr_args(args)))
        }
        ExprKind::Index(a, i) => list("index", [sexpr_expr(a), sexpr_expr(i)]),
        ExprKind::Field(a, name) => list("field", [sexpr_expr(a), name.clone()]),
        ExprKind::Lambda(f) => sexpr_fn(f),
        ExprKind::If { cond, then, else_ } => {
            list("if", [sexpr_expr(cond), sexpr_block(then)].into_iter().chain(else_.iter().map(|e| sexpr_expr(e))))
        }
        ExprKind::Match { subject, arms } => {
            let arms = arms.iter().map(|arm| {
                let mut items = vec![sexpr_pattern(&arm.pattern)];
                if let Some(g) = &arm.guard {
                    items.push(list("if", [sexpr_expr(g)]));
                }
                items.push(sexpr_expr(&arm.body));
                list("arm", items)
            });
            list("match", std::iter::once(sexpr_expr(subject)).chain(arms))
        }
        ExprKind::Block(b) => sexpr_block(b),
    }
}

fn sexpr_pattern(p: &Pattern) -> String {
    match p {
        Pattern::Wildcard => "_".to_string(),
        Pattern::Bind(name) => name.clone(),
        Pattern::Int(n) => n.to_string(),
        Pattern::Float(x) => format!("{x:?}"),
        Pattern::Str(s) => format!("{s:?}"),
        Pattern::Bool(b) => b.to_string(),
        Pattern::Nil => "nil".to_string(),
        Pattern::Range { lo, hi, inclusive } => {
            let lo = lo.map(|n| n.to_string()).unwrap_or_default();
            let hi = hi.map(|n| n.to_string()).unwrap_or_default();
            format!("{lo}{}{hi}", if *inclusive { "..=" } else { ".." })
        }
        Pattern::Or(ps) => list("|", ps.iter().map(sexpr_pattern)),
    }
}
