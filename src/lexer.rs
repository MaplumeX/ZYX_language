//! 词法分析：源码 → `Vec<Token>`。
//!
//! 换行规则（reference.md 1.7）在这里处理：只有"有意义"的换行才产生 `Tok::Newline`，
//! 语法分析器因此只需把 `Newline` / `Semi` 当作语句结束符。

use crate::error::{Result, ZyxError};
use crate::token::{StrPart, Tok, Token, keyword};

pub fn lex(src: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = src.chars().collect();
    Lexer::new(&chars, 0, chars.len(), 1, 1, false).run()
}

struct Lexer<'a> {
    chars: &'a [char],
    i: usize,
    end: usize,
    line: u32,
    col: u32,
    toks: Vec<Token>,
    /// 未闭合的 `(` `[` `{`。栈顶是 `(` 或 `[` 时换行被忽略。
    delims: Vec<char>,
}

/// 全角标点及其对应的半角写法。
const FULL_WIDTH: &[(char, &str)] = &[
    ('（', "("),
    ('）', ")"),
    ('，', ","),
    ('；', ";"),
    ('：', ":"),
    ('“', "\""),
    ('”', "\""),
    ('‘', "'"),
    ('’', "'"),
    ('【', "["),
    ('】', "]"),
    ('｛', "{"),
    ('｝', "}"),
    ('！', "!"),
    ('。', "."),
    ('＝', "="),
    ('＋', "+"),
    ('－', "-"),
    ('＊', "*"),
    ('／', "/"),
    ('＜', "<"),
    ('＞', ">"),
    ('｜', "|"),
];

impl<'a> Lexer<'a> {
    fn new(chars: &'a [char], start: usize, end: usize, line: u32, col: u32, nested: bool) -> Self {
        Lexer {
            chars,
            i: start,
            end,
            line,
            col,
            toks: Vec::new(),
            // 插值内部视作在括号里，换行不产生 Newline。
            delims: if nested { vec!['('] } else { Vec::new() },
        }
    }

    fn peek(&self, k: usize) -> Option<char> {
        let j = self.i + k;
        if j < self.end { Some(self.chars[j]) } else { None }
    }

    fn advance(&mut self) -> char {
        let c = self.chars[self.i];
        self.i += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        c
    }

    fn push(&mut self, tok: Tok, line: u32, col: u32) {
        self.toks.push(Token { tok, line, col });
    }

    fn err(&self, line: u32, col: u32, msg: impl Into<String>) -> ZyxError {
        ZyxError::syntax(line, col, msg)
    }

