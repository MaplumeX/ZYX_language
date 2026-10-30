# 用 Rust 实现 ZYX 解释器

解释器需要在性能上和 CPython 对比，候选有 C、C++、Rust 和 Python。选 Rust：性能与 C/C++ 同级，`enum` 适合表示动态类型的值，`Vec`/`HashMap`/`String` 省去手写基础设施，借用检查在编译期拦住大部分内存错误。代码主要由编程 Agent 编写，编译期安全比"写得快"更重要。

## Considered Options

- **C**：性能最高、有 clox 可参考，但要手写动态数组、哈希表和垃圾回收，悬空指针类 bug 排查成本高。
- **C++**：有标准库容器，但语言特性复杂，内存安全仍靠自觉。
- **Python**：开发最快，但会比 CPython 慢 50 到 100 倍，性能对比失去意义。

## Consequences

共享可变值（数组、结构体、闭包捕获的变量）用 `Rc<RefCell<...>>` 表示，暂不回收循环引用。
