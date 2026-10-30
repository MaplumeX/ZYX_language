# 01 树遍历版基准（优化前）

- 日期：2026-10-31
- CPU：Intel(R) Core(TM) i9-14900HX
- 系统：Linux-6.18.33.2-microsoft-standard-WSL2-x86_64-with-glibc2.39
- Python：3.12.3
- Rust：rustc 1.97.1 (8bab26f4f 2026-07-14)
- 每个用例运行 3 次，取墙钟时间中位数（含进程启动）

倍数 = ZYX 耗时 / Python 耗时，小于 1 表示比 Python 快。

| 用例 | Python (s) | ZYX tree (s) | tree 倍数 | 输出一致 |
|---|---|---|---|---|
| fib | 0.553 | 3.232 | 5.84 | 是 |
| functional | 0.509 | 0.776 | 1.53 | 是 |
| loop | 1.016 | 4.895 | 4.82 | 是 |
| sieve | 0.449 | 3.127 | 6.96 | 是 |
| strings | 0.359 | 0.701 | 1.95 | 是 |
| structs | 0.322 | 1.841 | 5.71 | 是 |
