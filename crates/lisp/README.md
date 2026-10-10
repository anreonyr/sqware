# Lisp

独立 `no_std + alloc` 语言库。前端按 Source → Logos token → S 表达式 arena → 核心表达式分层；运行时包含值、堆、环境、可暂停 Machine 和纯原语。没有系统协议依赖。

公开入口是 Reader、Engine、Limits、Step、RootValue 和宿主请求／恢复接口。内核句柄与续体不公开。源码与代码按存活闭包持有，非移动 GC 回收闭包环境环；宿主根和暂停参数受保护。

```sh
cargo test -p lisp --target x86_64-unknown-linux-gnu
cargo check -p lisp --target riscv64gc-unknown-none-elf
```

完整语义、Shell 装配、限制和实例见 [Lisp Shell](../../docs/lisp-shell.md)。
