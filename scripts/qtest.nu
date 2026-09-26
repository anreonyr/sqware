#!/usr/bin/env nu

# 跑 `embedded-test` 用例（内核内）。
#
# 它**不是**一套测试框架，只是一层转发：把 [`qemu-args.nu`](qemu-args.nu) 那张**板子参数表**
# 翻成 runner 认的 `--qemu-arg=`，再交给 `cargo qtest`。
# 参数表仍**只有一处**——本文件不含任何一个板子参数的字面量，只把**两格**按整机用例的需要
# 换掉：`-initrd`（参数表在 `--board-only` 下让出它）与**串口那一格**（经参数表的
# `--serial` 口换，见下）。
#
# 用法：
#   nu scripts/qtest.nu                              # 跑当前 manifest 的全部用例
#   nu scripts/qtest.nu --package kernel             # 指定包（工作区里用）
#   nu scripts/qtest.nu --package kernel --scene rig # 造 rig 景、带它跑整机那一例
#   nu scripts/qtest.nu --package kernel --scene root --feed exit
#   nu scripts/qtest.nu -- --list                    # `--` 之后原样转给 cargo-qtest
#
# **一例 = 一张镜像，一次运行 = 一个景**：`cargo-qtest` 没有逐例过滤器（`--help` 里只有
# `--test <目标名>`——那是**测试目标**名，不是用例名），而 `--qemu-arg=` 是**整次运行**的
# ⇒ 一张镜像只能服务一轮。故整机用例只有**一例**（`tests/embedded.rs` 的 `scene`），
# 跑七个景就是七次调用。`--scene` 给的景名由本脚本打印出来——报告里那一行只说 `scene`，
# 景在这一行。
#
# # `--scene` 时：串口搬到一条**我们能喂输入的**通道上
#
# **照实记（为什么非搬不可）**：`cargo-qtest` 把 QEMU 的 stdin 钉成 `Stdio::null()`
# （它源码 `qemu.rs` 那一行）⇒ 往 `cargo qtest` 里灌 stdin **一个字节也到不了 QEMU**。
# 而整机收场的扳机是 `echo`（装配单位次 18，最后一条）——它的退场由**控制台输入**驱动。
# 故依赖输入日程的景（`root` / `product`）原先在内核内判据上**判不了**：它们偶尔能收场，
# 是因为探针先 panic、级联把 `echo` 扑杀，那条道才响。
#
# 三段拼起来（都在 `<TRACE_OUT>/` 里，一次运行生成一次）：
#
#   ① **串口那一格**经参数表换成 `tcp:127.0.0.1:<端口>`，**QEMU 当 client**，
#      帮手（`scene-feed.py`）当 listener、端口由它自己 bind(0) 后写进 `scene-feed.port`
#      ——**监听套接字常驻**，于是"上一个用例的 TIME_WAIT"挡不住下一个（实测：让 QEMU 当
#      server 时，第二例起 `Address already in use`，十例里九例秒红）。
#   ② **命令行改写器**（`scene-qemu.sh`，经 `cargo qtest --qemu` 递进去）：**只摘掉
#      `cargo-qtest` 硬编码的 `-nographic`**。那一手会**隐式**给 serial0 绑 stdio
#      （"除非已被显式改写"只认它**前面**出现的 `-serial`），于是参数表那一枚排到 serial1，
#      而 guest 只跟 UART0（`0x10000000`）说话 ⇒ 一个字节都收不到（实测：捕获全空）。
#   ③ **帮手**：每 accept 一次 ＝ 一例的 QEMU 连上了（用例是**串行**的——`cargo-qtest`
#      的 `for test in tests`），把 guest 控制台追进捕获文件，并**每 2 s 重喂一次**
#      `--feed`（默认 `exit`）直到写不进去（那一例结束）。
#
#   `--scene` 不给 ⇒ 这三段全不启用，串口还是 `stdio`。
#
# **照实记（第一版走不通的两条）**：
#   ① "板子参数照旧 ＋ 末尾追加 `-serial tcp:…`"——命令行里于是有**两枚 `-serial`**
#      （QEMU 当 serial0/serial1），十例里九例当场红。
#   ② "经参数表只留一枚、QEMU 当 server（`server=on,wait=on`）"——第二例起
#      `Address already in use`：QEMU 不设 `SO_REUSEADDR`，上一个用例的 TIME_WAIT
#      挡着端口。故改成**帮手当 listener、QEMU 当 client**：监听套接字常驻，TIME_WAIT
#      落在 client 侧，不挡 listener。
#
# **照实记（机侧控制台其实一直留在 dump 里——我先前写错了一半）**：曾以为"串口搬走之后
# `cargo-qtest` 那份 dump 就没有机侧输出了"。量下来不是：QEMU 带着
# `-semihosting-config enable=on,target=native` 时，**控制台走的是 semihosting**
# （落在 QEMU 的 stdout，`cargo-qtest` 照旧抓得到——失败 dump 里 `system: gone …` 一行
# 不少）。我们那条串口因此是一根**只喂不读**的输入线，捕获文件里通常只有连接标记。
# 保留"失败时端出捕获尾"那一手只为防串口路由被改动，别指望它有机侧输出。
#
# **读数（量出来的）**：七个景**全绿**（release 档、每景 1 例）——
#   root 5.28 s · product 5.23 · again 2.17 · load 0.72 · group 0.37 · beat 2.22 · rig 3.33。
#   `--scene product --feed list`（不喂 `exit`）→ **FAILED**：喂入是承重的，不是巧合。
#   不给 `--scene`（debug 档）→ **9 passed · 1 failed**：健康面九例由默认那轮覆盖，
#   整机那一例报红（无镜像哨，响得出来）。
#
# **照实记（整机用例为什么跑 release）**：同一个 `root` 景，**release 产品路 6/6 稳、
# 14 笔结局**；**debug 产品路结局笔数 5 / 11 / 6 / 11 乱跳、偶发 panic** ⇒ debug 档下
# 这条世界本来就不可靠（`rig` 的照实记早写过"debug 下每轮都挂在 20 ms 那一缝上"）。
# 而测试目标原先**只能在 debug 下编**（`kernel::health::*` 是 `#[cfg(debug_assertions)]`，
# release 档 `cannot find `spare` in `health``）⇒ 那个 flaky 是**档**的问题，不是判据的
# 问题。把健康面那九例一并 gate 进 `debug_assertions` 之后，release 档的测试目标只剩整机
# 那一例，于是它能跑在与产品路**同一个档**上——顺带每景从十几秒降到一秒级。
#
# **档那一格的裁决（尾账收口）**：**整机一律 release**——`cargo image` 的 `--profile` 默认
# release，测试目标 `--scene` 带 `-r`。这不是哪个消费者的偏好，是**世界**的性质（见上）。
# `debug` 档留给**自检**（健康面那九例、内核启动自检）与单元级核对；整机跑 debug 今天会
# 给出一台**起不完**的机器（结局笔数 5 / 11 / 6 而非 14）。
#
# **照实记（归档这一格）**：测试路**没有结构化导出**（内核那个 `semihosting` feature 不在
# 测试构建里），故这一轮的现场就是 `cargo qtest` 自己的输出——机侧控制台走 semihosting
# 落在它里面。纪律与 `runner.nu` 同款：**失败留档**到 `<TRACE_OUT>/scene-<景>-qtest.log`
# （无 `--scene` 那轮叫 `qtest.log`），正常那轮丢掉，不留一堆绿的日志。
#
# **先装那个 runner**（它是个宿主工具，不在仓里）：
#
#   cargo install cargo-qemu-test --target x86_64-unknown-linux-gnu   # ⇒ cargo-qtest
#
# **照实记（`--target` 那一格是必须的）**：本工作区的 `.cargo/config.toml` 把
# `[build] target` 钉在 riscv 上，而 `cargo install` **也吃这一格** ⇒ 不带
# `--target x86_64-unknown-linux-gnu` 会拿 riscv 去编这个宿主工具，编出来一堆
# `cannot find trait PartialEq`（`std` 不在场）。实测踩过。
# **照实记（`--smp` 那一格原本默认成 1，已撤）**：原先这里写"`-smp 4` 必炸——`_start`
# 每颗 hart 都会跑一遍 ⇒ 4 颗一起冲进 `__embedded_test_start`、并发读 semihosting 命令行"。
# **量下来那句是错的**：它只对 `-bios none`（M 态、所有 hart 从复位向量起跑）成立，
# 而参数表给的是 `-bios SBI.bin`——S 态下**只有引导 hart 进内核**，副核由
# `boot::boot_harts()` 经 HSM 拉起（**产品路本来就是 `-smp 4`**，`rig`/`soak` 的判据
# 前提正是"空核替全局兑现到点"）。实测 `QEMU_SMP=4` 下原八例 8/8 绿、1.92 s。
# 故这里不再改 `-smp` **默认**，跟参数表走；`QEMU_SMP` 显式设了当然也听调用方的。
# 那条结论有钉子：`kernel/src/health/hart.rs` 的 `hart_multi` 用例。
# **照实记（`icount` 那一格仍与"起机那条路"不同）**：这里把 `icount` 置空。旧的门
# 统一关掉它（"按宿主时间节流会让 guest 与输入日程失步"，见 `qemu-args.nu` 的头注）；
# 用例要在同一档下可比，故这里也关。

