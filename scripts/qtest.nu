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
#   nu scripts/qtest.nu --package kernel --scene accept
#   nu scripts/qtest.nu --package kernel --scene accept --feed-script programs/tests/terminal/session.py
#   nu scripts/qtest.nu --package kernel --scene memory-transfer --feed-script programs/tests/memory-transfer/driver.py -- --features semihosting
#   nu scripts/qtest.nu --package kernel --scene memory-transfer-tlb --feed-script programs/tests/memory-transfer/tlb.py -- --features semihosting
#   CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true nu scripts/qtest.nu --package kernel \
#     --scene memory-transfer-fresh --feed-script programs/tests/memory-transfer/fresh.py \
#     -- --features semihosting,fresh-scene -- scene
#   nu scripts/qtest.nu -- --list                    # `--` 之后原样转给 cargo-qtest
#
# **（`memory-transfer-fresh` 另需带护栏那一档 ＋ 两格 feature）**：那一景靠一个在途夹具
# 注入一次撤页确认失败，而夹具与那四个探针同属 `cfg(debug_assertions)` ⇒ 不带
# `CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true` 就没有夹具体，那一次移交会直接成功，
# 本景当场红（程序侧那一条 `Busy` 断言就是硬闸）。`fresh-scene` 是它的**显式选景**
# （默认关着，D1/TLB/accept/product 连夹具体都没有），`semihosting` 仍是为结构化导出。
# `-- scene` 那一格不能省：带护栏的 release 测试目标里有全部 `debug_assertions` 用例，
# 一例一张镜像、整机用例只有 `scene` 这一例。
#
# **（`memory-transfer` 与 `memory-transfer-tlb` 两景必须带 `--features semihosting`）**：
# 两景的判定都要读**结构化导出**（`sqware-diagnose.jsonl` 里的 `room.wait`/`room.wake`/
# `memory.page_fault`），而那个导出只在内核 `semihosting` feature 打开时才落盘
# （`kernel/Cargo.toml` 非默认）。不带这一格，驱动会因缺事件而超时失败——正是该有的样子，
# 不降断言。
#
# **（不带 `--scene` 那一轮，`scene` 那一例必红——那是设计）**：`scene` 的判据是"这台
# 机器真的起过一次"，而**镜像由 `--scene` 那一格给**（`-initrd`）；不带景就没有镜像，
# `kernel/src/entry/boot/mod.rs` 在测试模式下报告缺失镜像（宁可红，也不要"静默起一台没有
# 程序的机器"）。故"全部用例绿"的跑法是**两轮**：
#
#   nu scripts/qtest.nu --package kernel            # 健康面（十四例，只在 debug 档）
#   nu scripts/qtest.nu --package kernel --scene accept   # 整机那一例
#
# **一例 = 一张镜像，一次运行 = 一个景**：`cargo-qtest` 没有逐例过滤器（`--help` 里只有
# `--test <目标名>`——那是**测试目标**名，不是用例名），而 `--qemu-arg=` 是**整次运行**的
# ⇒ 一张镜像只能服务一轮。故整机用例只有**一例**（`tests/embedded.rs` 的 `scene`），
# 跑七个景就是七次调用。`--scene` 给的景名由本脚本打印出来——报告里那一行只说 `scene`，
# 景在这一行。
#
# **验收要重复跑，失败要按签名定位**：早期记录过 `accept` 35 跑红 5 跑。
# 2026-10-09 复查复现了 Control 重复扫描能力表拖过 1 秒握手期限，以及 Terminal
# 按下标发现能力时的漏读。修复分别改为每轮一次候选扫描、响应显式交付数据端点。
# 此处记录的是历史复查，不作为本轮重构的验证结果；每轮实际结果以执行输出为准。
#
# # `--scene` 时：串口搬到一条**我们能喂输入的**通道上
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
#      `--feed`（默认 EOF 字节）直到写不进去（那一例结束）。
#
#   `--scene` 不给 ⇒ 这三段全不启用，串口还是 `stdio`。
#
# **读数（量出来的）**：七个景**全绿**（release 档、每景 1 例）——
#   accept 5.28 s · product 5.23 · again 2.17 · load 0.72 · group 0.37 · beat 2.22 · rig 3.33。
#   `--scene product --feed list`（不喂 `exit`）→ **FAILED**：喂入是承重的，不是巧合。
#   不给 `--scene`（debug 档）→ **14 passed · 1 failed**：健康面那十四例由默认那轮覆盖，
#   整机那一例报红（无镜像哨，响得出来）。
#
# **档那一格的裁决（尾账收口）**：**整机一律 release**——`cargo image` 的 `--profile` 默认
# release，测试目标 `--scene` 带 `-r`。这不是哪个消费者的偏好，是**世界**的性质（见上）。
# `debug` 档留给**自检**（健康面那十四例、内核启动自检）与单元级核对；整机跑 debug 今天会
# 给出一台**起不完**的机器（结局笔数 5 / 11 / 6 而非 14）。
#
# **（归档这一格）**：测试路**没有结构化导出**（内核那个 `semihosting` feature 不在
# 测试构建里），故这一轮的现场就是 `cargo qtest` 自己的输出——机侧控制台走 semihosting
# 落在它里面。纪律与 `runner.nu` 同款：**失败留档**到 `<TRACE_OUT>/scene-<景>-qtest.log`
# （无 `--scene` 那轮叫 `qtest.log`），正常那轮丢掉，不留一堆绿的日志。
#
# **（但那一份是"尾若干行"）**：`cargo qtest` 在失败时只印 QEMU 输出的**尾巴**——超时那一档
# 尾巴里恰好没有"机器跑到哪一步停住"。要全份就设 **`QEMU_TEE_LOG=<路径>`**：外壳顺路拷一份
# 全控制台（QEMU 的 stdout 原样过 tee，两边都留）。**不设它逐字与从前相同**（`exec` 那一支）。
#
# **先装那个 runner**（它是个宿主工具，不在仓里）：
#
#   cargo install cargo-qemu-test --target x86_64-unknown-linux-gnu   # ⇒ cargo-qtest

