#!/usr/bin/env nu
# 重建全部用户程序并拷为提交产物 user/*.elf（内核 boot.rs 按 include_bytes 内嵌）。
# 用户源码变更后：nu scripts/build-user.nu → cargo build（内核）重链。
# 不直接做进 kernel/build.rs（嵌套 cargo 会撞 target 锁）；产物按路径交内核嵌入。

cargo build -p user
for bin in [exiter counter yielder sleeper threader] {
    cp target/riscv64gc-unknown-none-elf/debug/user-$bin user/user-$bin.elf
}
