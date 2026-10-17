#! /usr/bin/env python3
"""Fresh 失败回退那一景的宿主驱动：串行那一枚 ＋ 结构化导出那一枚。

程序侧自带内部次序（内核夹具在占槽之后放开读者），宿主**不开门、不喂字节**；
它只做两件事：
  · 按前缀读引导那一台报出来的那几行读数，逐格断言；
  · 按严格规则读本轮结构化导出，按 **已知 tid ＋ 已知 VA ＋ 阶段** 核对真实
    `room.wait`/`room.wake`/`room.park`/`memory.page_fault`：
      A：等待者恰好入队一次、放行一次、正常收工；目标 VA 上有一次未结算的取数
         缺页（那一次等待）与一次已结算的物化；源 VA 上有一次已结算的缺页
         （那一次失败的撤页真把它的 PTE 撤了）。
      B：晚到者**一次也没等**、没 Park，目标 VA 上有一次已结算的物化。
末了把这两面的事实写成一份小 summary（证据留在忽略的 `trace/` 下）。
"""
import json
import os
import pathlib
import re
import time

from .console import Console
from .traces import Traces, room_payload

PREFIX = "memory-transfer-fresh: "
ARMED = 60.0
POLL = 0.02

# 内核 `MemoryFail::Busy` 在 ABI 的 `Debug` 面里写出的那一格（一次未确认的撤页同步，可重试）。
BUSY = "MemoryFail::Busy"
# 内核 `env::EXIT_OK`。
EXIT_OK = 0x0

A_READY = re.compile(
    r"phase=A source=(0x[0-9a-f]+) target=(0x[0-9a-f]+) view=(\d+) reader=(\d+)"
)
A_TRANSFER = re.compile(r"phase=A transfer=(\S+) kernel=(\w+)")
A_READER = re.compile(r"phase=A reader=(\d+) value=(0x[0-9a-f]+)")
A_SOURCE = re.compile(r"phase=A source=(0x[0-9a-f]+) sum=(0x[0-9a-f]+)")
B_READY = re.compile(r"phase=B source=(0x[0-9a-f]+) target=(0x[0-9a-f]+) view=(\d+)")
B_TRANSFER = re.compile(r"phase=B transfer=(\S+) kernel=(\w+)")
B_LATE = re.compile(r"phase=B late=(\d+) value=(0x[0-9a-f]+)")
CLOSED = "close done"


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
            os.replace(trace.path, trace_dir / "scene-memory-transfer-fresh-diagnose.jsonl")
        except FileNotFoundError:
            pass


def judge(console, trace):
    """等本景自己那八行读数，逐格判定，再用严格导出核一遍。"""
    until = time.monotonic() + ARMED
    a = line(console, A_READY, until)
    a_source, a_target, a_view, reader = int(a[1], 16), int(a[2], 16), int(a[3]), int(a[4])
    moved = line(console, A_TRANSFER, until)
    assert moved[1] == BUSY, f"phase A: the transfer answered {moved[1]!r}, not {BUSY!r}"
    assert moved[2] == "armed", f"phase A: the fixture answered {moved[2]!r}, not armed"
    told = line(console, A_READER, until)
    assert int(told[1]) == reader, "phase A: two different reader tids in one run"
    value_a = int(told[2], 16)
    assert value_a == 0, f"phase A: the woken reader read {value_a:#x}, not a fresh zero page"
    kept = line(console, A_SOURCE, until)
    assert int(kept[1], 16) == a_source, "phase A: the source sum is not about the announced page"
    a_sum = int(kept[2], 16)
    assert a_sum != value_a, "phase A: the source page reads like a fresh zero page"

    b = line(console, B_READY, until)
    b_source, b_target, b_view = int(b[1], 16), int(b[2], 16), int(b[3])
    # 四页必须两两不同：宿主就是按 VA 把“本阶段的页错误”对到本阶段的。
    pages = {
        "phase A source": a_source,
        "phase A target": a_target,
        "phase B source": b_source,
        "phase B target": b_target,
    }
    assert len(set(pages.values())) == len(pages), f"two of the four pages are the same: {pages}"
    moved_b = line(console, B_TRANSFER, until)
    assert moved_b[1] == BUSY, f"phase B: the transfer answered {moved_b[1]!r}, not {BUSY!r}"
    assert moved_b[2] == "armed", f"phase B: the fixture answered {moved_b[2]!r}, not armed"
    late_line = line(console, B_LATE, until)
    late = int(late_line[1])
    value_b = int(late_line[2], 16)
    assert value_b == 0, f"phase B: the late reader read {value_b:#x}, not a fresh zero page"
    line(console, re.compile(CLOSED), until)

    settle(console, trace, until)
    assert reader > 0 and late > 0, "the readers must have nonzero task ids"
    assert reader != late, "the two stages answer with the same tid"
    _woke_once(trace, reader, a_target, "phase A")
    source_kept = trace.faults_at(a_source, 1)
    assert _kinds(source_kept) == [("Store", True), ("Load", True)], (
        f"phase A: the source page's faults were {_kinds(source_kept)}, wanted the fill's "
        f"Store and then a settled Load after the failed move revoked its PTE"
    )
    _materialized_once(trace, late, b_target, "phase B")
    source_load = next(
        index
        for index, event in enumerate(trace.events)
        if (fault := event["kind"].get("memory", {}).get("page_fault"))
        and a_source <= fault["va"] < a_source + 4096
        and fault["fault"] == "Load"
    )
    late_spawn = next(
        index
        for index, event in enumerate(trace.events)
        if (spawn := room_payload(event, "spawn")) is not None and spawn["tid"] == late
    )
    # 导出没有 Transfer 返回事件；只核可观察到的跨阶段顺序，完整晚到边界由程序序保证。
    assert late_spawn > source_load, "phase B: the late reader spawned before phase A finished reading"

    return {
        "scene": "memory-transfer-fresh",
        "marker": PREFIX.strip(),
        "release_debug_assertions": os.environ.get("CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS", ""),
        "waiter": {
            "tid": reader,
            "target": hex(a_target),
            "view": a_view,
            "value": hex(value_a),
            "waits": len(trace.room("wait", reader)),
            "wakes": len(trace.room("wake", reader)),
            "parks": len(trace.room("park", reader)),
            "exits": [hex(reason) for reason in _exits(trace, reader)],
            "reaps": len(trace.room("reap", reader)),
            "faults": trace.faults_at(a_target, 1),
        },
        "source": {"page": hex(a_source), "sum": hex(a_sum), "faults": source_kept},
        "late": {
            "tid": late,
            "target": hex(b_target),
            "view": b_view,
            "value": hex(value_b),
            "waits": len(trace.room("wait", late)),
            "wakes": len(trace.room("wake", late)),
            "parks": len(trace.room("park", late)),
            "exits": [hex(reason) for reason in _exits(trace, late)],
            "reaps": len(trace.room("reap", late)),
            "faults": trace.faults_at(b_target, 1),
        },
        "events": len(trace.events),
    }