# 喂日程的帮手（`--scene` 时落到 `<TRACE_OUT>/scene-feed.py` 再起）。
# 参数：端口文件 / 捕获文件 / 要重喂的那一句。**listener**，QEMU 以 client 身份连上来。
# 用 python 只为"同一条连接上既读又写"这件事；bash 做不了 listener，`nc` 那套要另装工具。
const FEED = '#!/usr/bin/env python3
"""喂日程的帮手：连上一次 ＝ 一例；断开 ＝ 那一例结束，回去等下一例。"""
import socket, sys, threading, time, importlib.util, pathlib
sys.dont_write_bytecode = True

port_file, cap, text = sys.argv[1], sys.argv[2], sys.argv[3]
after = float(sys.argv[4]) if len(sys.argv) > 4 else 5.0
script = sys.argv[5] if len(sys.argv) > 5 else ""
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

    if script:
        result = pathlib.Path(cap + ".result")
        try:
            # 驱动程序那一支可以带**同目录的兄弟模块**（按通道分文件，见 driver 那份头注）：
            # 把它当**包**加载（搜索路径 = 它所在目录）并先注册进 `sys.modules`，
            # 相对导入才找得到 —— 就这两处适配，没有第二套加载器。
            # 不带相对导入的脚本（`programs/tests/terminal/session.py`）逐字照旧。
            spec = importlib.util.spec_from_file_location(
                "scene_driver",
                script,
                submodule_search_locations=[str(pathlib.Path(script).resolve().parent)],
            )
            driver = importlib.util.module_from_spec(spec)
            sys.modules["scene_driver"] = driver
            spec.loader.exec_module(driver)
            driver.run(conn, cap)
            result.write_text("ok")
        except Exception as error:
            result.write_text(str(error))
            raise
        finally:
            conn.close()
        continue

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
# **（诊断：留全份控制台）** `cargo qtest` 在失败时只印 QEMU 输出**尾若干行**（超时那一档尤其
# 只剩尾巴），而"机器跑到哪一步停住"恰恰要看**前面**。`QEMU_TEE_LOG` 给了就顺路拷一份全份；
# 不给 = 与从前逐字相同（多一个 `cat` 的空管道而已）。
const WRAPPER = '#!/usr/bin/env bash
set -o pipefail
args=()
for a in "$@"; do
  [ "$a" = "-nographic" ] || args+=("$a")
done
if [ -n "$QEMU_TEE_LOG" ]; then
  qemu-system-riscv64 "${args[@]}" 2>&1 | tee "$QEMU_TEE_LOG"
else
  exec qemu-system-riscv64 "${args[@]}"
fi'

