# 条件必须是布尔值，没有隐式真值转换

`if`、`while`、`and`、`or`、`not` 和 `match` 守卫只接受 `bool`，其他类型报 `TypeError`。Python/JS 式的真值转换会让 `if count` 在 `count == 0` 时悄悄走错分支，这类 bug 难以发现；严格规则把它变成一条带行列号的明确报错，也让 ZYX 区别于常见脚本语言。

## Considered Options

- **Python/JS 式**：`0`、`""`、`[]`、`nil`、`false` 为假。写法最短，但继承了上述隐蔽 bug。
- **Ruby/Lua 式**：只有 `false` 和 `nil` 为假。判空方便，但 `0` 为真对多数人反直觉。

## Consequences

写惯 Python 的人和 Agent 会频繁写出 `if arr { }`、`while n { }`，开发指南必须把它列为头号易错点，并给出 `len(arr) > 0`、`n != 0`、`x != nil` 等标准写法。
