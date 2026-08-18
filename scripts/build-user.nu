#!/usr/bin/env nu
# 重建用户程序并拷为提交产物 user/user-exiter.elf（内核 boot.rs include_bytes 内嵌）。
# 用户源码变更后：nu scripts/build-user.nu → cargo build（内核）重链。
# 不直接做进 kernel/build.rs（嵌套 cargo 会撞 target 锁）；产物按路径交给内核嵌入。

cargo build -p user --bin user-exiter
cp target/riscv64gc-unknown-none-elf/debug/user-exiter user/user-exiter.elf
