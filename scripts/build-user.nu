#!/usr/bin/env nu
# 重建全部用户程序并拷为提交产物 user/*.elf（内核 boot.rs 按 include_bytes 内嵌）。
# 用户源码变更后：nu scripts/build-user.nu → cargo build（内核）重链。
# 不直接做进 kernel/build.rs（嵌套 cargo 会撞 target 锁）；产物按路径交内核嵌入。

def main [] {
    # 编译
    print "Building user package..."
    cargo build -p user

    let bins = [exiter counter yielder sleeper threader heaper]
    let target_dir = "target/riscv64gc-unknown-none-elf/debug"

    mkdir user

    for bin in $bins {
        let src = $target_dir | path join $"user-($bin)"
        let dst = "user" | path join $"user-($bin).elf"
        if ($src | path exists) {
            cp --verbose $src $dst
        } else {
            error make {
                msg: $"Binary ($src) does not exist. Check Cargo.toml bin names."
            }
        }
    }
    print "All binaries copied successfully."
}
