# ZYX 语言参考手册

> 版本：v0.2（核心设计已确认，见 `docs/adr/`） · 本文件是 ZYX 语义的唯一依据，解释器、测试集和开发指南都以它为准。

ZYX 是一门动态类型、解释执行的通用脚本语言。语法采用花括号分块，特色包括：管道运算符 `|>`、统一调用语法、字符串插值、带区间与守卫的 `match`、块表达式，以及中英文可混用的关键字。

```zyx
函数 fib(n) {
    如果 n < 2 { 返回 n }
    fib(n - 1) + fib(n - 2)
}

let total = 0
for i in 1..=10 { total += fib(i) }
print("total = {total}")          # total = 143
```

运行方式：

```sh
zyx hello.zyx            # 运行脚本（默认后端）
zyx --backend tree a.zyx # 指定树遍历后端
zyx --backend vm a.zyx   # 指定字节码虚拟机后端
```

---

## 1. 词法

### 1.1 源文件

源文件为 UTF-8 编码，扩展名 `.zyx`。

### 1.2 注释

`#` 到行尾为注释。没有块注释。

### 1.3 标识符

由 Unicode 字母（含汉字）、数字和 `_` 组成，不能以数字开头，区分大小写。例如：`count`、`_tmp`、`总和`、`学生2`。

### 1.4 关键字

每个关键字都有中文别名，二者完全等价，可在同一文件中混用。中英文关键字都是保留字，不能用作标识符。

中文关键字与相邻的标识符之间必须用空格或符号隔开：`如果 n < 2` 正确，`如果n < 2` 会被识别为一个名为 `如果n` 的标识符。

| 英文 | 中文 | 用途 |
|---|---|---|
| `fn` | `函数` | 函数声明、匿名函数 |
| `let` | `令` | 变量声明 |
| `if` | `如果` | 条件分支 |
| `else` | `否则` | 条件分支 |
| `while` | `当` | 条件循环 |
| `for` | `对于` | 遍历循环 |
| `in` | `在` | 遍历循环 |
| `break` | `跳出` | 跳出循环 |
| `continue` | `继续` | 进入下一轮循环 |
| `return` | `返回` | 函数返回 |
| `match` | `匹配` | 模式匹配 |
| `struct` | `结构` | 结构体声明 |
| `impl` | `实现` | 结构体方法 |
| `and` | `且` | 逻辑与 |
| `or` | `或` | 逻辑或 |
| `not` | `非` | 逻辑非 |
| `true` | `真` | 布尔真 |
| `false` | `假` | 布尔假 |
| `nil` | `空` | 空值 |

### 1.5 字面量

| 类别 | 示例 | 说明 |
|---|---|---|
| 整数 | `42`、`1_000_000`、`0xff` | 下划线仅作分隔，可用十六进制 |
| 浮点 | `3.14`、`1.5e3`、`2.0` | 小数点两侧都必须有数字，`5.` 不合法 |
| 字符串 | `"hello"`、`"第 {n} 行"` | 只用双引号，支持插值 |
| 布尔 | `true`、`false`、`真`、`假` | |
| 空值 | `nil`、`空` | |
| 数组 | `[1, "a", [2, 3]]` | 允许末尾逗号 |

### 1.6 字符串转义与插值

转义序列：`\n` `\t` `\r` `\\` `\"` `\0`。

字符串中的 `{表达式}` 会被求值并按 `str()` 规则转为字符串后拼接。字面的花括号写成 `{{` 和 `}}`。

```zyx
let name = "ZYX"
print("Hello, {name}! 1 + 1 = {1 + 1}")   # Hello, ZYX! 1 + 1 = 2
print("{{这不是插值}}")                     # {这不是插值}
```

插值内部不能再嵌套字符串字面量；需要时先赋给变量。

### 1.7 语句分隔与换行

语句以换行或 `;` 结束。以下情况的换行被忽略，语句延续到下一行：

1. 换行位于未闭合的 `(` 或 `[` 内；
2. 行尾是二元运算符、`,`、`=`、复合赋值符或 `=>`；
3. 下一个非空行以 `|>` 或 `.` 开头（用于链式调用）。

`else` 可以出现在 `}` 之后的下一行。

```zyx
let r = [3, 1, 2]
    |> sort()
    |> map(fn(x) => x * 2)
```

