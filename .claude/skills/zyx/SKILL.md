---
name: zyx
description: 编写、运行和调试 ZYX 语言（.zyx 文件）程序。ZYX 是一门花括号语法的动态类型脚本语言，有管道运算符 |>、字符串插值、match、结构体与 impl、中英文关键字。凡是要写或修改 .zyx 代码、解释 ZYX 报错、为 ZYX 写测试时使用。
---

# ZYX 编程指南（给编程 Agent）

ZYX 看起来像 Rust/JS 和 Python 的混合，但有几条规则和它们**都不一样**。动笔前先读完第 1 节，写完后对照第 7 节自查。完整规格在 `docs/reference.md`。

## 1. 最容易写错的 12 条（务必遵守）

| # | 错误写法 | 正确写法 | 说明 |
|---|---|---|---|
| 1 | `if arr { }`、`while n { }`、`if name { }` | `if len(arr) > 0 { }`、`while n != 0 { }`、`if name != nil { }` | **条件必须是 bool**，没有真值转换，`and`/`or`/`not` 的操作数也必须是 bool |
| 2 | `x = 1`（未声明） | `let x = 1`，之后才能 `x = 2` | 赋值前必须 `let` 声明，否则运行前报 `NameError` |
| 3 | `7 / 2` 期望 `3.5` | `float(7) / 2` 或 `7.0 / 2` | 两个 int 相除是**向下取整**的整数除法：`7 / 2 == 3`，`-7 / 2 == -4` |
| 4 | `!ok`、`a && b`、`a \|\| b` | `not ok`、`a and b`、`a or b` | 没有 `!`、`&&`、`\|\|` |
| 5 | `elif`、`else if` 写成 `elseif` | `} else if cond {` | 只有 `else if` |
| 6 | `'abc'` | `"abc"` | 字符串只能用双引号 |
| 7 | `"age: " + 18` | `"age: {18}"` 或 `"age: " + str(18)` | `+` 两边必须都是字符串 |
| 8 | `"{join(xs, " ")}"`、`"{f("x")}"` | `let line = join(xs, " ")` 再 `"{line}"` | 插值 `{...}` 里**不能出现字符串字面量**，最常见于在插值里调用 `join` |
| 9 | `Point { x: 1, y: 2 }`、`new Point(...)` | `Point(1, 2)` 或 `Point(x: 1, y: 2)` | 结构体名就是构造函数 |
| 10 | `dict = {}`、`d["key"]` | 结构体数组，或两个平行数组 | **没有字典 / 映射类型**，`{ }` 是代码块 |
| 11 | `try { } catch` | 先校验：`parse_int(s)` 失败返回 `nil` | 没有异常捕获，任何错误都会终止程序 |
| 12 | `a[1..]`、`a[..3]` | `a[1..len(a)]`、`a[0..3]`、`a[-2..len(a)]` | 开区间 `a..` / `..b` 只能用在 `match` 模式里 |

其他常见坑：

- `sort`、`reverse`、`map`、`filter` **返回新数组**，不修改原数组；`push`、`pop`、`insert`、`remove` 才会原地修改。
- 数组和结构体是**引用语义**：`let b = a` 后修改 `b` 会影响 `a`，需要副本时用 `copy(a)`（浅拷贝）。
- 没有 `++` / `--`，用 `i += 1`。赋值是语句，不能写在表达式里（`if (x = 1)` 是错的）。
- 没有链式比较：`0 < x < 10` 要写成 `0 < x and x < 10`。
- `"ab" * 3` 可以，`3 * "ab"` 报错。
- 中文关键字和标识符之间要有空格：`如果 n < 2`，不能写 `如果n < 2`。
- `match` 没有命中任何分支时报错，通常要加一个 `_ => ...` 兜底分支。
- `round(2.5) == 3`、`round(-2.5) == -3`（远离零取整），与 Python 的银行家舍入不同。
- `"a  b".split(" ")` 得到 `["a", "", "b"]`：连续分隔符之间会产生空串，按空白切词时要 `filter(fn(w) => len(w) > 0)`。
- 函数或方法的最后一条语句是赋值（如 `self.n += 1`）时，返回值是 `nil`；需要返回值就在最后单独写出表达式。
- 全角符号只在代码里报错，字符串和注释里可以随意使用。数组、参数、结构体字段和 `match` 分支都允许末尾逗号。
- `and` / `or` 是**短路求值**，可以放心写 `x != nil and x > 0`、`i < len(a) and a[i] == 0`。
- 区间起点大于终点时为空（`len(5..2) == 0`），没有步长和倒序区间。倒序遍历写成 `for k in 0..n { let i = n - 1 - k ... }`，或用 `while`。
- 变量、函数、结构体、字段名都可以用中文（如 `令 总和 = 0`、`结构 学生 { 姓名 }`）。
- 裸 `return` 返回 `nil`；函数最后一条是赋值、`for`、`while` 时返回 `nil`。

