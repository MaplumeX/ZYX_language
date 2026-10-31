# 2048 开发日志

开发者：编程 Agent。关于 ZYX 语言只参考了 `.claude/skills/zyx/SKILL.md`。

### 1. 核心逻辑与自测模式（app(2048): 核心逻辑：单行合并、四向移动、可行移动判定、生成新方块与自测）

- **目标**：先实现与界面无关的核心函数：`merge_line`（单行合并）、`move_left/right/up/down`、`can_move`、`spawn_tile`，并在首行输入 `test` 时用 `assert_eq` 自测。
- **做法**：用 `LineResult`、`MoveResult` 两个结构体返回"结果 + 得分 + 是否移动"，因为 ZYX 没有元组（SKILL.md 第 3 节"不存在的东西"）。合并时先用管道 `line |> filter(fn(x) => x != 0)` 去掉空格，再用 `while` 扫描、相邻相等就合并并跳过两个；右移 = 每行反转后左移再反转，上/下移 = 转置后左/右移再转置。`move_board` 用 `match` 按方向分派。`array(SIZE, array(SIZE, 0))` 建二维数组（SKILL.md 第 3 节说明每项是独立副本），棋盘复制用 `map(copy)`，因为数组是引用语义（第 1 节"其他常见坑"）。`spawn_tile` 用 `random(1, 10) == 1` 实现 10% 概率出 4。自测覆盖 `[2,2,2,2]→[4,4,0,0]`、`[4,4,8,0]→[8,8,0,0]`、四个方向、原棋盘不被修改、无效移动、无路可走、生成新方块的分布。查阅了第 1 节 12 条、第 2 节语法速查（结构体、match、管道）、第 3 节内置函数表。
- **问题**：无。`printf 'test\n' | ./target/release/zyx examples/app/2048.zyx` 第一次运行即输出 `self-test passed`，`--backend vm` 结果相同。
- **解决**：无需修复。
- **文档反馈**：`input(prompt?)` 没有说明 prompt 在 stdin 不是终端时是否也会输出、输出后是否换行，写交互界面前需要自己试一下。