### 1.8 全角符号

在代码位置（字符串和注释之外）出现全角标点（如 `（` `）` `，` `；` `：` `“`）时，报 `SyntaxError` 并提示改用半角符号。

---

## 2. 类型与值

| 类型名 | 说明 | 示例 |
|---|---|---|
| `int` | 64 位有符号整数，溢出报错 | `42` |
| `float` | 64 位浮点数 | `3.14` |
| `str` | 不可变 Unicode 字符串，按字符计长度和下标 | `"你好"` |
| `bool` | 布尔值 | `true` |
| `nil` | 空值，唯一取值 `nil` | `nil` |
| `array` | 可变动态数组，元素类型任意 | `[1, 2]` |
| `range` | 整数区间 | `0..10` |
| `fn` | 函数或闭包 | `fn(x) => x` |
| 结构体名 | 用户定义的结构体实例 | `Point(1, 2)` |

`type(x)` 返回上表中的类型名字符串；结构体实例返回其结构体名，例如 `"Point"`。

### 2.1 值语义与引用语义

`int`、`float`、`str`、`bool`、`nil`、`range` 是值语义。`array` 和结构体实例是引用语义：赋值、传参都共享同一个对象，需要副本时用 `copy()`。

```zyx
let a = [1, 2]
let b = a
b.push(3)
print(a)          # [1, 2, 3]
```

### 2.2 条件必须是布尔值

`if`、`while`、`and`、`or`、`not` 以及 `match` 守卫的条件必须是 `bool`，否则报 `TypeError`。ZYX 没有"真值"转换：`if arr { }` 是错误的，应写 `if len(arr) > 0 { }`。

### 2.3 相等与比较

- `==` / `!=` 可用于任意两个值，不会报错。不同类型一律不相等，唯一例外是 `int` 与 `float` 按数值比较（`1 == 1.0` 为 `true`）。
- 数组和结构体按内容逐项比较；函数按身份比较。
- `<` `<=` `>` `>=` 只能用于两个数（`int`/`float`）或两个字符串（按 Unicode 码点字典序），其他组合报 `TypeError`。

### 2.4 转为字符串的规则

`print`、插值和 `str()` 都使用下列规则：

| 值 | 结果 |
|---|---|
| `42` | `42` |
| `2.0`、`3.14` | `2.0`、`3.14`（整数值的浮点数保留 `.0`） |
| `"hi"` | `hi`（顶层不加引号） |
| `true` / `nil` | `true` / `nil` |
| `[1, "a", nil]` | `[1, "a", nil]`（容器内的字符串加引号） |
| `Point(1, 2)` | `Point(x: 1, y: 2)` |
| `0..5` | `0..5` |
| 函数 | `<fn 名字>`，匿名函数为 `<fn>` |

---

## 3. 表达式

### 3.1 运算符优先级

从高到低：

| 级别 | 运算符 | 结合性 |
|---|---|---|
| 1 | 调用 `f(x)`、下标 `a[i]`、字段 `a.b` | 左 |
| 2 | `**` | 右 |
| 3 | 一元 `-`、`not` | 右 |
| 4 | `*` `/` `%` | 左 |
| 5 | `+` `-` | 左 |
| 6 | `..` `..=` | 不结合 |
| 7 | `\|>` | 左 |
| 8 | `<` `<=` `>` `>=` | 不结合 |
| 9 | `==` `!=` | 不结合 |
| 10 | `and` | 左 |
| 11 | `or` | 左 |

`-2 ** 2` 的结果是 `-4`。`a < b < c` 是语法错误，应写 `a < b and b < c`。

### 3.2 算术

- `int` 与 `int` 运算得 `int`；只要有一个 `float`，结果为 `float`。
- `int / int` 是**向下取整的整数除法**：`7 / 2` 为 `3`，`-7 / 2` 为 `-4`。需要小数结果时写 `float(7) / 2`。
- `%` 结果的符号与除数相同：`-7 % 3` 为 `2`。
- `**` 两侧都是 `int` 且指数非负时结果为 `int`，指数为负报 `ValueError`；含 `float` 时结果为 `float`。
- 整数溢出报 `RuntimeError`；除以零报 `RuntimeError`。

### 3.3 字符串运算