def _kinds(faults):
    """页错误序列的形状：`(种类, 已结算?)`，顺序即真实发生次序。"""
    return [(entry["fault"], entry["resolved"]) for entry in faults]


def _woke_once(trace, tid, target, what):
    """等待者那一边：恰一次真入队、放行一次、正常收工、恰回收一次。

    入队与放行只按**已知 tid**认；本阶段它没 Park（那就不是另一个等待位），
    目标页上那两次缺页也必须是“先无承载地等、后物化”那一种形状。
    """
    waits = trace.room("wait", tid)
    assert len(waits) == 1, f"{what}: tid {tid} queued {len(waits)} times, wanted 1"
    assert len({entry["key"] for entry in waits}) == 1, f"{what}: tid {tid} sat on several keys"
    assert not trace.room("park", tid), f"{what}: tid {tid} parked instead of waiting on its page"
    assert len(trace.room("wake", tid)) == 1, (
        f"{what}: tid {tid} woke {len(trace.room('wake', tid))} times, wanted 1"
    )
    assert _exits(trace, tid) == [EXIT_OK], f"{what}: tid {tid} ended as {_exits(trace, tid)}"
    assert len(trace.room("reap", tid)) == 1, f"{what}: tid {tid} was not reclaimed once"
    assert len(trace.room("spawn", tid)) == 1, f"{what}: tid {tid} was never spawned"
    kinds = _kinds(trace.faults_at(target, 1))
    assert kinds == [("Load", False), ("Load", True)], (
        f"{what}: the target page {target:#x} faulted as {kinds}, wanted one unsettled load "
        f"that waited and then one settled load that materialized it"
    )


def _materialized_once(trace, tid, target, what):
    """晚到那一边：它一次也没等，直接物化出那页。

    未结算的那种缺页就是“等过一次”：本阶段它一次也不该有；目标页上也只能有那一次物化。
    """
    assert not trace.room("wait", tid), f"{what}: the late reader waited"
    assert not trace.room("wake", tid), f"{what}: the late reader woke without a wait"
    assert not trace.room("park", tid), f"{what}: the late reader parked"
    assert _exits(trace, tid) == [EXIT_OK], f"{what}: the late reader ended as {_exits(trace, tid)}"
    assert len(trace.room("reap", tid)) == 1, f"{what}: the late reader was not reclaimed once"
    assert len(trace.room("spawn", tid)) == 1, f"{what}: the late reader was never spawned"
    kinds = _kinds(trace.faults_at(target, 1))
    assert kinds == [("Load", True)], (
        f"{what}: the target page {target:#x} faulted as {kinds}, wanted exactly one settled load"
    )


def _exits(trace, tid):
    return [entry["reason"] for entry in trace.room("exit", tid)]


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