## 2. 语法速查

```zyx
# 注释用井号；语句以换行或 ; 结束
let n = 10                     # 整数（64 位，溢出报错）
let pi = 3.14                  # 浮点
let s = "hi\n"                 # 字符串，转义 \n \t \" \\ ；字面花括号写 {{ }}
let ok = true                  # true / false
let nothing = nil
let xs = [1, 2, 3]             # 数组，元素类型任意
let r = 0..5                   # 区间 0,1,2,3,4；0..=5 含 5

# 字符串插值：{表达式}
print("n = {n}, n*2 = {n * 2}, xs = {xs}")

# 条件（不加括号，花括号必须写），if 也是表达式
if n > 5 {
    print("big")
} else if n > 0 {
    print("small")
} else {
    print("non-positive")
}
let sign = if n >= 0 { "+" } else { "-" }

# 循环
while n > 0 { n -= 1 }
for i in 0..3 { print(i) }             # 0 1 2
for x in xs { print(x) }
for i, x in xs { print("{i}: {x}") }   # 同时取下标和元素
for c in "abc" { print(c) }            # 遍历字符
# break / continue 只作用于最内层循环

# 函数：最后一个表达式就是返回值，return 可提前返回
fn add(a, b) { a + b }
fn abs_val(x) {
    if x < 0 { return -x }
    x
}
let double = fn(x) => x * 2            # 匿名函数（单表达式）
let show = fn(a) { print(a) }          # 匿名函数（块）

# 闭包按引用捕获外层变量
fn counter() {
    let c = 0
    fn() { c += 1; c }
}

# 结构体与方法
struct Point { x, y = 0 }              # 字段可有默认值
impl Point {
    fn len2(self) { self.x ** 2 + self.y ** 2 }   # self 是显式的第一个参数
    fn move_by(self, dx, dy) { self.x += dx; self.y += dy }
}
let p = Point(3, 4)
let q = Point(x: 1)                    # y 取默认值 0
p.move_by(1, 1)
print(p.len2(), p)                     # 41 Point(x: 4, y: 5)

# match：字面量、区间、| 多选、变量绑定、if 守卫、_ 兜底
let score = 95
let grade = match score {
    100 => "满分",
    90..100 => "A",
    60..90 => "B",
    0 | 1 => "几乎没写",
    v if v < 0 => "非法",
    _ => "C",
}
let op = "+"
let name = match op {              # 字符串、bool、nil 字面量也能匹配
    "+" => "加",
    "-" | "−" => "减",
    _ => "未知",                     # 分支用逗号或换行分隔，允许末尾逗号
}

# 管道与统一调用语法
let r2 = [5, 3, 8] |> filter(fn(x) => x > 3) |> sort()   # [5, 8]
print(xs.len(), len(xs))               # x.f(a) 等价于 f(x, a)
```

运算符优先级（高 → 低）：调用 / 下标 / 字段 → `**`（右结合）→ 一元 `-` `not` → `* / %` → `+ -` → `..` `..=` → `|>` → `< <= > >=` → `== !=` → `and` → `or`。

更多语义细节：