- `s + t`：两侧都必须是字符串，`"a" + 1` 报 `TypeError`，应写 `"a" + str(1)` 或 `"a{1}"`。
- `s * n`：重复 `n` 次，`"-" * 3` 为 `"---"`。
- `s[i]`：第 `i` 个字符，结果是长度为 1 的字符串。字符串不可变，`s[0] = "x"` 报 `TypeError`。

### 3.4 逻辑运算

`and`、`or` 短路求值，两侧都必须是 `bool`，结果为 `bool`。

### 3.5 区间

- `a..b`：`a` 到 `b - 1`；`a..=b`：`a` 到 `b`。两端必须是 `int`。
- 区间是值，可遍历、可传给 `map`/`filter` 等函数（结果为数组）、可用 `len()`。
- `a..` 和 `..b`、`..=b` 只能用在 `match` 模式中（见第 7 节）。

### 3.6 下标与切片

- `a[i]`：`i` 为 `int`；负数从末尾倒数，`a[-1]` 是最后一个元素。越界报 `IndexError`。
- `a[i] = v`：修改数组元素。
- `a[x..y]`、`a[x..=y]`：切片，对数组和字符串都适用，返回新对象。端点为负数时同样从末尾倒数（`s[0..-1]` 去掉最后一个字符）；超出范围的端点会被截断到合法范围，不报错。

### 3.7 统一调用语法

`x.name(参数...)` 的解析顺序：

1. `x` 是结构体实例，且其类型在 `impl` 中定义了方法 `name` → 调用该方法，`x` 作为 `self`；
2. 否则，若当前作用域可见名为 `name` 的函数（内置或用户定义）→ 等价于 `name(x, 参数...)`；
3. 否则报 `NameError`。若 `name` 既不是任何可见的函数，也不是程序中任何 `impl` 定义的方法，在运行前即报错。

因此 `arr.push(1)` 与 `push(arr, 1)` 等价，`5.double()` 与 `double(5)` 等价。字段中存放的函数不能用 `x.f()` 调用，需写成 `(x.f)()`。

### 3.8 管道运算符

`x |> f(a, b)` 等价于 `x.f(a, b)`，`x |> f` 等价于 `x.f()`，即按统一调用语法（3.7）解析：`x` 是定义了方法 `f` 的结构体时调用该方法，否则调用函数 `f(x, a, b)`。`x |> o.m(a)` 等价于 `o.m(x, a)`。右侧必须是函数名、`对象.方法名` 或这两者的调用形式。`|>` 的优先级低于算术和区间、高于比较和逻辑运算，所以：

```zyx
let a = [1, 2, 3] |> len() > 2     # ([1, 2, 3] |> len()) > 2，结果为 true
let b = 1..20 |> filter(f)         # filter(1..20, f)
let c = x + 1 |> str()             # str(x + 1)
```

### 3.9 `if` 与 `match` 是表达式

```zyx
let sign = if x > 0 { "正" } else if x < 0 { "负" } else { "零" }
```

`if` 缺少 `else` 分支且条件为假时，值为 `nil`。

### 3.10 块表达式

`{ ... }` 块的值是其最后一条语句的值：最后一条是表达式语句时取该表达式的值，否则为 `nil`。函数体、`if` 分支、`match` 分支都遵循此规则。

---

## 4. 语句

### 4.1 变量声明

```zyx
let x = 10
let y            # 值为 nil
令 名字 = "小明"
```

- 变量作用域为所在的块。同一块内重复 `let` 同名变量会遮蔽先前的变量。
- 使用未声明的变量报 `NameError`（在运行前的解析阶段检查）。

### 4.2 赋值

```zyx
x = 20
arr[0] = 1
p.x = 3
x += 1           # 支持 += -= *= /= %=
```

- 赋值是语句，不是表达式，`if (x = 1)` 是语法错误。
- 赋值前变量必须已用 `let` 声明，否则报 `NameError`。这能拦住拼写错误。

### 4.3 条件

```zyx
if score >= 90 {
    print("A")
} else if score >= 60 {
    print("B")
} else {
    print("C")
}
```

条件不需要括号，花括号不可省略。

### 4.4 循环

```zyx
while i < 10 { i += 1 }

for x in [1, 2, 3] { print(x) }        # 遍历数组
for c in "你好" { print(c) }            # 遍历字符
for i in 0..5 { print(i) }             # 遍历区间
for i, x in ["a", "b"] { print("{i}:{x}") }   # 同时取下标和元素
```

