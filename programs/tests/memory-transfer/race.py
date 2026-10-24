#! /usr/bin/env python3
"""真实多 Hart 交错那一景的宿主驱动：串行那一枚 ＋ 结构化导出那一枚。

程序侧自带内部次序（内核夹具在真在途窗口里放开对端、对端自己去碰那一次冷缺页；
第二段里对端在另一颗 hart 上真拆图、夹具在同一个窗口里放行并核它落地），
宿主**不开门、不喂字节**；它只做两件事：
  · 按前缀读引导那一台报出来的那几行读数，逐格断言（判据在**它**这一个跑点）；
  · 按严格规则读本轮结构化导出，按**已知 tid ＋ 已知 VA** 核对那一笔真登记、那一
    次真放行与目标页的物化形状——“夹具在窗口里真看到那条跨空间关系的登记”只认导出里
    的这一串真事件，不认程序自己举的牌子。
末了把这两面的事实写成一份小 summary（证据留在忽略的 `trace/` 下）。
"""
import json
import os
import pathlib
import re
import time

from .console import Console
from .traces import Traces, page_fault_payload, room_payload

PREFIX = "memory-transfer-race: "
ARMED = 60.0
POLL = 0.02

PAGE_SIZE = 0x1000
# 移交那一手的回执（`Result<(), MemoryFail>` 的 `Ok` 那一档）。
COMMITTED = "Ok(())"
# 夹具在真在途窗口里真看到了对端的登记：本景要证的那一件事。
ARMED_STATE = "armed"
# 对端自己走出口那一种结局与它的出口码（内核 `env::ExitCause::Reap` / `env::EXIT_OK`）。
REAP = "Reap"
EXIT_OK = 0x0

READY = re.compile(
    r"phase=fault peer=(\d+) va0=(0x[0-9a-f]+) va1=(0x[0-9a-f]+) read0=(0x[0-9a-f]+)"
)
MOVED = re.compile(r"phase=fault transfer=(\S+) kernel=(\w+) foreign=(\d+)")
READ = re.compile(r"phase=fault read1=(0x[0-9a-f]+) read0=(0x[0-9a-f]+)")
TORN = re.compile(
    r"phase=teardown transfer=(\S+) kernel=(\w+) press=(\d+)->(\d+) "
    r"peer1=(\d+)->(\d+) released=(0x[0-9a-f]+) resumed=(\d+)"
)
TORN_READ = re.compile(r"phase=teardown read0=(0x[0-9a-f]+)")
JOINED = re.compile(r"phase=fault joined cause=(\w+) reason=(0x[0-9a-f]+)")
CLOSED = "close done"


def pattern_sum(page):
    """内核 `fault::at(page, offset)` 那一份花纹的整页和（宿主独立算一份）。"""
    return sum((offset * 31 + page + 7) & 0xFF for offset in range(PAGE_SIZE))


def run(conn, capture):
    """喂入帮手指的那个入口：一例 = 一次连接。"""
    pending = pathlib.Path(os.environ["SCENE_SUMMARY_PENDING"])
    trace_dir = pathlib.Path(capture).parent
    console = Console(conn, capture)
    trace = Traces(pathlib.Path(__file__).resolve().parents[3] / "sqware-diagnose.jsonl")
    try:
        summary = judge(console, trace)
        pending.write_text(
            json.dumps(summary, indent=2, sort_keys=True) + "\n"
        )
    finally:
        try:
            os.replace(trace.path, trace_dir / "scene-memory-transfer-race-diagnose.jsonl")
        except FileNotFoundError:
            pass