    fn run(mut self) -> Result<Vec<Token>> {
        while let Some(c) = self.peek(0) {
            let (line, col) = (self.line, self.col);
            match c {
                ' ' | '\t' | '\r' => {
                    self.advance();
                }
                '#' => {
                    while let Some(c) = self.peek(0) {
                        if c == '\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                '\n' => {
                    self.advance();
                    self.newline(line, col);
                }
                '0'..='9' => self.number()?,
                '"' => self.string()?,
                c if c.is_alphabetic() || c == '_' => self.ident(),
                _ => self.punct(c, line, col)?,
            }
        }
        let (line, col) = (self.line, self.col);
        self.push(Tok::Eof, line, col);
        Ok(self.toks)
    }

    fn newline(&mut self, line: u32, col: u32) {
        if matches!(self.delims.last(), Some('(') | Some('[')) {
            return;
        }
        match self.toks.last() {
            None => return,
            Some(t) if matches!(t.tok, Tok::Newline | Tok::Semi | Tok::LBrace) || t.tok.continues_line() => return,
            _ => {}
        }
        if self.next_line_continues() {
            return;
        }
        self.push(Tok::Newline, line, col);
    }

    /// 下一个非空、非注释行是否以 `|>` 或 `.`（不是 `..`）开头。
    fn next_line_continues(&self) -> bool {
        let mut j = self.i;
        while j < self.end {
            match self.chars[j] {
                ' ' | '\t' | '\r' | '\n' => j += 1,
                '#' => {
                    while j < self.end && self.chars[j] != '\n' {
                        j += 1;
                    }
                }
                _ => break,
            }
        }
        let at = |k: usize| {
            if j + k < self.end {
                Some(self.chars[j + k])
            } else {
                None
            }
        };
        match (at(0), at(1)) {
            (Some('|'), Some('>')) => true,
            (Some('.'), next) => next != Some('.'),
            _ => false,
        }
    }

    fn number(&mut self) -> Result<()> {
        let (line, col) = (self.line, self.col);
        let mut text = String::new();
        let tok;
        if self.peek(0) == Some('0') && matches!(self.peek(1), Some('x') | Some('X')) {
            self.advance();
            self.advance();
            while let Some(c) = self.peek(0) {
                if c.is_ascii_hexdigit() || c == '_' {
                    self.advance();
                    if c != '_' {
                        text.push(c);
                    }
                } else {
                    break;
                }
            }
            if text.is_empty() {
                return Err(self.err(line, col, "invalid hexadecimal literal"));
            }
            let n = i64::from_str_radix(&text, 16).map_err(|_| self.err(line, col, "integer literal is too large"))?;
            tok = Tok::Int(n);
        } else {
            let mut is_float = false;
            self.digits(&mut text);
            if self.peek(0) == Some('.') && self.peek(1).is_some_and(|c| c.is_ascii_digit()) {
                is_float = true;
                text.push(self.advance());
                self.digits(&mut text);
            }
            if matches!(self.peek(0), Some('e') | Some('E')) {
                let has_exp = match self.peek(1) {
                    Some(c) if c.is_ascii_digit() => true,
                    Some('+') | Some('-') => self.peek(2).is_some_and(|c| c.is_ascii_digit()),
                    _ => false,
                };
                if has_exp {
                    is_float = true;
                    text.push(self.advance());
                    if matches!(self.peek(0), Some('+') | Some('-')) {
                        text.push(self.advance());
                    }
                    self.digits(&mut text);
                }
            }
            tok = if is_float {
                Tok::Float(
                    text.parse()
                        .map_err(|_| self.err(line, col, "invalid number literal"))?,
                )
            } else {
                Tok::Int(
                    text.parse()
                        .map_err(|_| self.err(line, col, "integer literal is too large"))?,
                )
            };
        }
        if self.peek(0).is_some_and(|c| c.is_alphabetic() || c == '_') {
            return Err(self.err(line, col, "invalid number literal"));
        }
        self.push(tok, line, col);
        Ok(())
    }

    fn digits(&mut self, text: &mut String) {
        while let Some(c) = self.peek(0) {
            if c.is_ascii_digit() || c == '_' {
                self.advance();
                if c != '_' {
                    text.push(c);
                }
            } else {
                break;
            }
        }
    }

    fn string(&mut self) -> Result<()> {
        let (line, col) = (self.line, self.col);
        self.advance();
        let mut parts = Vec::new();
        let mut buf = String::new();
        loop {
            let Some(c) = self.peek(0) else {
                return Err(self.err(line, col, "unterminated string"));
            };
            match c {
                '"' => {
                    self.advance();
                    break;
                }
                '\n' => return Err(self.err(line, col, "unterminated string")),
                '\\' => {
                    let (el, ec) = (self.line, self.col);
                    self.advance();
                    let Some(e) = self.peek(0) else {
                        return Err(self.err(line, col, "unterminated string"));
                    };
                    let ch = match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '0' => '\0',
                        '\\' => '\\',
                        '"' => '"',
                        _ => return Err(self.err(el, ec, format!("invalid escape sequence '\\{e}'"))),
                    };
                    self.advance();
                    buf.push(ch);
                }
                '{' if self.peek(1) == Some('{') => {
                    self.advance();
                    self.advance();
                    buf.push('{');
                }
                '}' if self.peek(1) == Some('}') => {
                    self.advance();
                    self.advance();
                    buf.push('}');
                }
                '}' => {
                    return Err(self.err(
                        self.line,
                        self.col,
                        "single '}' in string, use '}}' for a literal brace",
                    ));
                }
                '{' => {
                    if !buf.is_empty() {
                        parts.push(StrPart::Lit(std::mem::take(&mut buf)));
                    }
                    parts.push(StrPart::Expr(self.interpolation()?));
                }
                _ => {
                    self.advance();
                    buf.push(c);
                }
            }
        }
        if !buf.is_empty() || parts.is_empty() {
            parts.push(StrPart::Lit(buf));
        }
        self.push(Tok::Str(parts), line, col);
        Ok(())
    }

    /// 当前位于插值的 `{`。返回内部表达式的词法单元，并越过对应的 `}`。
    fn interpolation(&mut self) -> Result<Vec<Token>> {
        let (open_line, open_col) = (self.line, self.col);
        self.advance();
        let start = self.i;
        let mut depth = 1;
        let mut j = start;
        let mut col = self.col;
        loop {
            if j >= self.end || self.chars[j] == '\n' {
                return Err(self.err(open_line, open_col, "unterminated interpolation in string"));
            }
            match self.chars[j] {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                '"' => {
                    return Err(self.err(self.line, col, "string literals are not allowed inside interpolation"));
                }
                _ => {}
            }
            j += 1;
            col += 1;
        }
        let toks = Lexer::new(self.chars, start, j, self.line, self.col, true).run()?;
        if toks.len() == 1 {
            return Err(self.err(open_line, open_col, "empty interpolation in string"));
        }
        while self.i <= j {
            self.advance();
        }
        Ok(toks)
    }

