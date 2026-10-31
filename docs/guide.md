# ZYX 开发指南

这份指南带你从零开始用 ZYX 写程序：构建解释器、写第一个程序、按主题学会全部语法，最后介绍调试、测试和项目结构。语言的完整规格见 [`reference.md`](reference.md)；给编程 Agent 用的精简版见 [`.claude/skills/zyx/SKILL.md`](../.claude/skills/zyx/SKILL.md)。

## 1. ZYX 是什么

ZYX 是一门动态类型、解释执行的脚本语言，语法采用花括号分块。它的特色：

- **管道运算符** `|>`：`xs |> filter(f) |> sort()`，数据处理从左读到右。
- **统一调用语法**：`a.f(b)` 就是 `f(a, b)`，任何函数都能写成方法链。
- **字符串插值**：`"你好，{name}！"`。
- **`match` 表达式**：支持区间、多选、变量绑定和守卫条件。
- **块表达式**：代码块的最后一个表达式就是它的值，函数可以不写 `return`。
- **中英文关键字**：`如果`/`if`、`函数`/`fn` 等，可以混用。
- **严格的布尔条件**：`if` 只接受 `true` / `false`，杜绝"0 被当成假"一类的隐蔽错误。

## 2. 构建与运行

需要较新的稳定版 Rust 工具链（项目使用 2024 edition，开发时用的是 Rust 1.97）和 Python 3（只用于测试与性能脚本）。

```sh
git clone <仓库地址> ZYX_language
cd ZYX_language
make build                  # 等价于 cargo build --release
./target/release/zyx --help
```

可执行文件在 `target/release/zyx`。如果想在任意目录直接用 `zyx`，可以把它加入 `PATH`：

```sh
export PATH="$PWD/target/release:$PATH"
```

命令行用法：

| 命令 | 作用 |
|---|---|
| `zyx 程序.zyx` | 运行程序（默认树遍历后端） |
| `zyx --backend vm 程序.zyx` | 用字节码虚拟机运行，通常快 1.4 到 4.8 倍，结果完全相同 |
| `zyx --ast 程序.zyx` | 打印语法树（S 表达式形式），排查运算优先级时有用 |
| `zyx --dis 程序.zyx` | 打印编译后的字节码 |

## 3. 第一个程序

新建 `hello.zyx`：

```zyx
let name = input("你叫什么名字？")
print("你好，{name}！")
```

运行：

```sh
$ zyx hello.zyx
你叫什么名字？小明
你好，小明！
```

`input` 打印提示语后读入一行，`print` 输出并换行，`{name}` 把变量插进字符串。

## 4. 语言教程

### 4.1 变量与类型

变量用 `let` 声明，之后才能赋值。用到未声明的变量时，程序在运行前就会报错，所以拼错变量名很容易发现。

```zyx
let count = 0          # int：64 位整数
let price = 9.5        # float
let title = "ZYX"      # str
let done = false       # bool
let nothing = nil      # 空值
count = count + 1      # 已声明，可以赋值
count += 2             # 复合赋值：+= -= *= /= %=
print(count, type(price), type(title))   # 3 float str
```

ZYX 有这些类型：`int`、`float`、`str`、`bool`、`nil`、`array`、`range`、`fn`，以及你自己定义的结构体。

### 4.2 运算

```zyx
print(1 + 2 * 3)       # 7
print(2 ** 10)         # 1024，乘方
print(7 / 2)           # 3   注意：两个整数相除向下取整
print(float(7) / 2)    # 3.5 想要小数时先转 float
print(-7 % 3)          # 2   取模的符号跟随除数
print(1 + 0.5)         # 1.5 整数与浮点混合得浮点
print(3 > 2 and not false)   # true
```

整数溢出、除以零都会报错，而不是悄悄得到错误的结果。`and`、`or` 是短路求值：左边已经能决定结果时，右边不会执行，所以 `x != nil and x > 0` 是安全的。

### 4.3 字符串

```zyx
let s = "Hello, ZYX"
print(len(s), s[0], s[-1])     # 10 H X
print(s[7..10])                # ZYX，切片
print(s.upper(), s.lower())    # HELLO, ZYX hello, zyx
print("ab" * 3)                # ababab
print("a-b-c".split("-"))      # ["a", "b", "c"]
print(["x", "y"].join("+"))    # x+y
let n = 42
print("n 的平方是 {n * n}")     # 插值里可以写任意表达式
```

字符串用双引号，不可修改，长度和下标都按字符计（中文也算一个字符）。`+` 只能连接两个字符串，数字要用插值或 `str()` 转换。

### 4.4 条件与循环

```zyx
let score = 85
if score >= 90 {
    print("优秀")
} else if score >= 60 {
    print("及格")
} else {
    print("不及格")
}

# if 也是表达式
let pass = if score >= 60 { "通过" } else { "未通过" }

let i = 0
while i < 3 {
    print("第 {i} 次")
    i += 1
}

for k in 1..=5 { write(k, "") }   # 1 2 3 4 5
print()
for idx, fruit in ["苹果", "香蕉"] {
    print("{idx}: {fruit}")
}
```