- `%` 结果的符号跟随除数：`-7 % 3 == 2`。`int ** 负数` 报错，用 `2.0 ** -1`。
- int 与 float 混合运算得 float；`1 == 1.0` 为 `true`；不同类型用 `==` 比较结果为 `false`，不会报错；`<` 只能比较两个数或两个字符串。
- 下标从 0 开始，负数从末尾倒数：`a[-1]`。越界报 `IndexError`。切片 `a[i..j]` 越界会自动截断。
- 字符串不可变，`s[0] = "x"` 报错；`len(s)` 和下标都按字符（包括中文）计。
- 长表达式换行：行尾留运算符或逗号，或者下一行以 `|>` 或 `.` 开头。
- 函数和结构体的声明会提升，可以先调用、后声明；函数体可以引用同一作用域里稍后才 `let` 的变量，只要调用时它已赋值。

## 3. 内置函数

所有内置函数都可以写成方法形式：`len(a)` 等价于 `a.len()`，`push(a, x)` 等价于 `a.push(x)`。

| 类别 | 函数 | 说明 |
|---|---|---|
| 输入输出 | `print(...)` | 参数以空格分隔，末尾换行 |
| | `write(...)` | 同 `print`，但末尾不换行 |
| | `input(prompt?)` | 读一行（去掉换行符），**输入结束时返回 `nil`** |
| 转换 | `int(x)` | 字符串（允许首尾空白）、float（向零截断）、bool → int；失败报 `ValueError` |
| | `parse_int(s)` | 同 `int`，但失败返回 `nil`，**校验用户输入用它** |
| | `float(x)` | int、bool、字符串（允许首尾空白，如 `float(" 2.5 ")`）→ float；字符串不合法时报 `ValueError`，**没有** `parse_float`，需要校验浮点输入时自己检查字符 |
| | `str(x)`、`type(x)` | `type` 返回 `"int"`、`"float"`、`"str"`、`"bool"`、`"nil"`、`"array"`、`"range"`、`"fn"` 或结构体名 |
| 数组 | `len(x)` | 数组、字符串、区间的长度 |
| | `array(n, v)` | 长度为 n 的数组，每项为 v；v 是数组时每项是独立副本，适合建二维数组：`array(4, array(4, 0))` |
| | `push(a, x)` / `pop(a)` | 末尾追加 / 移除并返回末尾元素（空数组报错） |
| | `insert(a, i, x)` / `remove(a, i)` | 在下标 i 插入 / 移除并返回下标 i 的元素 |
| | `contains(c, x)` / `index_of(c, x)` | 数组含元素、字符串含子串；找不到下标返回 `-1` |
| | `copy(x)` | 数组或结构体的浅拷贝 |
| | `reverse(c)` | 数组返回反转后的**新数组**；字符串返回反转后的字符串 |
| | `sort(a, key?)` | 返回排好序的**新数组**（升序、稳定）。元素或 `key` 的结果必须是数或字符串，**不能是数组**，见下方"排序" |
| | `map(c, f)` / `filter(c, f)` / `reduce(c, init, f)` | `c` 可以是数组、区间或字符串；`filter` 的函数必须返回 bool；`reduce` 调用 `f(acc, x)` |
| | `sum(c)`、`min(...)`、`max(...)` | `min` / `max` 接受多个数，或一个数组 |
| | `join(a, sep)` | 元素按 `str()` 转换后连接 |
| 字符串 | `split(s, sep)` | `sep` 为 `""` 时切成单个字符；连续分隔符之间产生空串 |
| | `trim`、`upper`、`lower`、`replace(s, from, to)` | `replace` 替换**所有**出现 |
| | `starts_with(s, p)`、`ends_with(s, p)` | |
| | `pad_left(s, n, ch?)`、`pad_right(s, n, ch?)` | 补齐到 n 个字符，默认补空格；`s` 可以是数字，用于对齐表格；比 n 长时原样返回、不截断 |
| | `ord(c)`、`chr(n)` | 字符与码点互转 |
| 数学 | `abs`、`sqrt`（返回 float）、`floor`、`ceil`、`round`（返回 int） | |
| 其他 | `random(a, b)` | `[a, b]` 闭区间内的随机整数 |
| | `seed(n)` | 固定随机种子，便于复现 |
| | `time()` | 秒数（float），用于计时 |
| | `assert(cond, msg?)`、`assert_eq(a, b)` | 测试用，失败报 `AssertionError` |
| | `error(msg)` | 主动报错并终止 |
| | `exit(code?)` | 结束程序 |