- `break` 跳出最内层循环，`continue` 进入下一轮。在循环外使用报 `SyntaxError`。
- 循环本身的值为 `nil`。
- 遍历数组时修改该数组的长度，行为未定义。

### 4.5 返回

`return 表达式` 或 `return`（返回 `nil`）。在函数外使用报 `SyntaxError`。

---

## 5. 函数

### 5.1 声明

```zyx
fn add(a, b) {
    a + b          # 最后一个表达式即返回值
}

fn greet(name) {
    if name == "" { return "无名氏" }
    "你好，{name}"
}
```

- 参数个数必须与调用时一致，否则报 `TypeError`。
- 同一作用域内的 `fn` 和 `struct` 声明会被提升，可以在声明之前调用，从而支持相互递归。
- 函数体（包括匿名函数）可以引用同一作用域中稍后才声明的变量；如果调用时该变量还没执行到 `let`，报 `NameError: variable 'x' is used before it is initialized`。
- 函数可以嵌套声明。

### 5.2 匿名函数

```zyx
let double = fn(x) => x * 2                  # 单表达式形式
let show = fn(a, b) { print(a); print(b) }   # 块形式
```

### 5.3 闭包

函数捕获外层变量的**引用**，外层修改对函数可见，函数内修改对外层也可见。

```zyx
fn make_counter() {
    let n = 0
    fn() { n += 1; n }
}
let c = make_counter()
c(); c()
print(c())         # 3
```

### 5.4 调用深度

调用深度上限为 10000 层，超出报 `RuntimeError: stack overflow`。

---

## 6. 结构体

### 6.1 声明

```zyx
struct Point { x, y = 0 }          # 字段可以有默认值
结构 学生 { 姓名, 分数 }
```

### 6.2 构造

结构体名本身就是构造函数，支持按位置和按字段名两种方式，不能混用：

```zyx
let p = Point(3, 4)
let q = Point(x: 1)                # y 取默认值 0
let s = 学生(姓名: "小明", 分数: 95)
```

同一次调用中混用两种方式报 `SyntaxError`。缺少没有默认值的字段，或给出不存在的字段，报 `TypeError`。

### 6.3 字段访问

`p.x` 读取，`p.x = 5` 修改。访问或赋值不存在的字段报 `TypeError`；结构体创建后不能新增字段。

### 6.4 方法

```zyx
impl Point {
    fn len2(self) { self.x ** 2 + self.y ** 2 }
    fn move_by(self, dx, dy) { self.x += dx; self.y += dy }
}

let p = Point(3, 4)
print(p.len2())        # 25
p.move_by(1, 1)
```

方法的第一个参数接收实例本身，惯例命名为 `self`。同一结构体可以有多个 `impl` 块。

---

## 7. 模式匹配

```zyx
let grade = match score {
    100 => "满分",
    90..100 => "A",
    60..90 => "B",
    0 | 1 | 2 => "几乎没写",
    n if n < 0 => "非法分数",
    _ => "C",
}
```

| 模式 | 含义 |
|---|---|
| 字面量 `1`、`"a"`、`true`、`nil` | 用 `==` 比较 |
| `a..b`、`a..=b` | 值是 `int` 且落在区间内 |
| `a..` | 值是 `int` 且 `>= a` |
| `..b`、`..=b` | 值是 `int` 且 `< b`、`<= b` |
| `p1 \| p2` | 任一子模式匹配即可 |
| 标识符 `n` | 总是匹配，并把值绑定到 `n`（作用域为该分支） |
| `_` | 总是匹配，不绑定 |

- 任一模式后都可加守卫 `if 条件`，守卫必须是 `bool`。
- 自上而下尝试，取第一个匹配的分支。没有分支匹配时报 `RuntimeError: no match arm`。
- 分支之间用逗号或换行分隔；分支体可以是表达式或块。

---

## 8. 内置函数

所有内置函数都可以按统一调用语法写成方法形式，例如 `len(a)` 也可写作 `a.len()`。

### 8.1 输入输出

| 函数 | 说明 |
|---|---|
| `print(...)` | 输出各参数，以空格分隔，末尾换行；无参数时只输出换行 |
| `write(...)` | 同 `print`，但末尾不换行 |
| `input(prompt?)` | 输出提示语（不换行），读取一行并去掉行尾换行符；遇到输入结束返回 `nil` |

