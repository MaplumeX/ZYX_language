//! 统一的错误类型。格式固定为 `[line 行:列] 类别: 信息`（见 reference.md 第 9 节）。

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Syntax,
    Name,
    Type,
    Index,
    Value,
    Assertion,
    Runtime,
}

impl ErrorKind {
    pub fn name(self) -> &'static str {
        match self {
            ErrorKind::Syntax => "SyntaxError",
            ErrorKind::Name => "NameError",
            ErrorKind::Type => "TypeError",
            ErrorKind::Index => "IndexError",
            ErrorKind::Value => "ValueError",
            ErrorKind::Assertion => "AssertionError",
            ErrorKind::Runtime => "RuntimeError",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ZyxError {
    pub kind: ErrorKind,
    pub msg: String,
    pub line: u32,
    pub col: u32,
}

impl ZyxError {
    pub fn new(kind: ErrorKind, line: u32, col: u32, msg: impl Into<String>) -> Self {
        ZyxError {
            kind,
            msg: msg.into(),
            line,
            col,
        }
    }

    pub fn syntax(line: u32, col: u32, msg: impl Into<String>) -> Self {
        Self::new(ErrorKind::Syntax, line, col, msg)
    }
}

impl fmt::Display for ZyxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[line {}:{}] {}: {}",
            self.line,
            self.col,
            self.kind.name(),
            self.msg
        )
    }
}

pub type Result<T> = std::result::Result<T, ZyxError>;