    fn ident(&mut self) {
        let (line, col) = (self.line, self.col);
        let mut word = String::new();
        while let Some(c) = self.peek(0) {
            if c.is_alphanumeric() || c == '_' {
                word.push(self.advance());
            } else {
                break;
            }
        }
        let tok = keyword(&word).unwrap_or(Tok::Ident(word));
        self.push(tok, line, col);
    }

    fn punct(&mut self, c: char, line: u32, col: u32) -> Result<()> {
        if let Some((_, ascii)) = FULL_WIDTH.iter().find(|(fw, _)| *fw == c) {
            return Err(self.err(line, col, format!("full-width character '{c}', use '{ascii}' instead")));
        }
        self.advance();
        let next = self.peek(0);
        let two = |lexer: &mut Self, tok: Tok| {
            lexer.advance();
            tok
        };
        let tok = match c {
            '(' => {
                self.delims.push('(');
                Tok::LParen
            }
            ')' => {
                self.delims.pop();
                Tok::RParen
            }
            '[' => {
                self.delims.push('[');
                Tok::LBracket
            }
            ']' => {
                self.delims.pop();
                Tok::RBracket
            }
            '{' => {
                self.delims.push('{');
                Tok::LBrace
            }
            '}' => {
                self.delims.pop();
                Tok::RBrace
            }
            ',' => Tok::Comma,
            ':' => Tok::Colon,
            ';' => Tok::Semi,
            '.' if next == Some('.') => {
                self.advance();
                if self.peek(0) == Some('=') {
                    two(self, Tok::DotDotEq)
                } else {
                    Tok::DotDot
                }
            }
            '.' => Tok::Dot,
            '+' if next == Some('=') => two(self, Tok::PlusEq),
            '+' => Tok::Plus,
            '-' if next == Some('=') => two(self, Tok::MinusEq),
            '-' => Tok::Minus,
            '*' if next == Some('*') => two(self, Tok::StarStar),
            '*' if next == Some('=') => two(self, Tok::StarEq),
            '*' => Tok::Star,
            '/' if next == Some('=') => two(self, Tok::SlashEq),
            '/' => Tok::Slash,
            '%' if next == Some('=') => two(self, Tok::PercentEq),
            '%' => Tok::Percent,
            '=' if next == Some('=') => two(self, Tok::EqEq),
            '=' if next == Some('>') => two(self, Tok::FatArrow),
            '=' => Tok::Eq,
            '!' if next == Some('=') => two(self, Tok::NotEq),
            '!' => return Err(self.err(line, col, "unexpected '!', use 'not' for logical negation")),
            '<' if next == Some('=') => two(self, Tok::Le),
            '<' => Tok::Lt,
            '>' if next == Some('=') => two(self, Tok::Ge),
            '>' => Tok::Gt,
            '|' if next == Some('>') => two(self, Tok::Pipe),
            '|' if next == Some('|') => return Err(self.err(line, col, "unexpected '||', use 'or'")),
            '|' => Tok::Bar,
            '&' if next == Some('&') => return Err(self.err(line, col, "unexpected '&&', use 'and'")),
            '\'' => return Err(self.err(line, col, "strings use double quotes \"...\"")),
            _ => return Err(self.err(line, col, format!("unexpected character '{c}'"))),
        };
        self.push(tok, line, col);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Tok> {
        lex(src).unwrap().into_iter().map(|t| t.tok).collect()
    }

    fn err(src: &str) -> String {
        lex(src).unwrap_err().to_string()
    }

    #[test]
    fn numbers() {
        assert_eq!(
            kinds("42 1_000 0xff"),
            vec![Tok::Int(42), Tok::Int(1000), Tok::Int(255), Tok::Eof]
        );
        assert_eq!(
            kinds("3.14 1.5e3 2e-2"),
            vec![Tok::Float(3.14), Tok::Float(1500.0), Tok::Float(0.02), Tok::Eof]
        );
        assert_eq!(kinds("1..5"), vec![Tok::Int(1), Tok::DotDot, Tok::Int(5), Tok::Eof]);
        assert_eq!(
            kinds("5.f"),
            vec![Tok::Int(5), Tok::Dot, Tok::Ident("f".into()), Tok::Eof]
        );
        assert!(err("3abc").contains("invalid number literal"));
        assert!(err("99999999999999999999").contains("too large"));
    }

    #[test]
    fn chinese_keywords_map_to_same_tokens() {
        assert_eq!(kinds("如果 真 否则 空"), kinds("if true else nil"));
        assert_eq!(
            kinds("令 总和 = 0"),
            vec![Tok::Let, Tok::Ident("总和".into()), Tok::Eq, Tok::Int(0), Tok::Eof]
        );
    }

    #[test]
    fn strings_and_interpolation() {
        assert_eq!(
            kinds(r#""a\n\"b""#),
            vec![Tok::Str(vec![StrPart::Lit("a\n\"b".into())]), Tok::Eof]
        );
        assert_eq!(
            kinds(r#""""#),
            vec![Tok::Str(vec![StrPart::Lit(String::new())]), Tok::Eof]
        );
        assert_eq!(
            kinds(r#""{{x}}""#),
            vec![Tok::Str(vec![StrPart::Lit("{x}".into())]), Tok::Eof]
        );
        let toks = kinds(r#""n={n + 1}!""#);
        let Tok::Str(parts) = &toks[0] else { panic!() };
        assert_eq!(parts.len(), 3);
        let StrPart::Expr(inner) = &parts[1] else { panic!() };
        let inner: Vec<Tok> = inner.iter().map(|t| t.tok.clone()).collect();
        assert_eq!(inner, vec![Tok::Ident("n".into()), Tok::Plus, Tok::Int(1), Tok::Eof]);
    }

    #[test]
    fn interpolation_positions_are_absolute() {
        let toks = lex("let s = \"ab{x}\"").unwrap();
        let Tok::Str(parts) = &toks[3].tok else { panic!() };
        let StrPart::Expr(inner) = &parts[1] else { panic!() };
        assert_eq!((inner[0].line, inner[0].col), (1, 13));
    }

    #[test]
    fn string_errors() {
        assert_eq!(err("\"abc"), "[line 1:1] SyntaxError: unterminated string");
        assert!(err(r#""a}""#).contains("use '}}'"));
        assert!(err(r#""{"x"}""#).contains("not allowed inside interpolation"));
        assert!(err(r#""{}""#).contains("empty interpolation"));
        assert!(err(r#""\q""#).contains("invalid escape"));
    }

    #[test]
    fn newline_rules() {
        use Tok::*;
        // 连续空行折叠，开头的换行忽略
        assert_eq!(
            kinds("\n\na\n\n\nb\n"),
            vec![Ident("a".into()), Newline, Ident("b".into()), Newline, Eof]
        );
        // 括号内忽略换行
        assert_eq!(
            kinds("f(1,\n2)"),
            vec![Ident("f".into()), LParen, Int(1), Comma, Int(2), RParen, Eof]
        );
        // 行尾运算符延续
        assert_eq!(kinds("a +\nb"), vec![Ident("a".into()), Plus, Ident("b".into()), Eof]);
        // 下一行以 |> 或 . 开头时延续
        assert_eq!(
            kinds("a\n  |> f\n"),
            vec![Ident("a".into()), Pipe, Ident("f".into()), Newline, Eof]
        );
        assert_eq!(
            kinds("a\n # c\n .f\n"),
            vec![Ident("a".into()), Dot, Ident("f".into()), Newline, Eof]
        );
        // 以 .. 开头的行（match 区间模式）不延续
        assert_eq!(
            kinds("a\n..b"),
            vec![Ident("a".into()), Newline, DotDot, Ident("b".into()), Eof]
        );
        // 括号内的花括号块里换行有意义
        assert_eq!(
            kinds("f(fn() {\na\nb\n})"),
            vec![
                Ident("f".into()),
                LParen,
                Fn,
                LParen,
                RParen,
                LBrace,
                Ident("a".into()),
                Newline,
                Ident("b".into()),
                Newline,
                RBrace,
                RParen,
                Eof
            ]
        );
    }

    #[test]
    fn helpful_errors() {
        assert_eq!(
            err("print（1）"),
            "[line 1:6] SyntaxError: full-width character '（', use '(' instead"
        );
        assert!(err("!x").contains("use 'not'"));
        assert!(err("a && b").contains("use 'and'"));
        assert!(err("'a'").contains("double quotes"));
    }

    #[test]
    fn positions() {
        let toks = lex("let x\n  y").unwrap();
        let pos: Vec<(u32, u32)> = toks.iter().map(|t| (t.line, t.col)).collect();
        assert_eq!(pos, vec![(1, 1), (1, 5), (1, 6), (2, 3), (2, 4)]);
    }
}