条件**必须是布尔值**。想判断数组非空要写 `len(xs) > 0`，判断数字非零要写 `n != 0`，判断有值要写 `x != nil`。直接写 `if xs` 会得到 `TypeError: if condition must be bool, got array`。

`a..b` 不含 `b`，`a..=b` 含 `b`。`break` 和 `continue` 作用于最内层循环。

### 4.5 函数与闭包

```zyx
fn area(w, h) {
    w * h              # 最后一个表达式就是返回值
}

fn safe_div(a, b) {
    if b == 0 { return nil }   # 提前返回
    float(a) / b
}

let square = fn(x) => x * x          # 匿名函数
print(area(3, 4), safe_div(1, 4), square(5))   # 12 0.25 25

# 闭包会记住外层变量
fn make_counter() {
    let n = 0
    fn() { n += 1; n }
}
let next = make_counter()
next(); next()
print(next())          # 3
```

函数可以在声明之前调用，相互递归的函数不需要额外处理。

### 4.6 数组

```zyx
let xs = [3, 1, 4]
xs.push(1)                     # 原地修改
xs[0] = 9
print(xs, len(xs))             # [9, 1, 4, 1] 4
print(xs.sort(), xs)           # [1, 1, 4, 9] [9, 1, 4, 1]，sort 返回新数组
print(xs.contains(4), xs.index_of(4))   # true 2

let grid = array(3, array(3, 0))   # 3×3 二维数组
grid[1][1] = 5

let squares = map(1..=4, fn(k) => k * k)
let evens = filter(squares, fn(k) => k % 2 == 0)
print(squares, evens, sum(squares))     # [1, 4, 9, 16] [4, 16] 30
```

数组是引用语义：`let b = a` 后修改 `b` 也会改到 `a`，需要独立副本时用 `copy(a)`。

### 4.7 结构体与方法

```zyx
struct Rect { w, h = 1 }       # h 有默认值

impl Rect {
    fn area(self) { self.w * self.h }
    fn scale(self, k) {
        self.w *= k
        self.h *= k
    }
}

let r = Rect(2, 3)
let thin = Rect(w: 5)          # 具名构造，h 取默认值
r.scale(2)
print(r, r.area(), thin.area())   # Rect(w: 4, h: 6) 24 5
```

结构体名本身就是构造函数。方法写在 `impl` 块里，第一个参数接收实例本身，惯例叫 `self`。

ZYX 没有字典类型。需要"按名字查找"时，用结构体数组配合一个查找函数，参见 `SKILL.md` 的 6.2 节。

### 4.8 模式匹配

```zyx
fn describe(n) {
    match n {
        0 => "零",
        1 | 2 | 3 => "很少",
        4..10 => "几个",
        x if x < 0 => "负数 {x}",
        _ => "很多",
    }
}
print(describe(0), describe(2), describe(7), describe(-1), describe(99))
# 零 很少 几个 负数 -1 很多
```

分支从上到下尝试，第一个匹配的生效。没有分支匹配时会报错，所以通常最后放一个 `_`。

### 4.9 管道与方法链

```zyx
let words = ["banana", "Apple", "cherry", "date"]
let result = words
    |> map(lower)
    |> filter(fn(w) => len(w) > 4)
    |> sort()
    |> join(", ")
print(result)      # apple, banana, cherry
```

`x |> f(a)` 等价于 `x.f(a)`，也等价于 `f(x, a)`。下一行以 `|>` 开头时自动接在上一行后面。

### 4.10 输入与错误处理

ZYX 没有 `try` / `catch`，任何运行时错误都会终止程序。处理用户输入时要先校验：

```zyx
let total = 0
while true {
    let line = input("输入一个数（直接回车结束）：")
    if line == nil or line == "" { break }   # nil 表示输入已结束
    let n = parse_int(line)                  # 解析失败返回 nil，不会报错
    if n == nil {
        print("“{line}” 不是整数，请重新输入")
        continue
    }
    total += n
}
print("合计：{total}")
```

### 4.11 中文关键字

每个关键字都有中文写法，可以和英文混用：

```zyx
函数 阶乘(n) {
    如果 n <= 1 { 返回 1 }
    n * 阶乘(n - 1)
}
对于 i 在 1..=5 {
    print("{i}! = {阶乘(i)}")
}
```

| 英文 | 中文 | 英文 | 中文 | 英文 | 中文 |
|---|---|---|---|---|---|
| `fn` | `函数` | `while` | `当` | `struct` | `结构` |
| `let` | `令` | `for` | `对于` | `impl` | `实现` |
| `if` | `如果` | `in` | `在` | `and` | `且` |
| `else` | `否则` | `break` | `跳出` | `or` | `或` |
| `return` | `返回` | `continue` | `继续` | `not` | `非` |
| `match` | `匹配` | `true` / `false` | `真` / `假` | `nil` | `空` |

中文关键字与相邻的名字之间要用空格隔开。

## 5. 报错与调试

所有错误都输出到标准错误，格式统一：

