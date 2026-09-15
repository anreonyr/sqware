#!/usr/bin/env nu

# sqware 验收门（新基线版）——**五条判据，缺一不可**：
#
#   1) 自行退出：qemu 自己结束（域退场 ⇒ root 收场 ⇒ 级联 ⇒ 停机走 srst）⇒
#      外接 timeout 的退出码**不是 124**。
#   2) 无崩溃：捕获里没有 `[panic] at`（内核 panic 报告头）。
#   3) `echo: ready` —— 域能说一句话，且**不依赖任何服务**（调试面直通固件）。
#   4) 回显：喂一行进去，同一行回来 —— 能收能回。
#   5) `task: all tasks exited, system halted` —— 收场那一句（关机的唯一判据）。
#
# 这就是**全部**：新基线里没有服务、没有驱动、没有孔、没有协议，
# 故"逐步生效 / marker 齐全 / 实例计数"那一套随旧树一起清了。
#
# 用法：scripts/examine.nu            # 默认连跑 3 次，要求 3/3
#       scripts/examine.nu --repeat 5
#
# 现场一律留在 trace/gate-<时间戳>/runN.log：门红了要有东西可看。
#
# # 输入怎么喂
#
# 不逐步 expect（那需要长驻写端 + 活读日志，是旧门复杂度的大头），改成**定时喂**：
# 调试面读的是固件的串口，**字节先进 UART 的 FIFO**（16 字节），guest 什么时候来读都算数。
# 故两句输入都很短（`ping` / `exit`，合计 9 字节 < FIFO），早喂、晚喂都不丢。

const PING = "ping"
const EXIT = "exit"
const READY = "echo: ready"
const HALT = "task: all tasks exited, system halted"
const PANIC = "[panic] at"
const ECHO_TIMEOUT = 60

# 喂给 guest 的那串动作：等启动 → 一行 → 隔一会儿 → `exit` → 留够关机时间。
const FEED = 'sleep 5; printf "ping\n"; sleep 3; printf "exit\n"; sleep 15'

def main [--repeat: int = 3] {
    let root = ($env.FILE_PWD | path dirname)
    let elf = ($root | path join "target/riscv64gc-unknown-none-elf/release/sqware")
    if not ($elf | path exists) {
        print $"examine: 找不到 ($elf) —— 先 `cargo build --release`"
        exit 2
    }
    let out = ($root | path join $"trace/gate-(date now | format date '%Y%m%d-%H%M%S')")
    mkdir $out

    print $"examine: repeat=($repeat) elf=($elf) out=($out)"
    print $"examine: 判据 = 自退 · 无 panic · ($READY) · 回显 `($PING)` · ($HALT)"

    mut pass = 0
    for i in 1..$repeat {
        let log = ($out | path join $"run($i).log")
        # 验收门**关掉 icount**：按宿主时间节流会让 guest 与输入日程失步（旧门实测过）。
        let r = (with-env { QEMU_ICOUNT: "" } {
            ^bash -c $FEED
            | ^timeout $ECHO_TIMEOUT nu ($root | path join "scripts/boot.nu") $elf
            | complete
        })
        let text = ($r.stdout | str join "\n")
        $text | save --force $log
        let rc = $r.exit_code

        let why = (
            [
                (if $rc == 124 { "被超时杀" } else { "" })
                (if ($text | str contains $PANIC) { "有 panic" } else { "" })
                (if ($text | str contains $READY) { "" } else { $"缺[($READY)]" })
                (if (has_line $text $PING) { "" } else { $"缺回显[($PING)]" })
                (if ($text | str contains $HALT) { "" } else { $"缺[($HALT)]" })
            ] | where { |s| $s != "" }
        )
        if ($why | is-empty) {
            $pass = $pass + 1
            print $"run ($i): PASS"
        } else {
            print $"run ($i): FAIL — ($why | str join ' + ') —— 现场 ($log)"
        }
    }

    print $"examine: ($pass)/($repeat)"
    if $pass != $repeat { exit 1 }
}

# 某一整行是不是 `needle`（回显的判据必须是**整行相等**：`ping` 这种短词很容易
# 在别处蹭到一次子串命中，那样这条判据就等于没有）。
def has_line [text: string, needle: string] {
    ($text | lines | any { |l| ($l | str trim) == $needle })
}
