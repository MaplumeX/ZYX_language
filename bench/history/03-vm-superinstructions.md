# 03 虚拟机：超级指令与窥孔优化

- 日期：2026-10-31
- CPU：Intel(R) Core(TM) i9-14900HX
- 系统：Linux-6.18.33.2-microsoft-standard-WSL2-x86_64-with-glibc2.39
- Python：3.12.3
- Rust：rustc 1.97.1 (8bab26f4f 2026-07-14)
- 每个用例运行 3 次，取墙钟时间中位数（含进程启动）

倍数 = ZYX 耗时 / Python 耗时，小于 1 表示比 Python 快。

| 用例 | Python (s) | ZYX vm (s) | vm 倍数 | 输出一致 |
|---|---|---|---|---|
| fib | 0.572 | 0.972 | 1.70 | 是 |
| functional | 0.497 | 0.517 | 1.04 | 是 |
| loop | 0.919 | 1.143 | 1.24 | 是 |
| sieve | 0.437 | 1.526 | 3.49 | 是 |
| strings | 0.332 | 0.537 | 1.62 | 是 |
| structs | 0.310 | 0.960 | 3.10 | 是 |