### 8.2 类型与转换

| 函数 | 说明 |
|---|---|
| `type(x)` | 返回类型名字符串 |
| `int(x)` | 字符串按十进制解析（允许首尾空白）；浮点向零截断；布尔转 0/1；失败报 `ValueError` |
| `float(x)` | 转为浮点；失败报 `ValueError` |
| `parse_int(s)` | 同 `int(s)`，但解析失败时返回 `nil` 而不报错，用于校验用户输入 |
| `str(x)` | 按 2.4 节规则转为字符串 |

### 8.3 数组与通用

| 函数 | 说明 |
|---|---|
| `len(x)` | 数组、字符串、区间的长度 |
| `array(n, v)` | 创建长度为 `n`、每个元素为 `v` 的数组（`v` 为数组时每个元素都是独立副本） |
| `copy(a)` | 数组或结构体的浅拷贝 |
| `push(a, x)` | 在末尾追加，返回 `nil` |
| `pop(a)` | 移除并返回最后一个元素；空数组报 `IndexError` |
| `insert(a, i, x)` | 在下标 `i` 处插入 |
| `remove(a, i)` | 移除并返回下标 `i` 处的元素 |
| `contains(c, x)` | 数组是否含元素 `x`；字符串是否含子串 `x` |
| `index_of(c, x)` | 首次出现的下标，找不到返回 `-1` |
| `reverse(c)` | 返回反转后的新数组或新字符串 |
| `sort(a, key?)` | 返回排序后的新数组；可选的 `key` 函数把元素映射为比较键。稳定排序 |
| `map(c, f)` | 对数组或区间的每个元素调用 `f`，返回新数组 |
| `filter(c, f)` | 保留 `f` 返回 `true` 的元素，返回新数组 |
| `reduce(c, init, f)` | 以 `init` 为初值，依次计算 `f(acc, x)` |
| `sum(c)` | 数值求和，空数组为 `0` |
| `min(...)` / `max(...)` | 参数为多个数，或单个数组 |
| `join(a, sep)` | 把元素按 `str()` 转换后用 `sep` 连接 |

### 8.4 字符串

| 函数 | 说明 |
|---|---|
| `split(s, sep)` | 按分隔符切分为数组；`sep` 为 `""` 时切成单个字符 |
| `trim(s)` | 去掉首尾空白 |
| `upper(s)` / `lower(s)` | 转大写 / 小写 |
| `replace(s, from, to)` | 替换所有出现 |
| `starts_with(s, p)` / `ends_with(s, p)` | 前缀 / 后缀判断 |
| `pad_left(s, n, ch?)` / `pad_right(s, n, ch?)` | 用 `ch`（默认空格）补齐到 `n` 个字符；`s` 不是字符串时先按 `str()` 转换 |
| `ord(c)` / `chr(n)` | 字符与 Unicode 码点互转 |

### 8.5 数学

| 函数 | 说明 |
|---|---|
| `abs(x)` | 绝对值 |
| `sqrt(x)` | 平方根，返回 `float` |
| `floor(x)` / `ceil(x)` / `round(x)` | 返回 `int` |

### 8.6 其他

| 函数 | 说明 |
|---|---|
| `random(a, b)` | `[a, b]` 内均匀分布的随机整数 |
| `seed(n)` | 设定随机数种子，使结果可复现 |
| `time()` | 从某固定时刻起的秒数（`float`），用于计时 |
| `assert(cond, msg?)` | `cond` 为 `false` 时报 `AssertionError` |
| `assert_eq(a, b)` | `a != b` 时报 `AssertionError`，信息中包含两侧的值 |
| `error(msg)` | 主动抛出 `RuntimeError` |
| `exit(code?)` | 以指定退出码（默认 0）结束程序 |

---

## 9. 错误

### 9.1 类别

| 类别 | 何时发生 |
|---|---|
| `SyntaxError` | 词法、语法错误；`break`/`continue`/`return` 用错位置 |
| `NameError` | 使用或赋值未声明的变量；调用的名字既不是函数也不是任何方法（以上在运行前报告）；变量在初始化前被使用；统一调用语法在运行时找不到方法或函数 |
| `TypeError` | 类型不匹配；参数个数不对；字段不存在；条件不是 `bool` |
| `IndexError` | 下标越界；对空数组 `pop` |
| `ValueError` | `int("abc")` 等转换失败；负指数的整数幂 |
| `AssertionError` | `assert` / `assert_eq` 失败 |
| `RuntimeError` | 整数溢出；除以零；`match` 无分支匹配；栈溢出；`error()` |

