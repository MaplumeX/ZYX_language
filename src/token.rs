//! 词法单元。中文关键字在词法阶段映射为与英文相同的 `Tok`（ADR-0003）。

#[derive(Debug, Clone, PartialEq)]
pub enum StrPart {
    Lit(String),
    /// 插值 `{...}` 内部的词法单元，以 `Tok::Eof` 结尾。
    Expr(Vec<Token>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Int(i64),
    Float(f64),
    Str(Vec<StrPart>),
    Ident(String),

    // 关键字
    Fn,
    Let,
    If,
    Else,
    While,
    For,
    In,
    Break,
    Continue,
    Return,
    Match,
    Struct,
    Impl,
    And,
    Or,
    Not,
    True,
    False,
    Nil,

    // 分隔符
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,
    Comma,
    Dot,
    Colon,
    Semi,
    Newline,

    // 运算符
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    StarStar,
    Eq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    EqEq,
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    DotDot,
    DotDotEq,
    Pipe,
    Bar,
    FatArrow,

    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
    pub col: u32,
}

pub fn keyword(word: &str) -> Option<Tok> {
    let tok = match word {
        "fn" | "函数" => Tok::Fn,
        "let" | "令" => Tok::Let,
        "if" | "如果" => Tok::If,
        "else" | "否则" => Tok::Else,
        "while" | "当" => Tok::While,
        "for" | "对于" => Tok::For,
        "in" | "在" => Tok::In,
        "break" | "跳出" => Tok::Break,
        "continue" | "继续" => Tok::Continue,
        "return" | "返回" => Tok::Return,
        "match" | "匹配" => Tok::Match,
        "struct" | "结构" => Tok::Struct,
        "impl" | "实现" => Tok::Impl,
        "and" | "且" => Tok::And,
        "or" | "或" => Tok::Or,
        "not" | "非" => Tok::Not,
        "true" | "真" => Tok::True,
        "false" | "假" => Tok::False,
        "nil" | "空" => Tok::Nil,
        _ => return None,
    };
    Some(tok)
}

impl Tok {
    /// 用于报错信息的描述。
    pub fn describe(&self) -> String {
        let s = match self {
            Tok::Int(n) => return format!("number '{n}'"),
            Tok::Float(x) => return format!("number '{x}'"),
            Tok::Str(_) => "string",
            Tok::Ident(name) => return format!("identifier '{name}'"),
            Tok::Newline => "end of line",
            Tok::Eof => "end of file",
            Tok::Fn => "'fn'",
            Tok::Let => "'let'",
            Tok::If => "'if'",
            Tok::Else => "'else'",
            Tok::While => "'while'",
            Tok::For => "'for'",
            Tok::In => "'in'",
            Tok::Break => "'break'",
            Tok::Continue => "'continue'",
            Tok::Return => "'return'",
            Tok::Match => "'match'",
            Tok::Struct => "'struct'",
            Tok::Impl => "'impl'",
            Tok::And => "'and'",
            Tok::Or => "'or'",
            Tok::Not => "'not'",
            Tok::True => "'true'",
            Tok::False => "'false'",
            Tok::Nil => "'nil'",
            Tok::LParen => "'('",
            Tok::RParen => "')'",
            Tok::LBracket => "'['",
            Tok::RBracket => "']'",
            Tok::LBrace => "'{'",
            Tok::RBrace => "'}'",
            Tok::Comma => "','",
            Tok::Dot => "'.'",
            Tok::Colon => "':'",
            Tok::Semi => "';'",
            Tok::Plus => "'+'",
            Tok::Minus => "'-'",
            Tok::Star => "'*'",
            Tok::Slash => "'/'",
            Tok::Percent => "'%'",
            Tok::StarStar => "'**'",
            Tok::Eq => "'='",
            Tok::PlusEq => "'+='",
            Tok::MinusEq => "'-='",
            Tok::StarEq => "'*='",
            Tok::SlashEq => "'/='",
            Tok::PercentEq => "'%='",
            Tok::EqEq => "'=='",
            Tok::NotEq => "'!='",
            Tok::Lt => "'<'",
            Tok::Le => "'<='",
            Tok::Gt => "'>'",
            Tok::Ge => "'>='",
            Tok::DotDot => "'..'",
            Tok::DotDotEq => "'..='",
            Tok::Pipe => "'|>'",
            Tok::Bar => "'|'",
            Tok::FatArrow => "'=>'",
        };
        s.to_string()
    }

    /// 行尾出现这些词法单元时，换行不结束语句（reference.md 1.7）。
    pub fn continues_line(&self) -> bool {
        matches!(
            self,
            Tok::Plus
                | Tok::Minus
                | Tok::Star
                | Tok::Slash
                | Tok::Percent
                | Tok::StarStar
                | Tok::Eq
                | Tok::PlusEq
                | Tok::MinusEq
                | Tok::StarEq
                | Tok::SlashEq
                | Tok::PercentEq
                | Tok::EqEq
                | Tok::NotEq
                | Tok::Lt
                | Tok::Le
                | Tok::Gt
                | Tok::Ge
                | Tok::DotDot
                | Tok::DotDotEq
                | Tok::Pipe
                | Tok::Bar
                | Tok::FatArrow
                | Tok::And
                | Tok::Or
                | Tok::Not
                | Tok::Comma
                | Tok::Dot
                | Tok::Colon
        )
    }
}
