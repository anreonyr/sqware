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
#   nu scripts/qtest.nu --package kernel --scene iommu-ruin -- --features semihosting
#   nu scripts/qtest.nu --package kernel --scene accept --feed-script programs/tests/terminal/session.py
#   nu scripts/qtest.nu --package kernel --scene memory-transfer --feed-script programs/tests/memory-transfer/driver.py -- --features semihosting
#   nu scripts/qtest.nu --package kernel --scene memory-transfer-tlb --feed-script programs/tests/memory-transfer/tlb.py -- --features semihosting
#   CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true nu scripts/qtest.nu --package kernel \
#     --scene memory-transfer-fresh --feed-script programs/tests/memory-transfer/fresh.py \
#     -- --features semihosting,fresh-scene -- scene
#   CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true nu scripts/qtest.nu --package kernel \
#     --scene memory-transfer-race --feed-script programs/tests/memory-transfer/race.py \
#     -- --features semihosting,race-scene -- scene
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
# **（`memory-transfer-race` 与 fresh 同一档 ＋ 它自己那一格 feature）**：那一景靠一个
# 在途夹具放开另一个 Team／另一个 Space 里的对端、再等它真登记在那条关系上；夹具同样只在
# `cfg(debug_assertions)` 里编得出来，`race-scene` 是它的**显式选景**。多 Hart 只有一颗时
# （`QEMU_SMP=1`）那一手不可能进展：夹具的有界自旋超时，程序侧当场红——那是负对照，
# 不是把它调成绿的。
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
# exec 保持 runner 所持 PID 就是 QEMU；捕获进程不隔开超时终止责任。
const WRAPPER = '#!/usr/bin/env bash
set -o pipefail
args=()
for a in "$@"; do
  [ "$a" = "-nographic" ] || args+=("$a")
