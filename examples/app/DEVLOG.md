# 2048 开发日志

开发者：编程 Agent。关于 ZYX 语言只参考了 `.claude/skills/zyx/SKILL.md`。

### 1. 核心逻辑与自测模式（app(2048): 核心逻辑：单行合并、四向移动、可行移动判定、生成新方块与自测）

- **目标**：先实现与界面无关的核心函数：`merge_line`（单行合并）、`move_left/right/up/down`、`can_move`、`spawn_tile`，并在首行输入 `test` 时用 `assert_eq` 自测。
- **做法**：用 `LineResult`、`MoveResult` 两个结构体返回"结果 + 得分 + 是否移动"，因为 ZYX 没有元组（SKILL.md 第 3 节"不存在的东西"）。合并时先用管道 `line |> filter(fn(x) => x != 0)` 去掉空格，再用 `while` 扫描、相邻相等就合并并跳过两个；右移 = 每行反转后左移再反转，上/下移 = 转置后左/右移再转置。`move_board` 用 `match` 按方向分派。`array(SIZE, array(SIZE, 0))` 建二维数组（SKILL.md 第 3 节说明每项是独立副本），棋盘复制用 `map(copy)`，因为数组是引用语义（第 1 节"其他常见坑"）。`spawn_tile` 用 `random(1, 10) == 1` 实现 10% 概率出 4。自测覆盖 `[2,2,2,2]→[4,4,0,0]`、`[4,4,8,0]→[8,8,0,0]`、四个方向、原棋盘不被修改、无效移动、无路可走、生成新方块的分布。查阅了第 1 节 12 条、第 2 节语法速查（结构体、match、管道）、第 3 节内置函数表。
- **问题**：无。`printf 'test\n' | ./target/release/zyx examples/app/2048.zyx` 第一次运行即输出 `self-test passed`，`--backend vm` 结果相同。
- **解决**：无需修复。
- **文档反馈**：`input(prompt?)` 没有说明 prompt 在 stdin 不是终端时是否也会输出、输出后是否换行，写交互界面前需要自己试一下。

### 2. 游戏状态结构体、棋盘显示与命令循环（app(2048): 游戏状态结构体、棋盘显示与命令循环）

- **目标**：用结构体 `Game` 保存棋盘、分数、最高分、步数，用 `impl` 实现 `reset`、`render`、`step`；实现读命令的主循环，支持 `w/a/s/d`（大小写）、`r`、`h`、`q`、未知命令提示、无效移动提示、输入结束退出，以及首行 `seed N`。
- **做法**：`Game` 字段带默认值（`score = 0` 等，SKILL.md 第 2 节"结构体与方法"）；`render` 用 `pad_left(text, 5)` 让数字右对齐、格宽一致，空格显示 `.`；边框用 `"------+" * SIZE`（第 1 节"其他常见坑"说明字符串在左边才能乘）。为遵守第 1 条第 8 项"插值里不能有字符串字面量"，先 `let line = join(cells, "|")` 再插值。`direction_of` 用 `match` 把按键映射为方向并以 `_ => nil` 兜底。主循环用 `input("> ")`，返回 `nil` 时打印最终分数后退出；`seed` 参数用 `parse_int` 校验。先试验了 `input(prompt)` 的行为：stdin 是管道时提示符也会输出且不换行。
- **问题**：无。`printf 'seed 1\na\nD\nw\nxyz\nh\ns\nr\nq\n' | ./target/release/zyx examples/app/2048.zyx` 第一次运行即符合预期：计分、步数、最高分在 `r` 后保留、未知命令提示、帮助、退出都正常；`seed 1` 后连按 `w` 得到 5 次"无效移动"提示且步数不增加；树遍历和 `--backend vm` 输出 `diff` 一致。
- **解决**：无需修复。
- **文档反馈**：无。
