#! /usr/bin/env python3
"""热 TLB 那一景的宿主驱动：串行那一枚 ＋ 结构化导出那一枚。

程序侧自带内部次序（两枚共享 atomic 门 ＋ 直接 `rdtime` 期限），宿主**不开门、不喂字节**；
它只做两件事：
  · 按前缀读引导那一台报出来的那几行读数，逐格断言（判据在**它**这一个跑点，不在程序自己自报）；
  · 按严格规则读本轮结构化导出，核对那一次采样缺页**没有**落进内核（落进去就会有一条
    `memory.page_fault`），并核对读者那一台真的被起过、正常收工、恰回收一次。
末了把这两面的事实写成一份小 summary（证据留在忽略的 `trace/` 下）。
"""
import json
import os
import pathlib
import re
import time

from .console import Console
from .traces import Traces, room_payload

PREFIX = "memory-transfer-tlb: "
ARMED = 60.0
POLL = 0.02

# 程序侧那一段窗口的三个出口码（`window.rs` 的 `Window::*`）。
PENDING, TRAPPED, STALE, TIMEOUT = 0, 1, 2, 3
# 读者那一台的出口码（内核 `env::EXIT_OK`）。
EXIT_OK = 0x0

READER = re.compile(r"reader tid=(\d+) va=(0x[0-9a-f]+) view=(\d+) budget=(\d+)")
FACTS = re.compile(
    r"reader tid=(\d+) warm=(0x[0-9a-f]+) warm_at=(0x[0-9a-f]+) sample=(0x[0-9a-f]+) "
    r"sepc=(0x[0-9a-f]+) cause=(\d+) stval=(0x[0-9a-f]+) byte=(0x[0-9a-f]+) status=(\d+) "
    r"deadline=(0x[0-9a-f]+) sstatus=(0x[0-9a-f]+) stvec=(0x[0-9a-f]+)"
)
VERDICT = re.compile(r"(fault caught at the sampled load|FAIL .*)")
CAUGHT = "fault caught at the sampled load"
CLOSED = "close done"

# 花纹第一字节（`fault::at(0, 0)`，与程序侧同一处事实）。
WANT = 7
# `sstatus.SIE`。
SIE = 1 << 1
# `scause` 的 LoadPageFault。
LOAD_PAGE_FAULT = 13


def run(conn, capture):
    """喂入帮手指的那个入口：一例 = 一次连接。"""
    pending = pathlib.Path(os.environ["SCENE_SUMMARY_PENDING"])
    trace_dir = pathlib.Path(capture).parent
    console = Console(conn, capture)
    trace = Traces(pathlib.Path(capture).resolve().parent.parent / "sqware-diagnose.jsonl")
    try:
        summary = judge(console, trace)
        pending.write_text(
            json.dumps(summary, indent=2, sort_keys=True) + "\n"
        )
    finally:
        try:
            os.replace(trace.path, trace_dir / "scene-memory-transfer-tlb-diagnose.jsonl")
        except FileNotFoundError:
            pass