# 喂日程的帮手（`--scene` 时落到 `<TRACE_OUT>/scene-feed.py` 再起）。
# 参数：端口文件 / 捕获文件 / 要重喂的那一句。**listener**，QEMU 以 client 身份连上来。
# 用 python 只为"同一条连接上既读又写"这件事；bash 做不了 listener，`nc` 那套要另装工具。
const FEED = '#!/usr/bin/env python3
"""喂日程的帮手：连上一次 ＝ 一例；断开 ＝ 那一例结束，回去等下一例。"""
import socket, sys, threading, time

port_file, cap, text = sys.argv[1], sys.argv[2], sys.argv[3]
after = float(sys.argv[4]) if len(sys.argv) > 4 else 5.0
srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("127.0.0.1", 0))
srv.listen(8)
with open(port_file, "w") as f:
    f.write(str(srv.getsockname()[1]))

while True:
    conn, _ = srv.accept()
    with open(cap, "ab") as f:
        f.write("\n──── 一例 ────\n".encode())

    def tee(c=conn):
        while True:
            try:
                d = c.recv(65536)
            except OSError:
                return
            if not d:
                return
            with open(cap, "ab") as f:
                f.write(d)

    threading.Thread(target=tee, daemon=True).start()
    time.sleep(after)
    try:
        while True:
            conn.sendall((text + "\n").encode())
            time.sleep(2)
    except OSError:
        pass
    try:
        conn.close()
    except OSError:
        pass'