ZYX v1 没有异常捕获机制，任何错误都会终止程序。

### 9.2 格式

错误信息输出到标准错误，格式固定为：

```
[line 行:列] 类别: 信息
```

例如：

```
[line 2:10] IndexError: index 5 out of range for array of length 3
```

`SyntaxError` 和大部分 `NameError` 在程序开始执行前报告，此时程序不会产生任何输出。

### 9.3 退出码

| 情况 | 退出码 |
|---|---|
| 正常结束 | 0（或 `exit` 指定的值） |
| 运行前报错（`SyntaxError`、解析阶段的 `NameError`） | 65 |
| 运行时报错 | 70 |
| 命令行用法错误、文件不存在 | 64 |

---

## 10. 示例程序

### 10.1 输入输出

```zyx
let name = input("你的名字：")
let age = int(input("你的年龄："))
print("你好，{name}！明年你 {age + 1} 岁。")
```

### 10.2 FizzBuzz

```zyx
for i in 1..=15 {
    let s = match i {
        n if n % 15 == 0 => "FizzBuzz",
        n if n % 3 == 0 => "Fizz",
        n if n % 5 == 0 => "Buzz",
        _ => str(i),
    }
    print(s)
}
```

### 10.3 管道与统一调用语法

```zyx
let nums = [5, 3, 8, 1, 9, 2]
let result = nums
    |> filter(fn(x) => x > 2)
    |> map(fn(x) => x * 10)
    |> sort()
print(result)          # [30, 50, 80, 90]
print(nums.sum())      # 28
```

### 10.4 结构体与方法

```zyx
struct Point { x, y = 0 }

impl Point {
    fn add(self, other) { Point(self.x + other.x, self.y + other.y) }
    fn len2(self) { self.x ** 2 + self.y ** 2 }
}

let p = Point(3, 4)
let q = Point(x: 1)
let r = p.add(q)
print(r)               # Point(x: 4, y: 4)
print(p.len2())        # 25
r.x = 10
print(r.x)             # 10
```

### 10.5 闭包

```zyx
fn make_counter() {
    let n = 0
    fn() { n += 1; n }
}

let c = make_counter()
c(); c()
print(c())             # 3
```

### 10.6 字符串处理

```zyx
let text = "the quick brown fox jumps over the lazy dog"
let words = text.split(" ")
print(len(words))      # 9

let longest = words.reduce("", fn(acc, w) => if len(w) > len(acc) { w } else { acc })
print(longest)         # quick
print(text.upper()[0..9])   # THE QUICK
```

### 10.7 冒泡排序

```zyx
fn bubble(a) {
    let n = len(a)
    for i in 0..n {
        for j in 0..n - 1 - i {
            if a[j] > a[j + 1] {
                let t = a[j]
                a[j] = a[j + 1]
                a[j + 1] = t
            }
        }
    }
    a
}

print(bubble([5, 2, 9, 1]))   # [1, 2, 5, 9]
```

### 10.8 中文关键字

```zyx
结构 学生 { 姓名, 分数 }

函数 等级(分) {
    匹配 分 {
        90..=100 => "优",
        60..90 => "良",
        _ => "差",
    }
}

令 名单 = [学生("小明", 95), 学生("小红", 72), 学生("小刚", 40)]
对于 序号, 人 在 名单 {
    print("{序号 + 1}. {人.姓名}：{等级(人.分数)}")
}
# 1. 小明：优
# 2. 小红：良
# 3. 小刚：差
```

### 10.9 猜数字

```zyx
seed(42)
let target = random(1, 100)
let tries = 0
while true {
    let line = input("猜一个 1 到 100 的数：")
    if line == nil { break }
    let g = int(line)
    tries += 1
    if g < target {
        print("太小了")
    } else if g > target {
        print("太大了")
    } else {
        print("猜中了！共猜了 {tries} 次")
        break
    }
}
```

### 10.10 2048 的一行合并