```text
[line 12:5] TypeError: if condition must be bool, got int
```

`12:5` 是第 12 行第 5 列。错误分为两种：

- **运行前报错**（退出码 65）：`SyntaxError` 以及大部分 `NameError`，比如拼错变量名、少了括号、混入了全角符号。程序一行都不会执行。
- **运行时报错**（退出码 70）：`TypeError`、`IndexError`、`ValueError`、`RuntimeError`、`AssertionError`。报错之前已经打印的内容会保留。

| 报错信息 | 通常的原因 |
|---|---|
| `full-width character '（', use '(' instead` | 中文输入法下打出了全角括号或逗号 |
| `undefined variable 'x'` | 变量没有用 `let` 声明，或拼错了 |
| `undefined function or method 'fliter'` | 函数名拼错 |
| `if condition must be bool, got array` | 条件写成了 `if xs`，应写 `if len(xs) > 0` |
| `unsupported operand types for '+': str and int` | 字符串和数字相加，改用插值 |
| `index 5 out of range for array of length 3` | 下标越界 |
| `no match arm for value 42` | `match` 缺少兜底分支 `_` |
| `variable 'x' is used before it is initialized` | 函数调用发生在它用到的变量 `let` 之前 |

排查技巧：

- 运算顺序拿不准时，用 `zyx --ast` 看语法树。例如 `-2 ** 2` 会显示为 `(- (** 2 2))`，说明先算乘方。
- 在关键位置加 `print` 或 `assert(条件, "说明")`。
- 两个后端的结果理应完全一致；如果不一致，说明解释器有 bug，欢迎记录下来。

## 6. 测试

### 6.1 运行测试

```sh
make test              # Rust 单元测试 + 全部 ZYX 黑盒测试（两个后端）
make test BACKENDS=vm  # 只测虚拟机后端
python3 tests/run_tests.py --backend vm closures   # 只跑路径中含 closures 的测试
```

### 6.2 编写测试

黑盒测试就是 `tests/zyx/` 下的普通 `.zyx` 程序，用注释写期望结果：

| 注释 | 含义 |
|---|---|
| `# expect: 文本` | 标准输出的下一行应为该文本，多条按顺序逐行比对 |
| `# expect-error: 文本` | 标准错误应为该文本（完整一行） |
| `# expect-exit: 数字` | 退出码，默认成功时为 0 |
| `# stdin: 文本` | 作为一行标准输入喂给程序 |

```zyx
# stdin: 3
let n = int(input())
print(n * n)                # expect: 9
assert_eq(n % 2, 1)
```

错误测试同样是普通程序，额外写上期望的报错：

```zyx
let xs = [1, 2, 3]
print(xs[5])
# expect-error: [line 2:9] IndexError: index 5 out of range for array of length 3
# expect-exit: 70
```

一个错误测试文件只测一个错误，因为第一个错误就会终止程序。

## 7. 性能

```sh
make bench    # ZYX 两个后端与 Python 对比，结果写入 bench/results.md，并重新生成图表
```

虚拟机后端在 6 组测试中耗时是 CPython 3.12 的 0.99 到 2.99 倍，详见 [性能测试报告](performance-report.md)。写对性能敏感的代码时有几点经验：

- 用 `--backend vm` 运行。
- 热循环放在函数里：函数内的局部变量访问最快。
- 拼接大量字符串时，先 `push` 进数组，最后 `join`；或者对局部变量用 `s += x`，独占的字符串会原地追加。

## 8. 项目结构

```text
ZYX_language/
├── src/
│   ├── lexer.rs, token.rs     # 词法分析：源码 → 词法单元（含中文关键字映射）
│   ├── parser.rs, ast.rs      # 语法分析：词法单元 → 语法树
│   ├── resolver.rs, ir.rs     # 槽位解析：语法树 → 中间表示，变量定位为槽位
│   ├── interp.rs              # 后端一：树遍历解释器
│   ├── bytecode.rs, vm.rs     # 后端二：字节码编译器与虚拟机
│   ├── value.rs, ops.rs       # 值的表示与运算语义（两个后端共用）
│   ├── builtins.rs            # 内置函数（两个后端共用）
│   └── main.rs                # 命令行入口
├── tests/zyx/                 # 黑盒测试，tests/run_tests.py 为运行器
├── bench/                     # 性能测试用例与脚本
├── docs/                      # 规格、指南、报告、架构决策记录（adr/）
├── examples/app/              # 应用程序（2048），由编程 Agent 开发
└── .claude/skills/zyx/        # 给编程 Agent 的文档
```

一个程序的执行过程：

```text
源码 ─词法分析─▶ 词法单元 ─语法分析─▶ 语法树 ─槽位解析─▶ 中间表示 ─┬─▶ 树遍历解释器
                                                              └─▶ 字节码编译器 ─▶ 虚拟机
```

为什么这样设计，可以看 [`docs/adr/`](adr/) 下的架构决策记录，比如为什么用 Rust（ADR-0001）、为什么保留两个后端（ADR-0002）、为什么条件必须是布尔值（ADR-0007）。