不存在的东西：字典、集合、元组、类、继承、模块导入、文件读写、格式化占位符（`%d`、`{:>5}`）、可变参数的用户函数、位运算。

### 排序

```zyx
let xs = [3, 1, 2]
print(sort(xs))                        # [1, 2, 3]
print(reverse(sort(xs)))               # [3, 2, 1]，降序
print(sort(xs, fn(x) => -x))           # [3, 2, 1]，数值降序的另一种写法
# 多级排序：sort 是稳定的，先按次要键排，再按主要键排
# 例：按次数降序，次数相同时按名字升序
let ranked = items |> sort(fn(e) => e.name) |> sort(fn(e) => -e.count)
```

`sort(xs, fn(x) => [x.a, x.b])` 会报 `TypeError: sort() cannot compare array and array`，要用上面的两次排序代替。字符串降序用 `reverse(sort(...))`，但这样相同键的元素顺序也会被反转。

### 输出格式

`print`、`str()` 和插值使用同一套格式：

| 值 | 显示为 |
|---|---|
| `42`、`-7` | `42`、`-7` |
| `6.0 / 3`、`1.0 / 3`、`1e20` | `2.0`、`0.3333333333333333`、`1e20`（整数值的浮点保留 `.0`） |
| `"hi"` | `hi`（顶层不加引号） |
| `[1, "s", nil]` | `[1, "s", nil]`（容器里的字符串加引号） |
| `true`、`nil` | `true`、`nil` |
| 结构体实例 | `Point(x: 1, y: 2)` |
| 区间 `0..=3` | `0..4` |

没有格式化占位符。保留一位小数可以算 `round(x * 10) / 10.0`，但它是 float，`20.0` 会打印成 `20.0` 而不是 `20.00`。需要固定位数时用下面的函数（返回字符串），对齐再配合 `pad_left` / `pad_right`：

```zyx
# 保留两位小数（四舍五入），返回字符串：fmt2(20) == "20.00"，fmt2(-0.5) == "-0.50"
fn fmt2(x) {
    let cents = round(x * 100)
    let sign = if cents < 0 { "-" } else { "" }
    let c = abs(cents)
    let frac = pad_left(c % 100, 2, "0")     # 先存进变量：插值里不能写 "0"
    "{sign}{c / 100}.{frac}"
}
```

注意浮点误差：`fmt2(7.005)` 的结果取决于 `7.005 * 100` 的二进制近似值。

### 关键字（都是保留字，不能当变量名）

| 英文 | 中文 | 英文 | 中文 | 英文 | 中文 |
|---|---|---|---|---|---|
| `fn` | `函数` | `while` | `当` | `struct` | `结构` |
| `let` | `令` | `for` | `对于` | `impl` | `实现` |
| `if` | `如果` | `in` | `在` | `and` | `且` |
| `else` | `否则` | `break` | `跳出` | `or` | `或` |
| `return` | `返回` | `continue` | `继续` | `not` | `非` |
| `match` | `匹配` | `true` / `false` | `真` / `假` | `nil` | `空` |

除此之外的名字（`from`、`to`、`self`、`type`、`list` 等）都不是保留字。内置函数名可以被同名的变量或函数遮蔽。

## 4. 运行与调试

```sh
make build                         # 编译解释器（cargo build --release）
./target/release/zyx 程序.zyx       # 运行（默认树遍历后端）
./target/release/zyx --backend vm 程序.zyx   # 字节码虚拟机后端，更快，语义相同
./target/release/zyx --ast 程序.zyx  # 打印语法树，排查优先级问题
make test                          # 全部测试（两个后端）
```

报错格式固定为 `[line 行:列] 类别: 信息`，写到标准错误：