# QEMU 命令行改写器（`--scene` 时落到 `<TRACE_OUT>/scene-qemu.sh`，经 `cargo qtest --qemu` 递）。
# **只摘掉 `cargo-qtest` 硬编码的 `-nographic`**——它会隐式把 serial0 绑到 stdio，把我们那一枚
# `-serial` 挤到 serial1（guest 只跟 UART0 说话）。其余参数原样透传。
const WRAPPER = '#!/usr/bin/env bash
args=()
for a in "$@"; do
  [ "$a" = "-nographic" ] || args+=("$a")
done
exec qemu-system-riscv64 "${args[@]}"'

def main [--package: string, --scene: string, --profile: string, --feed: string, --feed-after: int = 5, ...rest: string] {
  if ($env.QEMU_ICOUNT? | is-empty) { $env.QEMU_ICOUNT = "" }

  let script_dir = $env.FILE_PWD
  let repo = ($script_dir | path dirname)
  let trapdir = ($env.TRACE_OUT? | default ($repo | path join "trace"))

  mut cap = ""
  mut scene_args = []
  mut serial = ""
  mut wrapper = ""

  # 景：先造镜像，再把 `-initrd` 指过去（**绝对路径**——runner 的工作目录不是仓根），
  # 并把串口换到一条我们能喂输入的通道上（见头注）。
  # 档默认 release：`cargo image` 自己也是这个默认，而 `rig` 的照实记说 release 是
  # 它跑得动的前提（debug 下每轮都挂在 20 ms 那一缝上）。
  if not ($scene | is-empty) {
    let prof = if ($profile | is-empty) { "release" } else { $profile }
    let at = (
      ^cargo image $scene $prof
      | lines
      | where { |l| ($l | str starts-with "ok") }
      | last
      | str replace "ok" ""
      | str trim
    )
    if not ($at | path exists) {
      error make {msg: $"造不出镜像：cargo image ($scene) ($prof) 说落在 ($at)"}
    }
    print $"景 ($scene)（档 ($prof)）· 镜像 ($at)"
    $scene_args = ["--qemu-arg=-initrd" $"--qemu-arg=($at)"]

    mkdir $trapdir
    $cap = ($trapdir | path join $"scene-($scene)-feed.log")
    let helper = ($trapdir | path join "scene-feed.py")
    let pid_file = ($trapdir | path join "scene-feed.pid")
    let port_file = ($trapdir | path join "scene-feed.port")
    let err_file = ($trapdir | path join "scene-feed.err")
    let text = if ($feed | is-empty) { "exit" } else { $feed }
    $FEED | save --force $helper
    rm -f $port_file
    $env.FEED_HELPER = $helper
    $env.FEED_PID = $pid_file
    $env.FEED_PORTFILE = $port_file
    $env.FEED_CAP = $cap
    $env.FEED_TEXT = $text
    $env.FEED_AFTER = ($feed_after | into string)
    $env.FEED_ERR = $err_file
    # 上一轮万一留了帮手，先收掉（它自己会一直 listen）。
    if ($pid_file | path exists) {
      ^bash -c 'p=$(cat "$FEED_PID"); kill "$p" 2>/dev/null; true'
    }
    ^bash -c 'nohup python3 "$FEED_HELPER" "$FEED_PORTFILE" "$FEED_CAP" "$FEED_TEXT" "$FEED_AFTER" >"$FEED_ERR" 2>&1 & echo $! > "$FEED_PID"'
    # 端口由帮手 bind(0) 后报出来（有界等；它没起来就当场说清楚，不要等到十例都超时）。
    let got = (^bash -c 'for i in $(seq 1 50); do if [ -s "$FEED_PORTFILE" ]; then cat "$FEED_PORTFILE"; exit 0; fi; sleep 0.1; done; exit 1' | complete)
    if ($got.exit_code != 0) {
      print $"喂入帮手没起来（($err_file)）："
      ^cat $err_file
      error make {msg: "喂入帮手没起来——python3 在不在 PATH 里？见上面那份 stderr"}
    }
    $serial = $"tcp:127.0.0.1:($got.stdout | str trim)"

    # 改写器：`cargo qtest` 要一个**能直接执行**的路径。
    $wrapper = ($trapdir | path join "scene-qemu.sh")
    $WRAPPER | save --force $wrapper
    $env.QEMU_WRAPPER = $wrapper
    ^bash -c 'chmod +x "$QEMU_WRAPPER"'
    print $"喂入 ($text)（重喂到机器退）· 串口 ($serial) · 捕获 ($cap)"
  }

  # 参数表**唯一出处**：串口那一格也归它（`--serial`），本文件不写板子字面量。
  let serial_argv = if ($serial | is-empty) { [] } else { ["--serial" $serial] }
  let board = (^nu ($script_dir | path join "qemu-args.nu") --board-only ...$serial_argv
      | lines | where { |l| ($l | str trim) != "" })
  let qemu_args = ($board | each { |a| $"--qemu-arg=($a)" })
  let pkg = if ($package | is-empty) { [] } else { ["--package" $package] }
  let qemu_bin = if ($wrapper | is-empty) { [] } else { ["--qemu" $wrapper] }
  # 整机用例跑 **release**：同一个 `root` 景，release 产品路 6/6 稳（14 笔结局），debug 产品路
  # 结局笔数 5 / 11 / 6 / 11 乱跳、偶发 panic ⇒ debug 档下这条世界本来就不可靠。健康面那九例
  # 是 `#[cfg(debug_assertions)]`（它们的身子在 `kernel::health` 里），故 release 档的测试
  # 目标只剩整机那一例——正好，也快。
  let test_profile = if ($scene | is-empty) { [] } else { ["-r"] }

  # **归档（O10）**：`cargo qtest` 的输出**就是**这一轮的现场——机侧控制台走 semihosting
  # 落在它的 stdout 里（见头注），而测试路没有结构化导出（内核那个 `semihosting` feature
  # 不在测试构建里）。故与 `runner.nu` 同款纪律：**失败留档、正常那轮丢掉**。
  # `o+e>| tee { save }` 既保终端流、又保退出码（`try` 那两条地雷见 `runner.nu` 头注）。
  mkdir $trapdir
  let qtlog = ($trapdir | path join (if ($scene | is-empty) { "qtest.log" } else { $"scene-($scene)-qtest.log" }))
  # 命令与重定向**必须同一行**（分行 ⇒ `nu::parser::unexpected_redirection`）。故先把 argv
  # 拼好，再一行发出去——退出码仍由 `LAST_EXIT_CODE` 拿（`try` 那两条地雷见 `runner.nu` 头注）。
  let qtest_argv = ["qtest" "--target" "riscv64gc-unknown-none-elf" ...$test_profile ...$qemu_bin ...$qemu_args ...$scene_args ...$pkg ...$rest]
  try { ^cargo ...$qtest_argv o+e>| tee { save --force $qtlog } } catch { }
  let code = $env.LAST_EXIT_CODE

  if ($qtlog | path exists) {
    if $code == 0 {
      rm --force $qtlog
    } else {
      print $"现场（cargo qtest 全份）-> ($qtlog)"
    }
  }

  if not ($scene | is-empty) {
    ^bash -c 'p=$(cat "$FEED_PID"); kill "$p" 2>/dev/null; true'
    # 串口是一根"只喂不读"的输入线；机侧控制台仍走 semihosting（见头注）。这一手是为
    # 串口路由万一被改动时留的后手——读不到东西是正常的。
    if $code != 0 and ($cap | path exists) {
      print $"（喂入侧捕获：($cap)，下面是尾 40 行）"
      ^tail -40 $cap
    }
  }
  exit $code
}
