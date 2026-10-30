//! 语法分析：`Vec<Token>` → `Program`。递归下降，层级与 reference.md 附录 A 的 EBNF 一一对应。
//!
//! `break` / `continue` / `return` 的位置检查也在这里完成。

use std::rc::Rc;

use crate::ast::*;
use crate::error::{Result, ZyxError};
use crate::lexer;
use crate::token::{StrPart, Tok, Token};

pub fn parse(src: &str) -> Result<Program> {
    let toks = lexer::lex(src)?;
    Parser::new(toks).program()
}

struct Parser {
    toks: Vec<Token>,
    i: usize,
    loop_depth: usize,
    fn_depth: usize,
}

impl Parser {
    fn new(toks: Vec<Token>) -> Self {
        Parser { toks, i: 0, loop_depth: 0, fn_depth: 0 }
    }

    // ---------- 基础操作 ----------

    fn peek(&self) -> &Tok {
        &self.toks[self.i].tok
    }

    fn peek_at(&self, k: usize) -> &Tok {
        let j = (self.i + k).min(self.toks.len() - 1);
        &self.toks[j].tok
    }

    fn pos(&self) -> Pos {
        let t = &self.toks[self.i];
        Pos { line: t.line, col: t.col }
    }

    fn advance(&mut self) -> Token {
        let t = self.toks[self.i].clone();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }

    fn at(&self, tok: &Tok) -> bool {
        std::mem::discriminant(self.peek()) == std::mem::discriminant(tok)
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.at(tok) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn err_here(&self, msg: impl Into<String>) -> ZyxError {
        let p = self.pos();
        ZyxError::syntax(p.line, p.col, msg)
    }

    fn err_at(pos: Pos, msg: impl Into<String>) -> ZyxError {
        ZyxError::syntax(pos.line, pos.col, msg)
    }

    fn expect(&mut self, tok: &Tok, context: &str) -> Result<Token> {
        if self.at(tok) {
            Ok(self.advance())
        } else {
            Err(self.err_here(format!("expected {} {context}, found {}", tok.describe(), self.peek().describe())))
        }
    }

    fn ident(&mut self, context: &str) -> Result<String> {
        match self.peek().clone() {
            Tok::Ident(name) => {
                self.advance();
                Ok(name)
            }
            other => Err(self.err_here(format!("expected identifier {context}, found {}", other.describe()))),
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek(), Tok::Newline | Tok::Semi) {
            self.advance();
        }
    }

    /// 语句之后必须是换行、`;`、`}` 或文件结束。
    fn end_stmt(&mut self) -> Result<()> {
        match self.peek() {
            Tok::Newline | Tok::Semi | Tok::RBrace | Tok::Eof => Ok(()),
            other => Err(self.err_here(format!("expected end of statement, found {}", other.describe()))),
        }
    }

    // ---------- 语句 ----------

    fn program(mut self) -> Result<Program> {
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.at(&Tok::Eof) {
            if self.at(&Tok::RBrace) {
                return Err(self.err_here("unexpected '}'"));
            }
            stmts.push(self.statement()?);
            self.end_stmt()?;
            self.skip_newlines();
        }
        Ok(stmts)
    }

    fn block(&mut self) -> Result<Block> {
        let pos = self.pos();
        self.expect(&Tok::LBrace, "to start a block")?;
        let mut stmts = Vec::new();
        self.skip_newlines();
        while !self.at(&Tok::RBrace) {
            if self.at(&Tok::Eof) {
                return Err(Self::err_at(pos, "unclosed '{', expected '}'"));
            }
            stmts.push(self.statement()?);
            self.end_stmt()?;
            self.skip_newlines();
        }
        self.advance();
        Ok(Block { stmts, pos })
    }

    fn loop_body(&mut self) -> Result<Block> {
        self.loop_depth += 1;
        let body = self.block();
        self.loop_depth -= 1;
        body
    }

    fn statement(&mut self) -> Result<Stmt> {
        let pos = self.pos();
        let kind = match self.peek() {
            Tok::Let => {
                self.advance();
                let name = self.ident("after 'let'")?;
                let init = if self.eat(&Tok::Eq) { Some(self.expr()?) } else { None };
                StmtKind::Let { name, init }
            }
            Tok::Fn if matches!(self.peek_at(1), Tok::Ident(_)) => StmtKind::Fn(self.fn_decl()?),
            Tok::Struct => StmtKind::Struct(self.struct_decl()?),
            Tok::Impl => self.impl_decl()?,
            Tok::While => {
                self.advance();
                let cond = self.expr()?;
                let body = self.loop_body()?;
                StmtKind::While { cond, body }
            }
            Tok::For => {
                self.advance();
                let first = self.ident("after 'for'")?;
                let (index, var) = if self.eat(&Tok::Comma) {
                    (Some(first), self.ident("after ',' in 'for'")?)
                } else {
                    (None, first)
                };
                if index.as_ref() == Some(&var) {
                    return Err(Self::err_at(pos, format!("duplicate loop variable '{var}'")));
                }
                self.expect(&Tok::In, "in 'for' loop")?;
                let iter = self.expr()?;
                let body = self.loop_body()?;
                StmtKind::For { index, var, iter, body }
            }
            Tok::Break | Tok::Continue => {
                let tok = self.advance();
                if self.loop_depth == 0 {
                    return Err(Self::err_at(pos, format!("{} outside of loop", tok.tok.describe())));
                }
                if tok.tok == Tok::Break { StmtKind::Break } else { StmtKind::Continue }
            }
            Tok::Return => {
                self.advance();
                if self.fn_depth == 0 {
                    return Err(Self::err_at(pos, "'return' outside of function"));
                }
                let value = match self.peek() {
                    Tok::Newline | Tok::Semi | Tok::RBrace | Tok::Eof => None,
                    _ => Some(self.expr()?),
                };
                StmtKind::Return(value)
            }
            _ => self.expr_or_assign()?,
        };
        Ok(Stmt { kind, pos })
    }

    fn expr_or_assign(&mut self) -> Result<StmtKind> {
        let target = self.expr()?;
        let op = match self.peek() {
            Tok::Eq => None,
            Tok::PlusEq => Some(BinOp::Add),
            Tok::MinusEq => Some(BinOp::Sub),
            Tok::StarEq => Some(BinOp::Mul),
            Tok::SlashEq => Some(BinOp::Div),
            Tok::PercentEq => Some(BinOp::Mod),
            _ => return Ok(StmtKind::Expr(target)),
        };
        let op_pos = self.pos();
        self.advance();
        if !matches!(target.kind, ExprKind::Ident(_) | ExprKind::Index(..) | ExprKind::Field(..)) {
            return Err(Self::err_at(op_pos, "invalid assignment target"));
        }
        let value = self.expr()?;
        Ok(StmtKind::Assign { target, op, value })
    }

    fn params(&mut self) -> Result<Vec<String>> {
        self.expect(&Tok::LParen, "before parameters")?;
        let mut params: Vec<String> = Vec::new();
        while !self.at(&Tok::RParen) {
            let pos = self.pos();
            let name = self.ident("in parameter list")?;
            if params.contains(&name) {
                return Err(Self::err_at(pos, format!("duplicate parameter '{name}'")));
            }
            params.push(name);
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RParen, "after parameters")?;
        Ok(params)
    }

    /// 函数体：进入新的函数层，循环层清零（循环内的匿名函数里不能 `break`）。
    fn fn_body(&mut self, lambda_arrow: bool) -> Result<Block> {
        let saved_loop = std::mem::replace(&mut self.loop_depth, 0);
        self.fn_depth += 1;
        let body = if lambda_arrow {
            let pos = self.pos();
            self.expr().map(|e| Block { stmts: vec![Stmt { pos: e.pos, kind: StmtKind::Expr(e) }], pos })
        } else {
            self.block()
        };
        self.fn_depth -= 1;
        self.loop_depth = saved_loop;
        body
    }

    fn fn_decl(&mut self) -> Result<Rc<FnDecl>> {
        let pos = self.pos();
        self.expect(&Tok::Fn, "")?;
        let name = self.ident("after 'fn'")?;
        let params = self.params()?;
        let body = self.fn_body(false)?;
        Ok(Rc::new(FnDecl { name: Some(name), params, body, pos }))
    }

    fn struct_decl(&mut self) -> Result<Rc<StructDecl>> {
        let pos = self.pos();
        self.advance();
        let name = self.ident("after 'struct'")?;
        self.expect(&Tok::LBrace, "after struct name")?;
        let mut fields: Vec<(String, Option<Expr>)> = Vec::new();
        self.skip_newlines();
        while !self.at(&Tok::RBrace) {
            let fpos = self.pos();
            let field = self.ident("in struct fields")?;
            if fields.iter().any(|(f, _)| *f == field) {
                return Err(Self::err_at(fpos, format!("duplicate field '{field}'")));
            }
            let default = if self.eat(&Tok::Eq) { Some(self.expr()?) } else { None };
            fields.push((field, default));
            if !self.eat(&Tok::Comma) && !matches!(self.peek(), Tok::Newline | Tok::RBrace) {
                return Err(self.err_here(format!("expected ',' or '}}' in struct fields, found {}", self.peek().describe())));
            }
            self.skip_newlines();
        }
        self.advance();
        Ok(Rc::new(StructDecl { name, fields, pos }))
    }

    fn impl_decl(&mut self) -> Result<StmtKind> {
        self.advance();
        let name = self.ident("after 'impl'")?;
        self.expect(&Tok::LBrace, "after impl name")?;
        let mut methods = Vec::new();
        self.skip_newlines();
        while !self.at(&Tok::RBrace) {
            if !(self.at(&Tok::Fn) && matches!(self.peek_at(1), Tok::Ident(_))) {
                return Err(self.err_here(format!("expected method declaration in impl, found {}", self.peek().describe())));
            }
            methods.push(self.fn_decl()?);
            self.end_stmt()?;
            self.skip_newlines();
        }
        self.advance();
        Ok(StmtKind::Impl { name, methods })
    }

    // ---------- 表达式 ----------

    fn expr(&mut self) -> Result<Expr> {
        self.or_expr()
    }

    /// 管道的优先级低于区间、高于比较：`1..9 |> f() > 2` 即 `f(1..9) > 2`。
    fn pipe(&mut self) -> Result<Expr> {
        let mut lhs = self.range()?;
        while self.at(&Tok::Pipe) {
            let pos = self.pos();
            self.advance();
            let rhs = self.postfix()?;
            lhs = Self::desugar_pipe(lhs, rhs, pos)?;
        }
        Ok(lhs)
    }

    /// `x |> f` → `x.f()`；`x |> f(a)` → `x.f(a)`；`x |> o.m(a)` → `o.m(x, a)`。
    /// 前两种走统一调用语法，因此结构体方法也能用管道调用。
    fn desugar_pipe(lhs: Expr, rhs: Expr, pos: Pos) -> Result<Expr> {
        let mut rhs_pos = rhs.pos;
        let kind = match rhs.kind {
            ExprKind::Ident(name) => ExprKind::MethodCall { obj: Box::new(lhs), name, args: Vec::new() },
            ExprKind::Call { callee, args } if matches!(callee.kind, ExprKind::Ident(_)) => {
                // 报错位置指向函数名
                rhs_pos = callee.pos;
                let ExprKind::Ident(name) = callee.kind else { unreachable!() };
                ExprKind::MethodCall { obj: Box::new(lhs), name, args }
            }
            ExprKind::Field(obj, name) => ExprKind::MethodCall { obj, name, args: vec![Arg { name: None, value: lhs }] },
            ExprKind::Call { callee, mut args } => {
                args.insert(0, Arg { name: None, value: lhs });
                ExprKind::Call { callee, args }
            }
            ExprKind::MethodCall { obj, name, mut args } => {
                args.insert(0, Arg { name: None, value: lhs });
                ExprKind::MethodCall { obj, name, args }
            }
            _ => return Err(Self::err_at(pos, "right side of '|>' must be a function name or call")),
        };
        Ok(Expr { kind, pos: rhs_pos })
    }

    fn binary(op: BinOp, lhs: Expr, rhs: Expr, pos: Pos) -> Expr {
        Expr { kind: ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)), pos }
    }

    fn or_expr(&mut self) -> Result<Expr> {
        let mut lhs = self.and_expr()?;
        while self.at(&Tok::Or) {
            let pos = self.pos();
            self.advance();
            let rhs = self.and_expr()?;
            lhs = Self::binary(BinOp::Or, lhs, rhs, pos);
        }
        Ok(lhs)
    }

    fn and_expr(&mut self) -> Result<Expr> {
        let mut lhs = self.equality()?;
        while self.at(&Tok::And) {
            let pos = self.pos();
            self.advance();
            let rhs = self.equality()?;
            lhs = Self::binary(BinOp::And, lhs, rhs, pos);
        }
        Ok(lhs)
    }

    fn equality_op(tok: &Tok) -> Option<BinOp> {
        match tok {
            Tok::EqEq => Some(BinOp::Eq),
            Tok::NotEq => Some(BinOp::Ne),
            _ => None,
        }
    }

    fn comparison_op(tok: &Tok) -> Option<BinOp> {
        match tok {
            Tok::Lt => Some(BinOp::Lt),
            Tok::Le => Some(BinOp::Le),
            Tok::Gt => Some(BinOp::Gt),
            Tok::Ge => Some(BinOp::Ge),
            _ => None,
        }
    }

    fn no_chain(&self, is_op: fn(&Tok) -> Option<BinOp>) -> Result<()> {
        if is_op(self.peek()).is_some() {
            return Err(self.err_here("comparison operators cannot be chained, use 'and'"));
        }
        Ok(())
    }

    fn equality(&mut self) -> Result<Expr> {
        let lhs = self.comparison()?;
        let Some(op) = Self::equality_op(self.peek()) else { return Ok(lhs) };
        let pos = self.pos();
        self.advance();
        let rhs = self.comparison()?;
        self.no_chain(Self::equality_op)?;
        self.no_chain(Self::comparison_op)?;
        Ok(Self::binary(op, lhs, rhs, pos))
    }

    fn comparison(&mut self) -> Result<Expr> {
        let lhs = self.pipe()?;
        let Some(op) = Self::comparison_op(self.peek()) else { return Ok(lhs) };
        let pos = self.pos();
        self.advance();
        let rhs = self.pipe()?;
        self.no_chain(Self::comparison_op)?;
        Ok(Self::binary(op, lhs, rhs, pos))
    }

    fn range(&mut self) -> Result<Expr> {
        let start = self.additive()?;
        let inclusive = match self.peek() {
            Tok::DotDot => false,
            Tok::DotDotEq => true,
            _ => return Ok(start),
        };
        let pos = self.pos();
        self.advance();
        let end = self.additive()?;
        if matches!(self.peek(), Tok::DotDot | Tok::DotDotEq) {
            return Err(self.err_here("range operators cannot be chained"));
        }
        Ok(Expr { kind: ExprKind::Range { start: Box::new(start), end: Box::new(end), inclusive }, pos })
    }

    fn additive(&mut self) -> Result<Expr> {
        let mut lhs = self.term()?;
        loop {
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => return Ok(lhs),
            };
            let pos = self.pos();
            self.advance();
            let rhs = self.term()?;
            lhs = Self::binary(op, lhs, rhs, pos);
        }
    }

    fn term(&mut self) -> Result<Expr> {
        let mut lhs = self.unary()?;
        loop {
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Mod,
                _ => return Ok(lhs),
            };
            let pos = self.pos();
            self.advance();
            let rhs = self.unary()?;
            lhs = Self::binary(op, lhs, rhs, pos);
        }
    }

    fn unary(&mut self) -> Result<Expr> {
        let op = match self.peek() {
            Tok::Minus => UnOp::Neg,
            Tok::Not => UnOp::Not,
            _ => return self.power(),
        };
        let pos = self.pos();
        self.advance();
        let operand = self.unary()?;
        Ok(Expr { kind: ExprKind::Unary(op, Box::new(operand)), pos })
    }

    fn power(&mut self) -> Result<Expr> {
        let base = self.postfix()?;
        if !self.at(&Tok::StarStar) {
            return Ok(base);
        }
        let pos = self.pos();
        self.advance();
        let exp = self.unary()?;
        Ok(Self::binary(BinOp::Pow, base, exp, pos))
    }

    fn postfix(&mut self) -> Result<Expr> {
        let mut e = self.primary()?;
        loop {
            let pos = self.pos();
            match self.peek() {
                Tok::LParen => {
                    let args = self.args()?;
                    e = Expr { kind: ExprKind::Call { callee: Box::new(e), args }, pos };
                }
                Tok::LBracket => {
                    self.advance();
                    let index = self.expr()?;
                    self.expect(&Tok::RBracket, "after index")?;
                    e = Expr { kind: ExprKind::Index(Box::new(e), Box::new(index)), pos };
                }
                Tok::Dot => {
                    self.advance();
                    let name = self.ident("after '.'")?;
                    if self.at(&Tok::LParen) {
                        let args = self.args()?;
                        e = Expr { kind: ExprKind::MethodCall { obj: Box::new(e), name, args }, pos };
                    } else {
                        e = Expr { kind: ExprKind::Field(Box::new(e), name), pos };
                    }
                }
                _ => return Ok(e),
            }
        }
    }

    fn args(&mut self) -> Result<Vec<Arg>> {
        self.expect(&Tok::LParen, "")?;
        let mut args: Vec<Arg> = Vec::new();
        while !self.at(&Tok::RParen) {
            let pos = self.pos();
            let name = match (self.peek().clone(), self.peek_at(1)) {
                (Tok::Ident(n), Tok::Colon) => {
                    self.advance();
                    self.advance();
                    Some(n)
                }
                _ => None,
            };
            if let Some(prev) = args.last()
                && prev.name.is_some() != name.is_some()
            {
                return Err(Self::err_at(pos, "cannot mix positional and named arguments"));
            }
            if let Some(n) = &name
                && args.iter().any(|a| a.name.as_ref() == Some(n))
            {
                return Err(Self::err_at(pos, format!("duplicate named argument '{n}'")));
            }
            let value = self.expr()?;
            args.push(Arg { name, value });
            if !self.eat(&Tok::Comma) {
                break;
            }
        }
        self.expect(&Tok::RParen, "after arguments")?;
        Ok(args)
    }

    fn primary(&mut self) -> Result<Expr> {
        let pos = self.pos();
        let kind = match self.peek().clone() {
            Tok::Int(n) => {
                self.advance();
                ExprKind::Int(n)
            }
            Tok::Float(x) => {
                self.advance();
                ExprKind::Float(x)
            }
            Tok::Str(parts) => {
                self.advance();
                self.string(parts)?
            }
            Tok::True => {
                self.advance();
                ExprKind::Bool(true)
            }
            Tok::False => {
                self.advance();
                ExprKind::Bool(false)
            }
            Tok::Nil => {
                self.advance();
                ExprKind::Nil
            }
            Tok::Ident(name) => {
                self.advance();
                ExprKind::Ident(name)
            }
            Tok::LParen => {
                self.advance();
                let inner = self.expr()?;
                self.expect(&Tok::RParen, "after expression")?;
                return Ok(inner);
            }
            Tok::LBracket => {
                self.advance();
                let mut items = Vec::new();
                while !self.at(&Tok::RBracket) {
                    items.push(self.expr()?);
                    if !self.eat(&Tok::Comma) {
                        break;
                    }
                }
                self.expect(&Tok::RBracket, "after array elements")?;
                ExprKind::Array(items)
            }
            Tok::Fn => {
                self.advance();
                if !self.at(&Tok::LParen) {
                    return Err(self.err_here("expected '(' after 'fn' in anonymous function"));
                }
                let params = self.params()?;
                let arrow = self.eat(&Tok::FatArrow);
                let body = self.fn_body(arrow)?;
                ExprKind::Lambda(Rc::new(FnDecl { name: None, params, body, pos }))
            }
            Tok::If => return self.if_expr(),
            Tok::Match => return self.match_expr(),
            Tok::LBrace => ExprKind::Block(self.block()?),
            other => return Err(self.err_here(format!("expected expression, found {}", other.describe()))),
        };
        Ok(Expr { kind, pos })
    }

    fn string(&mut self, parts: Vec<StrPart>) -> Result<ExprKind> {
        if let [StrPart::Lit(s)] = parts.as_slice() {
            return Ok(ExprKind::Str(s.clone()));
        }
        let mut out = Vec::new();
        for part in parts {
            match part {
                StrPart::Lit(s) => out.push(InterpPart::Lit(s)),
                StrPart::Expr(toks) => {
                    // 插值内部与外层处于同一个循环 / 函数中
                    let mut sub = Parser::new(toks);
                    sub.loop_depth = self.loop_depth;
                    sub.fn_depth = self.fn_depth;
                    let e = sub.expr()?;
                    if !sub.at(&Tok::Eof) {
                        return Err(sub.err_here(format!("unexpected {} in interpolation", sub.peek().describe())));
                    }
                    out.push(InterpPart::Expr(e));
                }
            }
        }
        Ok(ExprKind::Interp(out))
    }

    fn if_expr(&mut self) -> Result<Expr> {
        let pos = self.pos();
        self.advance();
        let cond = self.expr()?;
        let then = self.block()?;
        // `else` 可以位于 `}` 的下一行
        let mut k = 0;
        while matches!(self.peek_at(k), Tok::Newline) {
            k += 1;
        }
        let else_ = if matches!(self.peek_at(k), Tok::Else) {
            for _ in 0..=k {
                self.advance();
            }
            if self.at(&Tok::If) {
                Some(Box::new(self.if_expr()?))
            } else {
                let bpos = self.pos();
                Some(Box::new(Expr { kind: ExprKind::Block(self.block()?), pos: bpos }))
            }
        } else {
            None
        };
        Ok(Expr { kind: ExprKind::If { cond: Box::new(cond), then, else_ }, pos })
    }

    fn match_expr(&mut self) -> Result<Expr> {
        let pos = self.pos();
        self.advance();
        let subject = self.expr()?;
        self.expect(&Tok::LBrace, "after match subject")?;
        let mut arms = Vec::new();
        self.skip_newlines();
        while !self.at(&Tok::RBrace) {
            if self.at(&Tok::Eof) {
                return Err(Self::err_at(pos, "unclosed 'match', expected '}'"));
            }
            let pattern = self.pattern()?;
            let guard = if self.eat(&Tok::If) { Some(self.expr()?) } else { None };
            self.expect(&Tok::FatArrow, "after match pattern")?;
            let body = self.expr()?;
            arms.push(Arm { pattern, guard, body });
            if !self.eat(&Tok::Comma) && !matches!(self.peek(), Tok::Newline | Tok::Semi | Tok::RBrace) {
                return Err(self.err_here(format!("expected ',' or new line after match arm, found {}", self.peek().describe())));
            }
            self.skip_newlines();
        }
        self.advance();
        Ok(Expr { kind: ExprKind::Match { subject: Box::new(subject), arms }, pos })
    }

    // ---------- 模式 ----------

    fn pattern(&mut self) -> Result<Pattern> {
        let first = self.single_pattern()?;
        if !self.at(&Tok::Bar) {
            return Ok(first);
        }
        let mut alts = vec![first];
        while self.eat(&Tok::Bar) {
            alts.push(self.single_pattern()?);
        }
        Ok(Pattern::Or(alts))
    }

    /// 模式里的整数，允许负号。
    fn pattern_int(&mut self) -> Result<i64> {
        let neg = self.eat(&Tok::Minus);
        match self.peek().clone() {
            Tok::Int(n) => {
                self.advance();
                Ok(if neg { -n } else { n })
            }
            other => Err(self.err_here(format!("expected integer in range pattern, found {}", other.describe()))),
        }
    }

    fn single_pattern(&mut self) -> Result<Pattern> {
        match self.peek().clone() {
            Tok::DotDot | Tok::DotDotEq => {
                let inclusive = self.advance().tok == Tok::DotDotEq;
                let hi = self.pattern_int()?;
                Ok(Pattern::Range { lo: None, hi: Some(hi), inclusive })
            }
            Tok::Minus | Tok::Int(_) => {
                if self.at(&Tok::Minus) && matches!(self.peek_at(1), Tok::Float(_)) {
                    self.advance();
                    let Tok::Float(x) = self.advance().tok else { unreachable!() };
                    return Ok(Pattern::Float(-x));
                }
                let lo = self.pattern_int()?;
                match self.peek() {
                    Tok::DotDotEq => {
                        self.advance();
                        let hi = self.pattern_int()?;
                        Ok(Pattern::Range { lo: Some(lo), hi: Some(hi), inclusive: true })
                    }
                    Tok::DotDot => {
                        self.advance();
                        let hi = if matches!(self.peek(), Tok::Int(_) | Tok::Minus) { Some(self.pattern_int()?) } else { None };
                        Ok(Pattern::Range { lo: Some(lo), hi, inclusive: false })
                    }
                    _ => Ok(Pattern::Int(lo)),
                }
            }
            Tok::Float(x) => {
                self.advance();
                Ok(Pattern::Float(x))
            }
            Tok::Str(parts) => match parts.as_slice() {
                [StrPart::Lit(s)] => {
                    let s = s.clone();
                    self.advance();
                    Ok(Pattern::Str(s))
                }
                _ => Err(self.err_here("interpolated strings are not allowed in patterns")),
            },
            Tok::True => {
                self.advance();
                Ok(Pattern::Bool(true))
            }
            Tok::False => {
                self.advance();
                Ok(Pattern::Bool(false))
            }
            Tok::Nil => {
                self.advance();
                Ok(Pattern::Nil)
            }
            Tok::Ident(name) => {
                self.advance();
                Ok(if name == "_" { Pattern::Wildcard } else { Pattern::Bind(name) })
            }
            other => Err(self.err_here(format!("expected pattern, found {}", other.describe()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ast(src: &str) -> String {
        match parse(src) {
            Ok(prog) => sexpr_program(&prog),
            Err(e) => panic!("parse failed: {e}"),
        }
    }

    fn err(src: &str) -> String {
        parse(src).unwrap_err().to_string()
    }

    #[test]
    fn precedence() {
        assert_eq!(ast("1 + 2 * 3"), "(+ 1 (* 2 3))");
        assert_eq!(ast("-2 ** 2"), "(- (** 2 2))");
        assert_eq!(ast("2 ** 3 ** 2"), "(** 2 (** 3 2))");
        assert_eq!(ast("a or b and not c"), "(or a (and b (not c)))");
        assert_eq!(ast("0..n - 1"), "(.. 0 (- n 1))");
        assert_eq!(ast("a + 1 == b * 2"), "(== (+ a 1) (* b 2))");
        assert_eq!(ast("x < 1 == true"), "(== (< x 1) true)");
        assert_eq!(ast("(1 + 2) * 3"), "(* (+ 1 2) 3)");
    }

    #[test]
    fn postfix_and_calls() {
        assert_eq!(ast("f(1, 2)(3)"), "(call (call f 1 2) 3)");
        assert_eq!(ast("a[0][1]"), "(index (index a 0) 1)");
        assert_eq!(ast("p.x.y"), "(field (field p x) y)");
        assert_eq!(ast("arr.push(1)"), "(method arr push 1)");
        assert_eq!(ast("(s.f)()"), "(call (field s f))");
        assert_eq!(ast("Point(x: 1, y: 2)"), "(call Point (x: 1) (y: 2))");
        assert_eq!(ast("s[0..9]"), "(index s (.. 0 9))");
    }

    #[test]
    fn pipes_desugar() {
        assert_eq!(ast("x |> f"), "(method x f)");
        assert_eq!(ast("x |> f(1) |> g()"), "(method (method x f 1) g)");
        assert_eq!(ast("x |> o.m(1)"), "(method o m x 1)");
        assert_eq!(ast("x |> mk(1)(2)"), "(call (call mk 1) x 2)");
        assert_eq!(ast("[1, 2] |> len() > 2"), "(> (method (array 1 2) len) 2)");
        assert_eq!(ast("a + b |> f()"), "(method (+ a b) f)");
        assert_eq!(ast("1..9 |> f()"), "(method (.. 1 9) f)");
        assert_eq!(ast("a and b |> f()"), "(and a (method b f))");
        assert_eq!(ast("xs\n  |> map(fn(x) => x * 2)\n  |> sum()"), "(method (method xs map (fn (x) (block (* x 2)))) sum)");
    }

    #[test]
    fn statements() {
        assert_eq!(ast("let x = 1\nlet y"), "(let x 1)\n(let y)");
        assert_eq!(ast("a[i] += 2"), "(+= (index a i) 2)");
        assert_eq!(ast("p.x = 3; q = 4"), "(= (field p x) 3)\n(= q 4)");
        assert_eq!(ast("while i < 3 { i += 1 }"), "(while (< i 3) (block (+= i 1)))");
        assert_eq!(ast("for i, x in xs { break }"), "(for (i x) xs (block (break)))");
        assert_eq!(ast("for c in \"ab\" { continue }"), "(for c \"ab\" (block (continue)))");
    }

    #[test]
    fn functions() {
        assert_eq!(ast("fn add(a, b) { a + b }"), "(fn add (a b) (block (+ a b)))");
        assert_eq!(ast("fn f() { return }"), "(fn f () (block (return)))");
        assert_eq!(ast("let d = fn(x) => x * 2"), "(let d (fn (x) (block (* x 2))))");
        assert_eq!(ast("fn mk() {\n  let n = 0\n  fn() { n += 1; n }\n}"), "(fn mk () (block (let n 0) (fn () (block (+= n 1) n))))");
    }

    #[test]
    fn structs_and_impl() {
        assert_eq!(ast("struct P { x, y = 0 }"), "(struct P x (y 0))");
        assert_eq!(ast("struct P {\n  x,\n  y\n}"), "(struct P x y)");
        assert_eq!(
            ast("impl P {\n  fn len2(self) { self.x ** 2 }\n}"),
            "(impl P (fn len2 (self) (block (** (field self x) 2))))"
        );
    }

    #[test]
    fn if_and_match() {
        assert_eq!(ast("if a { 1 } else if b { 2 } else { 3 }"), "(if a (block 1) (if b (block 2) (block 3)))");
        assert_eq!(ast("if a {\n  1\n}\nelse {\n  2\n}"), "(if a (block 1) (block 2))");
        assert_eq!(ast("if a { 1 }\nx"), "(if a (block 1))\nx");
        assert_eq!(
            ast("match s {\n  100 => \"a\",\n  90..100 => \"b\"\n  0 | 1 => \"c\",\n  n if n < 0 => \"d\",\n  ..=-5 => \"e\",\n  60.. => \"f\",\n  _ => { \"g\" },\n}"),
            "(match s (arm 100 \"a\") (arm 90..100 \"b\") (arm (| 0 1) \"c\") (arm n (if (< n 0)) \"d\") (arm ..=-5 \"e\") (arm 60.. \"f\") (arm _ (block \"g\")))"
        );
    }

    #[test]
    fn interpolation() {
        assert_eq!(ast("\"a{x + 1}b\""), "(interp \"a\" (+ x 1) \"b\")");
        assert_eq!(ast("\"{p.name}\""), "(interp (field p name))");
        assert_eq!(ast("while true { \"{ if a { break } }\" }"), "(while true (block (interp (if a (block (break))))))");
    }

    #[test]
    fn chinese_keywords() {
        assert_eq!(ast("函数 f(n) { 如果 n < 2 { 返回 n } }"), ast("fn f(n) { if n < 2 { return n } }"));
        assert_eq!(ast("对于 i 在 0..3 { 跳出 }"), "(for i (.. 0 3) (block (break)))");
    }

    #[test]
    fn syntax_errors() {
        assert_eq!(err("break"), "[line 1:1] SyntaxError: 'break' outside of loop");
        assert_eq!(err("return 1"), "[line 1:1] SyntaxError: 'return' outside of function");
        assert_eq!(err("while true { let f = fn() { break } }"), "[line 1:29] SyntaxError: 'break' outside of loop");
        assert_eq!(err("a < b < c"), "[line 1:7] SyntaxError: comparison operators cannot be chained, use 'and'");
        assert_eq!(err("f() = 1"), "[line 1:5] SyntaxError: invalid assignment target");
        assert_eq!(err("let x = 1 2"), "[line 1:11] SyntaxError: expected end of statement, found number '2'");
        assert_eq!(err("fn f(a, a) {}"), "[line 1:9] SyntaxError: duplicate parameter 'a'");
        assert_eq!(err("P(x: 1, 2)"), "[line 1:9] SyntaxError: cannot mix positional and named arguments");
        assert_eq!(err("x |> 1"), "[line 1:3] SyntaxError: right side of '|>' must be a function name or call");
        assert_eq!(err("if x {\n  1\n"), "[line 1:6] SyntaxError: unclosed '{', expected '}'");
        assert_eq!(err("let = 1"), "[line 1:5] SyntaxError: expected identifier after 'let', found '='");
        assert_eq!(err("\"{1 2}\""), "[line 1:5] SyntaxError: unexpected number '2' in interpolation");
        assert_eq!(err("}"), "[line 1:1] SyntaxError: unexpected '}'");
    }
}
