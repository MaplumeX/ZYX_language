//! 命令行入口：`zyx [--backend tree|vm] [--ast] <file.zyx>`

use std::process::exit;

use zyx::ast::sexpr_program;
use zyx::parser;

/// 退出码见 reference.md 9.3。
const EXIT_USAGE: i32 = 64;
const EXIT_COMPILE: i32 = 65;
const EXIT_RUNTIME: i32 = 70;

const USAGE: &str = "usage: zyx [--backend tree|vm] [--ast] <file.zyx>";

fn main() {
    let mut backend = String::from("tree");
    let mut dump_ast = false;
    let mut path = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--backend" => match args.next() {
                Some(b) if b == "tree" || b == "vm" => backend = b,
                _ => usage_error("--backend expects 'tree' or 'vm'"),
            },
            "--ast" => dump_ast = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            _ if arg.starts_with('-') => usage_error(&format!("unknown option '{arg}'")),
            _ if path.is_none() => path = Some(arg),
            _ => usage_error("only one script file can be given"),
        }
    }

    let Some(path) = path else { usage_error("missing script file") };
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("zyx: cannot read '{path}': {e}");
            exit(EXIT_USAGE);
        }
    };

    let program = match parser::parse(&src) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            exit(EXIT_COMPILE);
        }
    };

    if dump_ast {
        println!("{}", sexpr_program(&program));
        return;
    }

    eprintln!("zyx: the '{backend}' backend is not implemented yet");
    exit(EXIT_RUNTIME);
}

fn usage_error(msg: &str) -> ! {
    eprintln!("zyx: {msg}\n{USAGE}");
    exit(EXIT_USAGE);
}