def judge(console, trace):
    """等本景自己那五行读数，逐格判定，再用严格导出核一遍。"""
    until = time.monotonic() + ARMED
    ready = line(console, READY, until)
    peer = int(ready.group(1))
    va0 = int(ready.group(2), 16)
    va1 = int(ready.group(3), 16)
    read0 = int(ready.group(4), 16)
    assert peer > 0, f"the peer must have a real task id, got {peer}"
    # 那两格 VA 是对端**自己空间里**的坐标：0 不会是真坐标，目标页恰在源页后一页。
    assert va0 and va1, f"the peer published no real coordinates: va0={va0:#x} va1={va1:#x}"
    assert va1 == va0 + PAGE_SIZE, f"the target page {va1:#x} is not the next page after {va0:#x}"
    assert read0 != 0, "the peer read page 0 as zeros: the source page carries no pattern"

    moved = line(console, MOVED, until)
    assert moved.group(1) == COMMITTED, (
        f"the transfer answered {moved.group(1)!r}, not {COMMITTED!r}"
    )
    kernel = moved.group(2)
    # 夹具那一步不是 `armed`（多 Hart 只有一颗时必然到点）：本景当场红，不把超时当证明。
    assert kernel == ARMED_STATE, (
        f"the fixture answered kernel={kernel}: the in-flight window never saw the peer register"
    )
    foreign = int(moved.group(3))
    # 压着源页的两条关系里真有一条在**别人空间**里握着翻译：否则这一次移交没从别人那个
    # 空间里撕下什么，“跨 Team”就只是范围上的话（这一格是夹具在现场读出来的）。
    assert foreign == 1, (
        f"the fixture saw foreign={foreign}: no relation in another space held the source page"
    )

    read = line(console, READ, until)
    read1 = int(read.group(1), 16)
    assert int(read.group(2), 16) == read0, "the read-back changed the announced page-0 sum"
    # 新清零页只会读到 0：非零且与第 0 页同值 ⇒ 对端读到的就是**搬过去的那一张承载**。
    assert read1 != 0, "the peer read a zero page after the transfer: the carrier did not move"
    assert read1 == read0, (
        f"the peer read {read1:#x}, not the {read0:#x} it announced before the transfer"
    )

    # 第二段：对端交出它那枚**来源**子分支，那一次 `Cleanup::finish` 与在途窗口重叠。
    # 两面都在内核侧读真对象：窗口里先看到两条关系、外空间那枚翻译在；放行之后
    # 在同一个窗口里看到只剩一条、外空间那枚翻译已经没了。
    torn = line(console, TORN, until)
    assert torn.group(1) == COMMITTED, (
        f"the teardown transfer answered {torn.group(1)!r}, not {COMMITTED!r}"
    )
    teardown_kernel = torn.group(2)
    assert teardown_kernel == ARMED_STATE, (
        f"the fixture answered kernel={teardown_kernel}: the window never saw the teardown land"
    )
    before, after = int(torn.group(3)), int(torn.group(4))
    assert before == 2, (
        f"the window saw {before} relations press the source page before the teardown, wanted two"
    )
    assert after == 1, (
        f"the teardown left {after} relations pressing the source page, wanted one"
    )
    peer_before, peer_after = int(torn.group(5)), int(torn.group(6))
    assert peer_before == 1, (
        "the window never saw the other space's PTE before the teardown"
    )
    assert peer_after == 0, "the other space's PTE survived the teardown"
    released = int(torn.group(7), 16)
    assert released == 0, f"the peer's release answered {released:#x}"
    resumed = int(torn.group(8))
    assert resumed == 1, (
        "the paused teardown never resumed: the fixture released it but the finish did not return"
    )
    torn_read = line(console, TORN_READ, until)
    torn_sum = int(torn_read.group(1), 16)
    assert torn_sum != 0, "the moved carrier reads zeros after the teardown"
    assert torn_sum == pattern_sum(1), (
        f"page 0 reads {torn_sum:#x} after the teardown, not the {pattern_sum(1):#x} it should"
    )

    joined = line(console, JOINED, until)
    assert joined.group(1) == REAP, f"the peer ended as cause={joined.group(1)}, not {REAP}"
    reason = int(joined.group(2), 16)
    assert reason == EXIT_OK, f"the peer failed with reason={reason:#x}"
    line(console, re.compile(CLOSED), until)

    settle(console, trace, until)
    waits = trace.room("wait", peer)
    wakes = trace.room("wake", peer)
    assert len(waits) == 1, f"tid {peer} queued {len(waits)} times, wanted one registration"
    assert len(wakes) == 1, f"tid {peer} woke {len(wakes)} times, wanted one"
    assert not trace.room("park", peer), f"tid {peer} parked instead of waiting on its page"
    assert len(trace.room("spawn", peer)) == 1, f"tid {peer} was not spawned exactly once"
    exits = [entry["reason"] for entry in trace.room("exit", peer)]
    assert exits == [EXIT_OK], f"tid {peer} ended as {exits}"
    reaps = trace.room("reap", peer)
    assert len(reaps) == 1, f"tid {peer} was reclaimed {len(reaps)} times, wanted one"

    faults = trace.faults_at(va1, 1)
    kinds = [(entry["fault"], entry["resolved"]) for entry in faults]
    assert kinds == [("Load", False), ("Load", True)], (
        f"the target page {va1:#x} faulted as {kinds}, wanted one unsettled load that waited "
        f"and then one settled load that materialised the moved carrier"
    )
    # 次序也是本景的一条事实：对端先在那条关系上真登记，才被这一手放行（放行早于登记
    # 就是别的什么把它叫醒的），而物化出承载的那一次重试在放行之后。
    assert (
        _room_at(trace, "wait", peer)
        < _room_at(trace, "wake", peer)
        < _fault_at(trace, va1, True)
    ), f"tid {peer}: the registration, the wake and the materialised retry are out of order"
    # 第 0 页上不许有**未结算**的缺页：对端第一遍就读到了内容，没在这页上等过。
    # 已结算的那种（这条关系被提前发布过）出不出不是本景要证的事，故不替它钉形状。
    waited = [entry for entry in trace.faults_at(va0, 1) if not entry["resolved"]]
    assert not waited, f"the peer waited on page 0: {waited[:1]}"

    return {
        "scene": "memory-transfer-race",
        "marker": PREFIX.strip(),
        "release_debug_assertions": os.environ.get("CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS", ""),
        "peer": {
            "tid": peer,
            "va0": hex(va0),
            "va1": hex(va1),
            "read0": hex(read0),
            "read1": hex(read1),
            "kernel": kernel,
            "foreign": foreign,
            "cause": joined.group(1),
            "reason": hex(reason),
            "waits": len(waits),
            "wakes": len(wakes),
            "parks": len(trace.room("park", peer)),
            "exits": [hex(entry) for entry in exits],
            "reaps": len(reaps),
            "faults_at_va0": trace.faults_at(va0, 1),
            "faults_at_va1": faults,
        },
        "teardown": {
            "kernel": teardown_kernel,
            "press_before": before,
            "press_after": after,
            "peer_before": peer_before,
            "peer_after": peer_after,
            "released": hex(released),
            "resumed": resumed,
            "read0": hex(torn_sum),
        },
        "events": len(trace.events),
    }