def main [--package: string, --scene: string, --profile: string, --feed: string, --feed-script: string, --feed-after: int = 5, ...rest: string] {
  if ($env.QEMU_ICOUNT? | is-empty) { $env.QEMU_ICOUNT = "" }

  # **runner 守卫**：crates.io 上**两个包**都提供 `cargo-qtest` 这一枚 bin——
  # `cargo-qemu-test`（对每一例起一个 QEMU，本脚本按它写）与 `cargo-qtest`（一个"挑用例的
  # UI"包装器，**一个 qemu 字样都没有**）。装错的那一枚不认识 `--qemu-arg`，而它的报错长成
  # `error: unexpected argument '--qemu-arg' found`——看起来像本脚本的旗标写错了（实测误判过）。
  # 故先问一句 `--help`：那一格在 ⇒ 是对的那一枚。
  let ok_runner = (try {
    (^cargo qtest --help | complete | get stdout) | str contains "--qemu-arg"
  } catch { false })
  if not $ok_runner {
    error make {msg: "cargo-qtest 不是 cargo-qemu-test——装成了同名的另一个包（挑用例的 UI）。装对的那一枚：`cargo install cargo-qemu-test --version 0.3.2 --target x86_64-unknown-linux-gnu --locked --force`"}
  }

  let script_dir = $env.FILE_PWD
  let repo = ($script_dir | path dirname)
  let trapdir = ($env.TRACE_OUT? | default ($repo | path join "trace"))
  let summary = match $scene {
    "memory-transfer" => "scene-memory-transfer-phases.json"
    "memory-transfer-tlb" => "scene-memory-transfer-tlb-summary.json"
    "memory-transfer-fresh" => "scene-memory-transfer-fresh-summary.json"
    _ => ""
  }
  let pending = if ($summary | is-empty) {
    ""
  } else {
    rm --force ($trapdir | path join $summary)
    $trapdir | path join $"($summary).pending-(random uuid)"
  }
  $env.SCENE_SUMMARY_PENDING = $pending

  mut cap = ""
  mut scene_args = []
  mut serial = ""
  mut wrapper = ""

  # 景：先造镜像，再把 `-initrd` 指过去（**绝对路径**——runner 的工作目录不是仓根），
  # 并把串口换到一条我们能喂输入的通道上（见头注）。
  # 档默认 release：`cargo image` 自己也是这个默认，而 `rig` 说 release 是
  # 它跑得动的前提（debug 下每轮都挂在 20 ms 那一缝上）。
  if not ($scene | is-empty) {
    let prof = if ($profile | is-empty) { "release" } else { $profile }
    let image = (^cargo image $scene $prof | complete)
    if $image.exit_code != 0 {
      print -n $image.stdout
      print -n $image.stderr
      exit $image.exit_code
    }
    let at = (
      $image.stdout
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
    if ($env.QEMU_TEE_LOG? | is-empty) {
      $env.QEMU_TEE_LOG = ($trapdir | path join $"scene-($scene)-console.log")
    }
    $cap = ($trapdir | path join $"scene-($scene)-feed.log")
    let helper = ($trapdir | path join "scene-feed.py")
    let pid_file = ($trapdir | path join "scene-feed.pid")
    let port_file = ($trapdir | path join "scene-feed.port")
    let err_file = ($trapdir | path join "scene-feed.err")
    let text = if ($feed | is-empty) { "\u{4}" } else { $feed }
    $FEED | save --force $helper
    rm -f $port_file
    if not ($feed_script | is-empty) { rm -f $"($cap).result" }
    # 三景使用本轮完整导出；正式成功标记仅由本进程发布。
    if not ($summary | is-empty) {
      rm --force ($repo | path join "sqware-diagnose.jsonl")
      rm --force $cap
      rm --force ($trapdir | path join $"scene-($scene)-diagnose.jsonl")
    }
    $env.FEED_HELPER = $helper
    $env.FEED_PID = $pid_file
    $env.FEED_PORTFILE = $port_file
    $env.FEED_CAP = $cap
    $env.FEED_TEXT = $text
    $env.FEED_AFTER = ($feed_after | into string)
    $env.FEED_SCRIPT = (if ($feed_script | is-empty) { "" } else { $feed_script | path expand })
    $env.FEED_ERR = $err_file
    # 上一轮万一留了帮手，先收掉（它自己会一直 listen）。
    if ($pid_file | path exists) {
      ^bash -c 'p=$(cat "$FEED_PID"); kill "$p" 2>/dev/null; true'
    }
    ^bash -c 'nohup python3 "$FEED_HELPER" "$FEED_PORTFILE" "$FEED_CAP" "$FEED_TEXT" "$FEED_AFTER" "$FEED_SCRIPT" >"$FEED_ERR" 2>&1 & echo $! > "$FEED_PID"'
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
    let label = if not ($feed_script | is-empty) { $"交互脚本 ($feed_script)" } else if ($feed | is-empty) { "EOF" } else { $feed }
    print $"喂入 ($label)（重喂到机器退）· 串口 ($serial) · 捕获 ($cap)"
  }

  # 参数表**唯一出处**：串口那一格也归它（`--serial`），本文件不写板子字面量。
  # `--iommu` 也只经它那一口传：只有要那件硬件的景（`iommu-*`）才开。
  let serial_argv = if ($serial | is-empty) { [] } else { ["--serial" $serial] }
  let iommu_argv = if ($scene | is-empty) {
    []
  } else if ($scene | str starts-with "iommu") {
    ["--iommu"]
  } else {
    []
  }
  # 端点那一景的那张盘：**可再生**（每次按同一花纹重建，落在 `<TRACE_OUT>/` 里，
  # 不进库），且由参数表那边只读挂上。景那一侧按同一花纹核读回的数据，故盘是承重的。
  let endpoint_argv = if $scene == "iommu-dma" {
    let disk = ($trapdir | path join "iommu-dma.img")
    ^python3 -c 'import sys, pathlib; pathlib.Path(sys.argv[1]).write_bytes(bytes(range(256)) * 4096)' $disk
    ["--endpoint" $disk]
  } else {
    []
  }
  let board = (^nu ($script_dir | path join "qemu-args.nu") --board-only ...$iommu_argv ...$serial_argv ...$endpoint_argv
      | lines | where { |l| ($l | str trim) != "" })
  let qemu_args = ($board | each { |a| $"--qemu-arg=($a)" })
  let pkg = if ($package | is-empty) { [] } else { ["--package" $package] }
  let qemu_bin = if ($wrapper | is-empty) { [] } else { ["--qemu" $wrapper] }
  # 整机用例跑 **release**：同一个 `accept` 景，release 产品路 6/6 稳（14 笔结局），debug 产品路
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
  # **工作区那三条 rustflags 要由这一侧递一遍**：`cargo-qtest` 把 `RUSTFLAGS` 的值
  # 当作**种子**，再拼上它自己那两枚（对我们这一景是 `-Tembedded-test.x`），最后经
  # `cargo --config <临时 TOML>` 投下去——而那一份 config 的 `[target.'cfg(target_arch="riscv64")']`
  # 表**优先于**根 `.cargo/config.toml` 的 `[build] rustflags` ⇒ 不递的话那三条会静默丢。
  # **真相仍只有一处**（`.cargo/config.toml`）；这里重复的是"调用方给 runner 递种子"这一手，
  # 不是第二处定义。已显式设过 `RUSTFLAGS` 就不动它。
  if ($env.RUSTFLAGS? | is-empty) {
    $env.RUSTFLAGS = "-Crelocation-model=static -Cforce-frame-pointers=yes -Ccode-model=medium"
  }
  # `try` 的块作用域会丢掉外部命令的 LAST_EXIT_CODE；直接保留完整执行结果。
  let result = (^cargo ...$qtest_argv o+e>| tee { save --force $qtlog } | complete)
  print -n $result.stdout
  mut code = $result.exit_code
  if not ($feed_script | is-empty) {
    let result = $"($cap).result"
    $env.FEED_RESULT = $result
    ^bash -c 'for i in $(seq 1 50); do if [ -s "$FEED_RESULT" ]; then exit 0; fi; sleep 0.02; done; true'
    if not ($result | path exists) {
      print "交互驱动未完成"
      $code = 1
    } else if (open --raw $result | str trim) != "ok" {
      print (open --raw $result)
      $code = 1
    }
  }

  let completion = match $scene {
    "accept" => "accept: done"
    "product" => "system: done"
    "system-fault" => "system-fault: acceptance passed"
    _ => ""
  }
  if $code == 0 and not ($completion | is-empty) {
    let console = ($env.QEMU_TEE_LOG? | default "")
    let proof = $"exit tid=1 reason=0x0 note: ($completion)"
    if not ($console | path exists) or not (open --raw $console | str contains $proof) {
      print $"场景未完成：缺少入口任务的成功退出证明 ($completion)"
      $code = 1
    }
  }

  if $code == 0 and $scene == "accept" {
    let console = (open --raw $env.QEMU_TEE_LOG | lines)
    for probe in ["probe-terminal" "probe-denied" "probe-owner" "probe-rule" "probe-rule-other" "probe-watch" "probe-watch-after" "probe-watch-gone" "probe-lease" "probe-bound" "probe-rack" "probe-rack-mount" "probe-rack-guest" "probe-coalition" "probe-control" "probe-operator-gate" "probe-operator-land" "subject" "member" "guest"] {
      let proof = $"^exit tid=[0-9]+ reason=0x0 note: ($probe):"
      if ($console | where {|line| $line =~ $proof } | is-empty) {
        print $"验收探针未成功完成：($probe)"
        $code = 1
      }
    }
  }

  if $code == 0 and $scene == "memory-transfer" {
    # 两台都报各自那一句话：引导那一台报收尾（本景的成功证明），
    # 控制单元报自己也正常收了工。两句话都是本景独有的串，不按 tid 认——
    # 打开 `CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS` 时，引导期那些自检会先铸几台任务，
    # 入口那一台的 tid 就不是 1 了（实测那一轮是 10）。
    let console = (open --raw $env.QEMU_TEE_LOG | lines)
    for proof in ["memory-transfer: acceptance passed" "memory-transfer: run passed"] {
      let at = $"^exit tid=[0-9]+ reason=0x0 note: ($proof)"
      if ($console | where {|line| $line =~ $at } | is-empty) {
        print $"本景未成功完成：缺少 ($proof)"
        $code = 1
      }
    }
  }

  if $code == 0 and $scene == "memory-transfer-tlb" {
    # 两句都是本景独有的串：引导那一台报收尾（本景的成功证明），
    # 读者那一台报自己也真的出过那段热窗口。后者走 semihosting，没有 `exit tid=` 前缀。
    let console = (open --raw $env.QEMU_TEE_LOG | lines)
    for proof in [
      "^exit tid=[0-9]+ reason=0x0 note: memory-transfer-tlb: acceptance passed"
      "^memory-transfer-tlb: reader left"
    ] {
      if ($console | where {|line| $line =~ $proof } | is-empty) {
        print $"本景未成功完成：缺少 ($proof)"
        $code = 1
      }
    }
  }

  if $code == 0 and $scene == "memory-transfer-fresh" {
    # 引导那一台报收尾：两阶段的读者都已被它 join 到手才走到这一行，故这一句就是
    # 本景的成功证明（逐格判据在宿主驱动那一侧）。
    let console = (open --raw $env.QEMU_TEE_LOG | lines)
    let proof = "^exit tid=[0-9]+ reason=0x0 note: memory-transfer-fresh: acceptance passed"
    if ($console | where {|line| $line =~ $proof } | is-empty) {
      print $"本景未成功完成：缺少 ($proof)"
      $code = 1
    }
  }

  if $code == 0 and $scene == "iommu-probe" {
    # 引导那一台报收尾：拆除与隔离判据都在它自己那一侧逐条核过（身子见 `probe/iommu.rs`）。
    let console = (open --raw $env.QEMU_TEE_LOG | lines)
    let proof = "^exit tid=[0-9]+ reason=0x0 note: iommu-probe: acceptance passed"
    if ($console | where {|line| $line =~ $proof } | is-empty) {
      print $"本景未成功完成：缺少 ($proof)"
      $code = 1
    }
  }

  if $code == 0 and $scene == "iommu-dma" {
    # 同理：端点那一景的正证与负证都在那一台自己那一侧逐条核过（身子见 `probe/iommu/dma.rs`）。
    let console = (open --raw $env.QEMU_TEE_LOG | lines)
    let proof = "^exit tid=[0-9]+ reason=0x0 note: iommu-dma: acceptance passed"
    if ($console | where {|line| $line =~ $proof } | is-empty) {
      print $"本景未成功完成：缺少 ($proof)"
      $code = 1
    }
  }

  if $code == 0 and not ($summary | is-empty) {
    if ($pending | path exists) {
      mv --force $pending ($trapdir | path join $summary)
    } else {
      print "交互驱动未提交本轮判定文件"
      $code = 1
    }
  }

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
