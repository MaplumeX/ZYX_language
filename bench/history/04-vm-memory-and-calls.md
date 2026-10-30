# 04 虚拟机：值压缩到 16 字节、内联缓存、调用路径优化

- 日期：2026-10-31
- CPU：Intel(R) Core(TM) i9-14900HX
- 系统：Linux-6.18.33.2-microsoft-standard-WSL2-x86_64-with-glibc2.39
- Python：3.12.3
- Rust：rustc 1.97.1 (8bab26f4f 2026-07-14)
- 每个用例运行 3 次，取墙钟时间中位数（含进程启动）

倍数 = ZYX 耗时 / Python 耗时，小于 1 表示比 Python 快。

| 用例 | Python (s) | ZYX vm (s) | vm 倍数 | 输出一致 |
|---|---|---|---|---|
| fib | 0.560 | 0.897 | 1.60 | 是 |
| functional | 0.474 | 0.473 | 1.00 | 是 |
| loop | 0.952 | 1.079 | 1.13 | 是 |
| sieve | 0.446 | 1.313 | 2.94 | 是 |
| strings | 0.352 | 0.634 | 1.80 | 是 |
| structs | 0.319 | 0.477 | 1.49 | 是 |