def judge(console, trace):
    """等本景自己那四行读数，逐格判定，再用严格导出核一遍。"""
    until = time.monotonic() + ARMED
    reader = line(console, READER, until)
    tid = int(reader.group(1))
    va = int(reader.group(2), 16)
    view = int(reader.group(3))
    budget = int(reader.group(4))
    assert budget > 0, "the reader was given no deadline budget"

    facts = line(console, FACTS, until)
    assert int(facts.group(1)) == tid, "two different reader tids in one run"
    warm = int(facts.group(2), 16)
    warm_at = int(facts.group(3), 16)
    sample = int(facts.group(4), 16)
    sepc = int(facts.group(5), 16)
    cause = int(facts.group(6))
    stval = int(facts.group(7), 16)
    byte = int(facts.group(8), 16)
    status = int(facts.group(9))
    deadline = int(facts.group(10), 16)
    sstatus = int(facts.group(11), 16)
    stvec = int(facts.group(12), 16)

    assert status != STALE, "the sampled load read a byte: the hot TLB was not revoked"
    assert status != TIMEOUT, "the sample gate never opened: the reader left on its own deadline"
    assert status == TRAPPED, f"the reader left status={status} (0=no verdict)"
    # 四格地址必须都是真地址：窗口一处也没发布时，`sepc == sample` 会以 0 == 0 成立，
    # 那样“cause/stval 都对”就能把一次**没发生过的采样**判成通过（与程序侧 `judge` 同一条规则）。
    assert va and warm_at and sample and sepc, (
        f"the window never published its load addresses: va={va:#x} warm_at={warm_at:#x} "
        f"sample={sample:#x} sepc={sepc:#x}"
    )
    assert sample != warm_at, "the sampled load and the warm access are the same instruction"
    assert cause == LOAD_PAGE_FAULT, f"the trap was cause={cause}, not a load page fault"
    assert stval == va, f"the trap's stval={stval:#x} is not the announced page {va:#x}"
    assert sepc == sample, f"the trap's sepc={sepc:#x} is not the sampled load {sample:#x}"
    assert warm == WANT, f"the warm access read {warm:#x}, not the announced pattern"
    assert sstatus & SIE, "the window did not start with interrupts on"
    assert deadline > 0, "the reader never folded its deadline"
    assert stvec != 0, "the reader saved no production vector"

    verdict = line(console, VERDICT, until)
    assert verdict.group(1) == CAUGHT, f"the scene reported {verdict.group(1)!r}"
    line(console, re.compile(CLOSED), until)

    settle(console, trace, until)
    # 那一段 VA 上**只应有控制者预先物化那一次写入缺页**（`fault::fill` 的头一字节）；
    # 采样那一次是**取**：它若真落进内核，这里就会多一条 `Load`。
    faults = trace.faults_at(va, 1)
    loads = [f for f in faults if f["fault"] == "Load"]
    assert not loads, f"the sampled load fault reached the kernel: {loads[:1]}"
    assert [f for f in faults if f["fault"] == "Store"], (
        f"the source page was never materialized through a fault: {faults}"
    )
    assert trace.room("spawn", tid), f"tid {tid} was never spawned"
    exits = trace.room("exit", tid)
    assert [e["reason"] for e in exits] == [EXIT_OK], f"tid {tid} ended as {exits}"
    assert len(trace.room("reap", tid)) == 1, f"tid {tid} was not reclaimed exactly once"

    # 导出里那两笔的 hart 归属只是**边界读数**：`spawn` 由控制者（放行那一手）记，
    # `exit` 由读者收工时那颗 hart 记。`SIE` 一恢复读者就可能被迁到别的 hart，故
    # **不能**把“两者不同”当成“采样时在远端 hart”的证据；真正的证据是那一段 SIE=0
    # 连续窗口加控制者照旧进展，以及“去掉按 ASID 定向的 RFENCE 就变 Stale”那一个人工变体。
    # 故这里只把两个边界记进 summary，不拿它判。
    launched = harts(trace, "spawn", tid)
    finished = harts(trace, "exit", tid)

    return {
        "scene": "memory-transfer-tlb",
        "marker": PREFIX.strip(),
        "release_debug_assertions": os.environ.get("CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS", ""),
        "reader": {
            "tid": tid,
            "va": hex(va),
            "view": view,
            "budget": budget,
            "warm": hex(warm),
            "warm_at": hex(warm_at),
            "sample": hex(sample),
            "sepc": hex(sepc),
            "cause": cause,
            "stval": hex(stval),
            "byte": hex(byte),
            "status": status,
            "deadline": hex(deadline),
            "sstatus": hex(sstatus),
            "stvec": hex(stvec),
        },
        "kernel_load_faults_at_va": 0,
        "kernel_faults_at_va": faults,
        "boundary": {"launch_harts": launched, "exit_harts": finished},
        "events": len(trace.events),
    }


def harts(trace, kind, tid):
    """那一类 room 事件里 tid 等于这一位的，都把 `h`（哪颗 hart 留的）交出来。"""
    return [
        event["h"]
        for event in trace.events
        for payload in [room_payload(event, kind)]
        if payload is not None and payload.get("tid") == tid
    ]


def settle(console, trace, until):
    """等本轮跑完（对端关掉那一根线），再把末行的完整性与导出都收下来。"""
    while not console.closed:
        console.drain()
        trace.poll()
        if console.closed:
            break
        assert time.monotonic() < until, "the guest never closed the scene connection"
        time.sleep(POLL)
    trace.finish()


def line(console, pattern, until):
    """下一条本景口径里匹配 `pattern` 的行；到点或对端先走就是失败。"""
    while True:
        got = console.line(max(0.0, min(1.0, until - time.monotonic())))
        if got is None:
            raise AssertionError(
                f"{pattern.pattern!r}: no such console line; lines={console.tail()!r}"
            )
        text = got.decode("utf-8", "replace")
        if text.startswith(PREFIX) and (found := pattern.search(text[len(PREFIX) :])):
            return found
        if time.monotonic() >= until:
            raise AssertionError(
                f"{pattern.pattern!r}: no such console line; lines={console.tail()!r}"
            )