done
if [ -n "$QEMU_TEE_LOG" ]; then
  exec qemu-system-riscv64 "${args[@]}" > >(
    if tee "$QEMU_TEE_LOG"; then
      printf 0 1<> "$QEMU_TEE_STATUS"
    else
      printf 1 1<> "$QEMU_TEE_STATUS"
    fi
  ) 2>&1
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
  let retirement = $scene in ["iommu-ruin" "iommu-forced"]
  let hardware = $scene == "iommu-fail-fence"
  let readback = $scene == "iommu-fail-readback"
  let soak = $scene in ["iommu-soak" "iommu-soak-long"]
  let dual = $scene in ["iommu-dual" "iommu-dual-long"]
  let dual_generations = if $scene == "iommu-dual-long" { 1024 } else { 1 }
  let soak_generations = if $scene == "iommu-soak-long" { 1024 } else { 128 }
  let confirmation = $scene in ["iommu-fail-invalidate" "iommu-fail-reset" "iommu-fail-release" "iommu-fail-reference"]
  let guard = ($scene | str starts-with "iommu-guard-")
  let guard_target = if $guard { $scene | str replace "iommu-guard-" "" } else { "" }
  let daemon_rtc = $scene == "daemon-rtc"
  let daemon_target = $scene == "daemon-target"
  let daemon_stop = $scene == "daemon-stop"
  let daemon = $daemon_rtc or $daemon_target or $daemon_stop
  let evidence = $retirement or $confirmation or $hardware or $readback or $soak or $dual or $guard or $daemon
  let retire_mode = if $scene == "iommu-forced" { "forced" } else { "ruin" }
  let feed_script = if ($retirement or $hardware or $readback or $guard) and ($feed_script | is-empty) {
    let driver = if $hardware { "hardware" } else if $readback { "readback" } else if $guard { "guard" } else { $retire_mode }
    $repo | path join $"programs/tests/device-recovery/($driver).py"
  } else {
    $feed_script
  }
  let hold = if $retirement or $hardware or $readback or $guard {
    $"target/ruin-(random uuid).nbd"
  } else {
    ""
  }
  $env.DMA_HOLD = $hold
  $env.DMA_CONTROL = if $hardware or $readback { $"target/control-(random uuid).qtest" } else { "" }
  $env.DMA_HOLD_SECONDS = if $guard { "60" } else if $scene == "iommu-forced" { "30" } else { "10" }
  let summary = match $scene {
    "memory-transfer" => "scene-memory-transfer-phases.json"
    "memory-transfer-tlb" => "scene-memory-transfer-tlb-summary.json"
    "memory-transfer-fresh" => "scene-memory-transfer-fresh-summary.json"
    "memory-transfer-race" => "scene-memory-transfer-race-summary.json"
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
    # 产品那四景各有自己的部署（入口镜像按景名认）：编排都在 Harness 那两台单元里，
    # 故场景名→镜像名只做一次映射，不给生产镜像塞景码。
    let image_scene = match $scene {
      "iommu-product-start-before" => "iommu-product-before"
      "iommu-product-start-after" => "iommu-product-after"
      _ => $scene
    }
    let image = (^cargo image $image_scene $prof | complete)
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
    if $hardware or $readback {
      if (which dtc | is-empty) {
        error make {msg: "硬件故障场景需要 dtc 读取 QEMU 生成的设备树"}
      }
      $env.DMA_DTB = ($trapdir | path join "board.dtb")
      let dump_args = (^nu ($script_dir | path join "qemu-args.nu") --board-only --iommu | lines)
      let dump = (^qemu-system-riscv64 ...$dump_args -machine $"dumpdtb=($env.DMA_DTB)" | complete)
      if $dump.exit_code != 0 {
        error make {msg: $"设备树生成失败：($dump.stderr)"}
      }
    }
    if ($env.QEMU_TEE_LOG? | is-empty) {
      $env.QEMU_TEE_LOG = ($trapdir | path join $"scene-($scene)-console.log")
    }
    $env.QEMU_TEE_STATUS = ($trapdir | path join $"scene-($scene)-capture.status")
    "P" | save --force $env.QEMU_TEE_STATUS
    $cap = ($trapdir | path join $"scene-($scene)-feed.log")
    let helper = ($trapdir | path join "scene-feed.py")
    let pid_file = ($trapdir | path join "scene-feed.pid")
    let port_file = ($trapdir | path join "scene-feed.port")
    let err_file = ($trapdir | path join "scene-feed.err")
    let text = if ($feed | is-empty) { "\u{4}" } else { $feed }
    $FEED | save --force $helper
    rm -f $port_file
    if not ($feed_script | is-empty) { rm -f $"($cap).result" }
    # 只使用本轮导出；正式成功标记仅由本进程发布。
    if not ($summary | is-empty) or $evidence {
      rm --force ($repo | path join "sqware-diagnose.jsonl")
      rm --force $cap
      rm --force $env.QEMU_TEE_LOG
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
  let endpoint_argv = if ($scene in ["iommu-dma" "iommu-product" "iommu-product-fault" "iommu-product-start-before" "iommu-product-start-after" "iommu-ruin" "iommu-forced" "iommu-soak" "iommu-soak-long" "iommu-fail-invalidate" "iommu-fail-reset" "iommu-fail-release" "iommu-fail-reference" "iommu-fail-fence"]) or $readback or $guard or $dual {
    let disk = ($trapdir | path join "iommu-dma.img")
    ^python3 -c 'import sys, pathlib; pathlib.Path(sys.argv[1]).write_bytes(bytes(range(256)) * 4096)' $disk
    if not ($hold | is-empty) {
      $env.DMA_HOLD_DISK = $disk
      $env.DMA_HOLD_SERVER = ($repo | path join "programs/tests/device-recovery/block.py")
      $env.DMA_HOLD_PID = ($trapdir | path join "block.pid")
      $env.DMA_HOLD_ERR = ($trapdir | path join "block.err")
      ^bash -c 'nohup python3 "$DMA_HOLD_SERVER" "$DMA_HOLD" "$DMA_HOLD_DISK" >"$DMA_HOLD_ERR" 2>&1 & echo $! > "$DMA_HOLD_PID"'
      let ready = (^python3 -c 'import pathlib, sys, time; p=pathlib.Path(sys.argv[1]+".json"); end=time.monotonic()+5
while not p.exists() and time.monotonic()<end: time.sleep(.01)
sys.exit(0 if p.exists() else 1)' $hold | complete)
      if $ready.exit_code != 0 {
        ^cat $env.DMA_HOLD_ERR
        ^bash -c 'kill "$(cat "$DMA_HOLD_PID")" "$(cat "$FEED_PID")" 2>/dev/null; true'
        error make {msg: "在途块设备夹具未启动"}
      }
    }
    if $dual {
      let second = ($trapdir | path join "iommu-dma-second.img")
      ^python3 -c 'import sys, pathlib; pathlib.Path(sys.argv[1]).write_bytes(bytes(n ^ 0xa5 for n in range(256)) * 4096)' $second
      ["--endpoint" $disk "--endpoints" ([$second] | to json --raw)]
    } else {
      ["--endpoint" $disk]
    }
  } else {
    []
  }
  let hold_argv = if ($hold | is-empty) { [] } else { ["--endpoint-hold" $hold] }
  let control_argv = if $hardware or $readback { ["--qtest-socket" $env.DMA_CONTROL] } else { [] }
  let board = (^nu ($script_dir | path join "qemu-args.nu") --board-only ...$iommu_argv ...$serial_argv ...$endpoint_argv ...$hold_argv ...$control_argv
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
  let raw_code = $result.exit_code
  mut code = $raw_code
  if not ($scene | is-empty) {
    let capture_status = (try { open --raw $env.QEMU_TEE_STATUS | str trim } catch { "missing" })
    if $capture_status != "0" {
      print $"控制台捕获未成功完成：($capture_status)"
      # A guard halt is killed by the runner timeout; keep the raw 101 so the
      # independent verifier can judge the captured non-returning evidence.
      if not ($guard and ($code == 101)) {
        $code = 1
      }
    }
  }
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
    "daemon-rtc" => "daemon-rtc: done"
    "daemon-target" => "daemon-target: done"
    "daemon-stop" => "daemon-stop: done"
    "system-fault" => "system-fault: acceptance passed"
    _ => ""
  }
  if $code == 0 and not ($completion | is-empty) {
    let console = ($env.QEMU_TEE_LOG? | default "")
    let lines = if ($console | path exists) { open --raw $console | lines } else { [] }
    let entries = ($lines | parse -r '^boot: entry task=(?<task>[1-9][0-9]*)$')
    if ($entries | length) != 1 {
      print $"场景未完成：缺少唯一的实际入口任务身份 ($completion)"
      $code = 1
    } else {
      let task = ($entries | first | get task)
      let proof = $"^exit tid=($task) reason=0x0 note: ($completion)$"
      if ($lines | where {|line| $line =~ $proof } | length) != 1 {
        print $"场景未完成：缺少实际入口任务 ($task) 的唯一成功退出证明 ($completion)"
        $code = 1
      }
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
    let retained = "^probe-lease: declared retention acquired, checked and released$"
    if ($console | where {|line| $line =~ $retained } | is-empty) {
      print "验收探针未验证声明供给的保留授权"
      $code = 1
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

  if $code == 0 and $scene == "memory-transfer-race" {
    # 引导那一台报收尾：对端已被它 join 到手才走到这一行；逐格判据（那一次真冷缺页、
    # 那一次真放行、两页读数一致、那条外空间关系真握着一条翻译）在宿主驱动那一侧。
    # 这里另外核**机器级净回收**：两台都正常退场、整机真的收场、没有留下的等待者——
    # 那几行是内核打到 semihosting 控制台上的，只有本脚本看得到。
    let console = (open --raw $env.QEMU_TEE_LOG | lines)
    let want = [
      "^exit tid=[0-9]+ reason=0x0 note: memory-transfer-race: peer passed$"
      "^exit tid=[0-9]+ reason=0x0 note: memory-transfer-race: acceptance passed$"
      "^task: all tasks exited, system halted$"
      "^doom: held=0 starved=0 blocked=0 nudged=0$"
    ]
    for $pattern in $want {
      if ($console | where {|line| $line =~ $pattern } | is-empty) {
        print $"本景未成功完成：缺少 ($pattern)"
        $code = 1
      }
    }
  }

  if $code == 0 and not ($scene | is-empty) {
    let frames = (open --raw $env.QEMU_TEE_LOG | lines
      | parse -r '^memory: frame_allocations before=(?<before>[0-9]+) after=(?<after>[0-9]+)$')
    if ($frames | length) != 1 or ($frames | any {|row| $row.before != $row.after }) {
      print "本景未成功完成：物理帧分配未回到起场基线"
      $code = 1
    }
  }

  if $evidence {
    let exported = ($repo | path join "sqware-diagnose.jsonl")
    let archived = ($trapdir | path join $"scene-($scene)-diagnose.jsonl")
    if ($exported | path exists) { cp --force $exported $archived }
    if $guard {
      let nbd_events = ($trapdir | path join $"scene-($scene)-block-events")
      let nbd_state = ($trapdir | path join $"scene-($scene)-block-state.json")
      if (not ($hold | is-empty)) and (($"($hold).events") | path exists) {
        cp --force $"($hold).events" $nbd_events
        cp --force $"($hold).json" $nbd_state
      }
      let harts = ($env.QEMU_SMP? | default "4")
      let checked = (^python3 ($repo | path join "programs/tests/device-recovery/guard_check.py")
        $env.QEMU_TEE_LOG $archived $cap $guard_target $harts ($code | into string) $qtlog --hold $hold | complete)
      print -n $checked.stdout
      print -n $checked.stderr
      if $checked.exit_code != 0 { $code = 1 }
    } else if $daemon {
      let checker = if $daemon_rtc {
        "daemon_rtc.py"
      } else if $daemon_target {
        "daemon_target.py"
      } else {
        "daemon_stop.py"
      }
      let checked = (^python3 ($repo | path join "programs/tests/daemon-recovery" $checker)
        $env.QEMU_TEE_LOG $archived $qtlog ($raw_code | into string) | complete)
      print -n $checked.stdout
      print -n $checked.stderr
      if $checked.exit_code != 0 { $code = 1 }
    } else {
      let checker = if $retirement {
        {script: "ruin_check.py", args: [$cap $retire_mode]}
      } else if $confirmation {
        {script: "confirmation.py", args: [($scene | str replace "iommu-fail-" "")]}
      } else if $hardware {
        {script: "hardware_check.py", args: [$cap]}
      } else if $readback {
        {script: "readback_check.py", args: [$cap]}
      } else if $dual {
        {script: "dual_check.py", args: [($dual_generations | into string)]}
      } else {
        {script: "soak.py", args: [($soak_generations | into string)]}
      }
      if $code == 0 {
        let checked = (^python3 ($repo | path join "programs/tests/device-recovery" $checker.script)
          $env.QEMU_TEE_LOG $archived ...$checker.args | complete)
        print -n $checked.stdout
        print -n $checked.stderr
        if $checked.exit_code != 0 { $code = 1 }
      }
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
    for coordinate in [
      "64 iova=0x208000000"
      "511 iova=0x23fe00000"
      "512 iova=0x240000000"
      "126975 iova=0x3fffe00000"
    ] {
      let window = $"^iommu-probe: window boundary=($coordinate) response=0x[0-9a-f]+ previous=0x[0-9a-f]+ stale_response=0x[0-9a-f]+ ddtp_off=0x0 cqcsr=0x0 fqcsr=0x0$"
      if ($console | where {|line| $line =~ $window } | length) != 1 {
        print $"本景未成功完成：缺少唯一的窗口翻译与旧窗拒绝读数 ($coordinate)"
        $code = 1
      }
    }
  }

  if $code == 0 and $scene in ["iommu-product" "iommu-product-fault"] {
    # 产品那一侧：System 只装配（UnitFile/Deployment/Assembly/Control/Supply），
    # PCI／IOMMU／设备恢复各守本硬件职责（各自独立 Task/Space），Hub 只做发现、绑定、
    # 停止新请求与恢复协调；场景编排（固定扇区／哨兵／杀服务）在 Harness 那两台单元里。
    let console = (open --raw $env.QEMU_TEE_LOG | lines)
    mut want = [
      # 三个硬件域各自独立 Task/Space：各报自己的号与它自己映射的那一段（本域 VA）。
      "^iommu: space task=[0-9]+ iommu=0x[0-9a-f]+ retain=[0-9]+ ddtp=0x0 \\(off\\)$"
      "^pci: space task=[0-9]+ ecam=0x[0-9a-f]+ functions=[0-9]+$"
      "^pci: ready functions=[0-9]+$"
      "^virtio-blk: ready hub=[0-9]+ pci=[0-9]+ requester=[0-9]+ key=[0-9]+$"
      # 执行实例由控制器用受限构造建出（owner = Hub），再由 Hub 放行；
      # 停止与核销也归 Hub（实例面按真实 owner 核）。
      "^ctl: constructed device-work tid=[0-9]+ owner=hub$"
      "^hub: execution instance launched tid=[0-9]+$"
      "^ctl: binding 1 auxiliary=[0-9]+ alive=true$"
      "^ctl: binding 1 auxiliary=[0-9]+ alive=false$"
      # 落位与开启分开：落位时 MASTER 是关的（真回读），IOMMU 发布、恢复映射与子授权都
      # 就绪之后才开。
      "bus-master off=0x0$"
      "^pci: endpoint bus-master on=0x4$"
      # Hub 放行业务：发布之后才 Active（执行侧从它那一条孔拿到坐标）。
      "^iommu: binding 1 published requester=[0-9]+ held=6"
      "^virtio-blk: binding 1 active iova=0x[0-9a-f]+ execution=[0-9]+$"
      # 固定工作负载：真读一个扇区、花纹对得上、状态是 OK（判据在控制器那一侧）。
      "^ctl: served used=1 len=513 written_ok=true sector_ok=true status=0$"
      # 收场：先整域停执行方 CPU 控制，再按真实停链协调。
      "^hub: binding 1 execution domain stopped \\(tid=[0-9]+\\)$"
      "^iommu: binding 1 invalidated frames held=6 \\(not released\\)$"
      "^pci: endpoint bus-master off=0x0$"
      "^virtio-blk: binding 1 stopped reset=1 status=0 queue_enable=0$"
      "^iommu: binding 1 queues stopped cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 faults=0 last_did=0$"
      # 每一个字段都钉死：把停链换成空实现，这几格就回不到 0；帧在停链期间一直持着。
      "^iommu: binding 1 released ddtp=0x0 cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 frames=0 stopped=true$"
      "^virtio-blk: binding 1 detached$"
      "^hub: binding 1 closed and released device [0-9]+$"
      # 整机也真的收场（与普通 product 同一格）：任务都退了、没有残余持有。
      "^exit tid=1 reason=0x0 note: system: done$"
      "^task: all tasks exited, system halted$"
      "^memory: frame_allocations before=[0-9]+ after=[0-9]+$"
    ]
    if $scene == "iommu-product" {
      $want = ($want | append [
        "^work: requesting Close::Normal$"
        "^ctl: binding 1 closed and reclaimed$"
      ])
    } else {
      $want = ($want | append [
        "^ctl: constructed device-work tid=[0-9]+ owner=hub$"
        "^hub: execution instance launched tid=[0-9]+$"
        "^ctl: alpha exit requested \\(hub recovers owner death\\)$"
        "^ctl: rebind probe claimed serial=2 \\(device free again\\)$"
        "^virtio-blk: binding 2 active iova=0x[0-9a-f]+ execution=[0-9]+$"
        "^work: rang the stale iova doorbell$"
        "^ctl: stale iova submitted sentinel_held=true status_held=true$"
        "^iommu: binding 2 queues stopped cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 faults=[1-9][0-9]* last_did=[0-9]+$"
        "^iommu: binding 2 released ddtp=0x0 cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 frames=0 stopped=true$"
        "^hub: binding 2 closed and released device [0-9]+$"
        "^ctl: binding 2 auxiliary=[0-9]+ alive=true$"
        "^ctl: binding 2 auxiliary=[0-9]+ alive=false$"
      ])
    }
    for $pattern in $want {
      if ($console | where {|line| $line =~ $pattern } | is-empty) {
        print $"本景未成功完成：缺少 ($pattern)"
        $code = 1
      }
    }
    let pci = ($console | parse -r '^pci: space task=(?<task>[0-9]+) ecam=0x[0-9a-f]+ functions=(?<functions>[0-9]+)$')
    let pci_ready = ($console | parse -r '^pci: ready functions=(?<functions>[0-9]+)$')
    let drivers = ($console | parse -r '^virtio-blk: ready hub=(?<hub>[0-9]+) pci=(?<pci>[0-9]+) requester=(?<requester>[0-9]+) key=(?<key>[0-9]+)$')
    let hubs = ($console | parse -r '^system: minted hub tid=(?<task>[0-9]+)$')
    mut requester = ""
    mut key = ""
    if ($pci | length) != 1 or ($pci_ready | length) != 1 or ($drivers | length) != 1 or ($hubs | length) != 1 {
      print "本景未成功完成：PCI/Driver 身份不唯一"
      $code = 1
    } else {
      let pci_task = ($pci | first | get task)
      $requester = ($drivers | first | get requester)
      $key = ($drivers | first | get key)
      if ($drivers | first | get pci) != $pci_task or ($drivers | first | get hub) != ($hubs | first | get task) or ($pci_ready | first | get functions) != ($pci | first | get functions) {
        print "本景未成功完成：PCI/Driver 身份未对上"
        $code = 1
      }
      mut linked = [
        $"^iommu: binding 1 published requester=($requester) held=6"
        $"^hub: binding 1 closed and released device ($key)$"
      ]
      if $scene == "iommu-product-fault" {
        $linked = ($linked | append [
          $"^hub: binding 2 closed and released device ($key)$"
          $"^iommu: binding 2 queues stopped .* faults=[1-9][0-9]* last_did=($requester)$"
        ])
      }
      for $pattern in $linked {
        if ($console | where {|line| $line =~ $pattern } | is-empty) {
          print $"本景未成功完成：设备身份未对上 ($pattern)"
          $code = 1
        }
      }
    }
    # 停链次序是合同：撤翻译 → 清 MASTER → 设备 reset/drain → 关队列 → 释放；
    # 帧在确认之前一直持着；释放之后才放下恢复映射，才允许重绑。
    let ordered = [
      ["^hub: binding 1 execution domain stopped" "^iommu: binding 1 invalidated"]
      ["^iommu: binding 1 invalidated" "^pci: endpoint bus-master off=0x0$"]
      ["^pci: endpoint bus-master off=0x0$" "^virtio-blk: binding 1 stopped"]
      ["^virtio-blk: binding 1 stopped" "^iommu: binding 1 queues stopped"]
      ["^iommu: binding 1 queues stopped" "^iommu: binding 1 released"]
      ["^iommu: binding 1 invalidated" "^iommu: binding 1 released"]
      ["^iommu: binding 1 released" "^virtio-blk: binding 1 detached"]
      ["^virtio-blk: binding 1 detached" $"^hub: binding 1 closed and released device ($key)$"]
    ]
    for $pair in $ordered {
      let before = ($console | enumerate | where {|row| $row.item =~ $pair.0 } | get index | first)
      let after = ($console | enumerate | where {|row| $row.item =~ $pair.1 } | get index | first)
      if ($before | is-empty) or ($after | is-empty) or ($before >= $after) {
        print $"本景未按合同次序收口：($pair.0) 必须早于 ($pair.1)"
        $code = 1
      }
    }
  }

  if $code == 0 and $scene in ["iommu-product-start-before" "iommu-product-start-after"] {
    # 起手受控失败那两景：执行方在一个**明确的点**上停（发布之前／发布之后），
    # 由 Hub 按同一道门与同一条停链结清。要看到的是：没留活僵尸、本次责任真被结清、
    # 且两档之间设备那一侧的现场截然不同——发布之前失败 = 设备从来没被接上；
    # 发布之后失败 = 平台真把设备停了才释放。
    let console = (open --raw $env.QEMU_TEE_LOG | lines)
    let pci = ($console | parse -r '^pci: space task=(?<task>[0-9]+) ecam=0x[0-9a-f]+ functions=(?<functions>[0-9]+)$')
    let pci_ready = ($console | parse -r '^pci: ready functions=(?<functions>[0-9]+)$')
    let drivers = ($console | parse -r '^virtio-blk: ready hub=(?<hub>[0-9]+) pci=(?<pci>[0-9]+) requester=(?<requester>[0-9]+) key=(?<key>[0-9]+)$')
    let hubs = ($console | parse -r '^system: minted hub tid=(?<task>[0-9]+)$')
    mut key = ""
    if ($pci | length) != 1 or ($pci_ready | length) != 1 or ($drivers | length) != 1 or ($hubs | length) != 1 {
      print "本景未成功完成：PCI/Driver 身份不唯一"
      $code = 1
    } else {
      let pci_task = ($pci | first | get task)
      $key = ($drivers | first | get key)
      if ($drivers | first | get pci) != $pci_task or ($drivers | first | get hub) != ($hubs | first | get task) or ($pci_ready | first | get functions) != ($pci | first | get functions) {
        print "本景未成功完成：PCI/Driver 身份未对上"
        $code = 1
      }
    }
    mut want = [
      "^pci: space task=[0-9]+ ecam=0x[0-9a-f]+ functions=[0-9]+$"
      "^pci: ready functions=[0-9]+$"
      "^virtio-blk: ready hub=[0-9]+ pci=[0-9]+ requester=[0-9]+ key=[0-9]+$"
      "^work: claimed serial=1$"
      "^ctl: constructed device-work-(before|after) tid=[0-9]+ owner=hub$"
      "^ctl: binding 1 auxiliary=[0-9]+ alive=true$"
      "^ctl: binding 1 auxiliary=[0-9]+ alive=false$"
      "^hub: execution instance launched tid=[0-9]+$"
      "^hub: binding 1 execution domain stopped \\(tid=[0-9]+\\)$"
      "^iommu: binding 1 released ddtp=0x0 cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 frames=0 stopped=true$"
      "^virtio-blk: binding 1 detached$"
      $"^hub: binding 1 closed and released device ($key)$"
      "^exit tid=1 reason=0x0 note: system: done$"
      "^task: all tasks exited, system halted$"
      "^memory: frame_allocations before=[0-9]+ after=[0-9]+$"
    ]
    if $scene == "iommu-product-start-before" {
      $want = ($want | append [
        # 发布之前中止：从来没有过“已发布”那一行，执行方也没交过窗。
        "^work: requesting Close::Fault before publish$"
        "^ctl: binding 1 settled after abort$"
      ])
      let forged = (
        $console
        | where {|line| $line =~ "^iommu: binding 1 published" }
        | length
      )
      if $forged != 0 {
        print "本景未按预期收口：发布之前中止不该出现已发布那一行"
        $code = 1
      }
    } else {
      $want = ($want | append [
        # 发布之后中止：发布真发生过（撤翻译与整条停链才跑得有意义）。
        "^work: requesting Close::Fault after publish$"
        "^iommu: binding 1 published"
        "^iommu: binding 1 invalidated frames held=6 \\(not released\\)$"
        "^ctl: binding 1 settled after abort$"
      ])
    }
    for $pattern in $want {
      if ($console | where {|line| $line =~ $pattern } | is-empty) {
        print $"本景未成功完成：缺少 ($pattern)"
        $code = 1
      }
    }
  }

  if $code == 0 and $scene == "iommu-dma" {
    # 同理：端点那一景的正证与负证都在那一台自己那一侧逐条核过（身子见 `probe/iommu/dma.rs`）。
    let console = (open --raw $env.QEMU_TEE_LOG | lines)
    for proof in [
      "^pci-probe: reset-zero requester=[0-9]+ bar=[0-5] size=0x[0-9a-f]+ restored=0 master=0$"
      "^pci-probe: upper-half requester=[0-9]+ bar=[1-5] rejected=true restored=true$"
      "^exit tid=[0-9]+ reason=0x0 note: iommu-dma: acceptance passed"
    ] {
      if ($console | where {|line| $line =~ $proof } | is-empty) {
        print $"本景未成功完成：缺少 ($proof)"
        $code = 1
      }
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
    if $code == 0 and not $daemon {
      rm --force $qtlog
    } else if $code != 0 {
      print $"现场（cargo qtest 全份）-> ($qtlog)"
    }
  }

  if not ($scene | is-empty) {
    ^bash -c 'p=$(cat "$FEED_PID"); kill "$p" 2>/dev/null; true'
    if not ($hold | is-empty) {
      ^bash -c 'p=$(cat "$DMA_HOLD_PID"); kill "$p" 2>/dev/null; true'
    }
    # 串口是一根"只喂不读"的输入线；机侧控制台仍走 semihosting（见头注）。这一手是为
    # 串口路由万一被改动时留的后手——读不到东西是正常的。
    if $code != 0 and ($cap | path exists) {
      print $"（喂入侧捕获：($cap)，下面是尾 40 行）"
      ^tail -40 $cap
    }
  }
  exit $code
}