def _room_at(trace, kind, tid):
    """那位 tid 的那一类 room 事件在导出里的下标（恰一笔时才有意义）。"""
    return next(
        index
        for index, event in enumerate(trace.events)
        if (payload := room_payload(event, kind)) is not None and payload.get("tid") == tid
    )


def _fault_at(trace, va, resolved):
    """那一页上那种结算状态的那一笔缺页在导出里的下标。"""
    return next(
        index
        for index, event in enumerate(trace.events)
        for payload in [page_fault_payload(event)]
        if payload is not None and payload["va"] == va and payload["resolved"] is resolved
    )


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
    """下一条本景口径里匹配 `pattern` 的行；到点或对端先走就是失败。

    “这一轮没等到”不等于“对端走了”：每轮最多等一秒，没等到就接着等，直到总时限
    `until`。慢一点的模拟（比如把参数表那颗 `-smp 4` 改小）里第一行可能晚到几秒，
    而那不等于本景失败；真的关掉了那一端仍然当场失败。
    """
    while True:
        if console.closed:
            raise AssertionError(
                f"{pattern.pattern!r}: the guest left without that line; lines={console.tail()!r}"
            )
        got = console.line(max(0.0, min(1.0, until - time.monotonic())))
        if got is not None:
            text = got.decode("utf-8", "replace")
            if text.startswith(PREFIX) and (found := pattern.search(text[len(PREFIX) :])):
                return found
        if time.monotonic() >= until:
            raise AssertionError(
                f"{pattern.pattern!r}: no such console line; lines={console.tail()!r}"
            )