| 类别 | 常见原因 | 退出码 |
|---|---|---|
| `SyntaxError` | 少括号、全角符号（`（`、`，`）、`!`、单引号、插值里写了字符串、`break` 不在循环里 | 65 |
| `NameError` | 变量没 `let`、拼错函数名（运行前报 65）；变量在初始化前被使用（运行时报 70） | 65 / 70 |
| `TypeError` | **条件不是 bool**、类型不匹配、参数个数不对、字段不存在 | 70 |
| `IndexError` | 下标越界、`pop` 空数组 | 70 |
| `ValueError` | `int("abc")` 等转换失败 | 70 |
| `RuntimeError` | 除以零、整数溢出、`match` 无分支命中、递归超过 10000 层 | 70 |

运行前报错（退出码 65）时程序一行都不会执行，先修语法。

## 5. 写测试

测试是放在 `tests/zyx/` 下的 `.zyx` 文件，用注释写期望结果，`make test` 自动比对：

```zyx
# stdin: 42                 喂给 input() 的一行
let n = int(input())
print(n * 2)                # expect: 84
assert_eq(n % 5, 2)
# 错误用例：一个文件只测一个错误
# expect-error: [line 9:3] TypeError: ...
# expect-exit: 70
```

## 6. 完整示例

### 6.1 读取输入、统计成绩

```zyx
struct Student { name, score }

fn level(s) {
    match s {
        90..=100 => "A",
        75..90 => "B",
        60..75 => "C",
        _ => "D",
    }
}

let students = []
while true {
    let line = input()
    if line == nil { break }                 # 输入结束
    let parts = line.trim().split(" ")
    if len(parts) != 2 { continue }
    let score = parse_int(parts[1])
    if score == nil {
        print("跳过无效行：{line}")
        continue
    }
    students.push(Student(parts[0], score))
}

if len(students) == 0 {
    print("没有数据")
    exit(0)
}

let ranked = students |> sort(fn(s) => -s.score)
for i, s in ranked {
    print("{pad_left(i + 1, 2)}. {pad_right(s.name, 8)} {pad_left(s.score, 3)} {level(s.score)}")
}
let total = students |> map(fn(s) => s.score) |> sum()
print("平均分：{round(float(total) / len(students) * 10) / 10.0}")
```

### 6.2 没有字典时如何计数（结构体数组）

```zyx
struct Entry { word, count = 0 }

fn find(entries, word) {
    for i, e in entries {
        if e.word == word { return i }
    }
    -1
}

let entries = []
for w in "a b a c b a".split(" ") {
    let i = find(entries, w)
    if i == -1 {
        entries.push(Entry(w, 1))
    } else {
        entries[i].count += 1
    }
}
for e in entries |> sort(fn(e) => -e.count) {
    print("{e.word}: {e.count}")
}
# a: 3
# b: 2
# c: 1
```

### 6.3 二维数组与游戏循环

```zyx
let size = 3
let board = array(size, array(size, "."))

fn show(b) {
    for row in b { print(join(row, " ")) }
}

fn place(b, r, c, mark) {
    if r < 0 or r >= len(b) or c < 0 or c >= len(b) { return false }
    if b[r][c] != "." { return false }
    b[r][c] = mark
    true
}

place(board, 1, 1, "X")
place(board, 0, 2, "O")
show(board)
# . . O
# . X .
# . . .
```

## 7. 写完后的自查清单

- [ ] 每个 `if` / `while` / 守卫的条件都是比较或 bool 变量，而不是数组、数字、字符串、nil
- [ ] 每个变量第一次出现时都有 `let`
- [ ] 需要小数结果的除法用了 `float()`
- [ ] 没有 `!`、`&&`、`||`、`elif`、单引号、`{}` 字典、`Point{...}`
- [ ] 插值 `{...}` 里没有字符串字面量
- [ ] 读取输入时处理了 `input()` 返回 `nil` 和 `parse_int` 返回 `nil` 的情况
- [ ] 用了 `sort` / `reverse` 的返回值，而不是期望它们原地修改
- [ ] `match` 有兜底分支 `_`
- [ ] 实际运行过一遍（`./target/release/zyx 文件.zyx`），按报错中的行列号修正