```zyx
fn merge_row(row) {
    let tiles = row |> filter(fn(x) => x != 0)
    let out = []
    let score = 0
    let i = 0
    while i < len(tiles) {
        if i + 1 < len(tiles) and tiles[i] == tiles[i + 1] {
            out.push(tiles[i] * 2)
            score += tiles[i] * 2
            i += 2
        } else {
            out.push(tiles[i])
            i += 1
        }
    }
    while len(out) < len(row) { out.push(0) }
    [out, score]
}

print(merge_row([2, 2, 4, 0]))   # [[4, 4, 0, 0], 4]
```

### 10.11 运行时错误

```zyx
let arr = [1, 2, 3]
print(arr[5])
```

标准错误输出 `[line 2:10] IndexError: index 5 out of range for array of length 3`，退出码 70。

---

## 11. v1 暂不包含

以下特性不在 v1 范围内，后续视需要再加：字典/映射类型、异常捕获（`try`/`catch`）、文件读写、模块与导入、可变参数的用户函数、位运算。

---

## 附录 A：语法（EBNF）

`NL` 表示语句分隔（换行或 `;`）。关键字的中文别名在词法阶段已映射，此处只写英文。

```ebnf
program     = { NL } { stmt { NL } } EOF ;

stmt        = let_stmt | fn_decl | struct_decl | impl_decl
            | while_stmt | for_stmt | "break" | "continue"
            | return_stmt | assign_stmt | expr ;

let_stmt    = "let" IDENT [ "=" expr ] ;
assign_stmt = target ( "=" | "+=" | "-=" | "*=" | "/=" | "%=" ) expr ;
target      = IDENT | postfix "[" expr "]" | postfix "." IDENT ;
return_stmt = "return" [ expr ] ;
while_stmt  = "while" expr block ;
for_stmt    = "for" IDENT [ "," IDENT ] "in" expr block ;

fn_decl     = "fn" IDENT "(" [ params ] ")" block ;
params      = IDENT { "," IDENT } [ "," ] ;
struct_decl = "struct" IDENT "{" [ field { "," field } [ "," ] ] "}" ;
field       = IDENT [ "=" expr ] ;
impl_decl   = "impl" IDENT "{" { NL } { fn_decl { NL } } "}" ;

block       = "{" { NL } { stmt { NL } } "}" ;

expr        = or_expr ;
or_expr     = and_expr { "or" and_expr } ;
and_expr    = equality { "and" equality } ;
equality    = comparison [ ( "==" | "!=" ) comparison ] ;
comparison  = pipe [ ( "<" | "<=" | ">" | ">=" ) pipe ] ;
pipe        = range { "|>" postfix } ;
range       = additive [ ( ".." | "..=" ) additive ] ;
additive    = term { ( "+" | "-" ) term } ;
term        = unary { ( "*" | "/" | "%" ) unary } ;
unary       = ( "-" | "not" ) unary | power ;
power       = postfix [ "**" unary ] ;
postfix     = primary { call | "[" expr "]" | "." IDENT } ;
call        = "(" [ args ] ")" ;
args        = arg { "," arg } [ "," ] ;
arg         = [ IDENT ":" ] expr ;

primary     = INT | FLOAT | STRING | "true" | "false" | "nil"
            | IDENT | "(" expr ")" | array | lambda
            | if_expr | match_expr | block ;
array       = "[" [ expr { "," expr } [ "," ] ] "]" ;
lambda      = "fn" "(" [ params ] ")" ( "=>" expr | block ) ;
if_expr     = "if" expr block [ "else" ( if_expr | block ) ] ;

match_expr  = "match" expr "{" { NL } { arm ( "," | NL ) { NL } } "}" ;
arm         = pattern [ "if" expr ] "=>" ( expr | block ) ;
pattern     = single { "|" single } ;
single      = literal | "_" | IDENT
            | INT ( ".." | "..=" ) INT | INT ".."
            | ( ".." | "..=" ) INT ;
literal     = [ "-" ] INT | [ "-" ] FLOAT | STRING | "true" | "false" | "nil" ;
```

说明：

- 语句开头的 `{` 解析为块表达式。
- `fn` 后紧跟标识符时为函数声明，紧跟 `(` 时为匿名函数。
- 具名参数 `name: expr` 只对结构体构造有效，用于普通函数调用报 `TypeError`。
