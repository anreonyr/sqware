#!/usr/bin/env nu

# cargo QEMU runner — riscv64gc-unknown-none-elf
# 由 .cargo/config.toml 的 target.<triple>.runner 触发，cargo 把 ELF 路径追加为位置参数。
# 可配置环境变量:
#   QEMU_EXTRA_ARGS  空格分隔的额外 QEMU 参数，追加在命令行末尾（默认: 无）
#   QEMU_GDB=1       追加 -s（GDB 监听 1234）+ -S（复位后暂停 CPU）
#   QEMU_MEM         内存大小（默认 128M）
#   QEMU_SMP         CPU 核数（默认 4）

def main [elf: path] {
    # 定位项目根: 脚本在 <root>/scripts/, 故 root = FILE_PWD 的父目录。
    # FILE_PWD 是脚本所在目录（绝对路径），与调用者 cwd 无关，不硬编码绝对路径。
    let proj_root = ($env.FILE_PWD | path dirname)

    # BIOS: 仓库根目录的预编译 SBI 文件
    let bios = ($proj_root | path join "SBI")

    # 内存 / CPU 核数: $env.X? 未设置时为 null，default 补默认值
    let mem = ($env.QEMU_MEM? | default "128")
    let smp = ($env.QEMU_SMP? | default "4")

    # icount 随机种子: 32 位随机数，配合 -icount 保证 RNG 可复现
    let seed = (random binary 4 | into int)

    # 透传额外参数: 空格字符串拆成参数列表（正则切分兼容多空格/Tab），过滤空串
    let extra = (
        $env.QEMU_EXTRA_ARGS?
        | default ""
        | split row -r '\s+'
        | where { |s| $s != "" }
    )

    # GDB 开关: QEMU_GDB=1 时加 ["-s", "-S"], 否则空列表（spread 出去无影响）
    let gdb = if ($env.QEMU_GDB? == "1") { ["-s", "-S"] } else { [] }

    # 组装参数列表（nu 的外部命令不支持 \ 续行，用数组 + spread 展开。
    # ^ 强制外部命令; 数组内 ...$list 把列表 spread 成独立参数）。
    let qemu_args = [
        "-machine", "virt"
        "-bios", $bios
        "-kernel", $elf
        "-nographic"
        "-no-reboot"
        "-m", $mem
        "-smp", $smp
        "-seed", $seed
        "-icount", "auto,sleep=on"
        ...$gdb
        ...$extra
    ]
 
    ^cargo b --all
    ^qemu-system-riscv64 ...$qemu_args
}
