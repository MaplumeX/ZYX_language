//! 命令行入口：`zyx [--backend tree|vm] [--ast | --dis] <file.zyx>`

use std::process::exit;

use zyx::ast::sexpr_program;
use zyx::interp::Interp;
use zyx::vm::Vm;
use zyx::{parser, resolver};

/// 退出码见 reference.md 9.3。
const EXIT_USAGE: i32 = 64;
const EXIT_COMPILE: i32 = 65;
const EXIT_RUNTIME: i32 = 70;

/// 树遍历后端每层 ZYX 调用会占用多层 Rust 栈帧（虚拟机后端在内置函数回调时也会），
/// 在大栈线程中运行以支撑 10000 层调用深度。
const STACK_SIZE: usize = 1 << 30;

const USAGE: &str = "usage: zyx [--backend tree|vm] [--ast | --dis] <file.zyx>";

fn main() {
    let worker = std::thread::Builder::new().stack_size(STACK_SIZE).spawn(run_cli);
    let code = match worker.map(|h| h.join()) {
        Ok(Ok(code)) => code,
        _ => EXIT_RUNTIME,
    };
    exit(code);
}

fn run_cli() -> i32 {
    let mut backend = String::from("tree");
    let mut dump_ast = false;
    let mut dump_bytecode = false;
    let mut path = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--backend" => match args.next() {
                Some(b) if b == "tree" || b == "vm" => backend = b,
                _ => usage_error("--backend expects 'tree' or 'vm'"),
            },
            "--ast" => dump_ast = true,
            "--dis" => dump_bytecode = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return 0;
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
        return 0;
    }

    let ir = match resolver::resolve(&program) {
        Ok(ir) => ir,
        Err(e) => {
            eprintln!("{e}");
            exit(EXIT_COMPILE);
        }
    };

    if dump_bytecode {
        print!("{}", zyx::bytecode::disassemble(&ir, &zyx::bytecode::compile(&ir)));
        return 0;
    }

    let result = if backend == "vm" {
        let mut vm = Vm::new(&ir);
        let r = vm.run(&ir);
        vm.flush();
        r
    } else {
        let mut interp = Interp::new(&ir);
        let r = interp.run(&ir);
        interp.flush();
        r
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{e}");
            EXIT_RUNTIME
        }
    }
}

fn usage_error(msg: &str) -> ! {
    eprintln!("zyx: {msg}\n{USAGE}");
    exit(EXIT_USAGE);
}
