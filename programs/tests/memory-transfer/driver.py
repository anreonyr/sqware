"""本景宿主驱动的入口：把三条通道拼成一场判定。

  `console.py`  串行那一枚：本景自己打的口径 ＋ 宿主那枚门
  `traces.py`   结构化导出那一枚：内核真写出的事件
  `harness.py`  一场判定的骸架：等事件、发门、收官重核、写 summary

本文件只说本景的**阶段次序**与每阶段该看到的读数：
  A 一条 View 上同一 Missing 页、两名同 Team 任务都真的等过，恰一次填回放行两名；
  B 只叫走一名（不是 Doom 整 Team），另一名仍在等，事后才被填回放行；
  C 两页都 Missing，只撤第一页：那一页的等待者被真终止，另一页的重新入队，随后填回。

判定纪律（三条通道各用各的不扯）：事件按**已知 tid** ＋ 已知 VA ＋ 阶段配对，
不拿折叠后的 key 当唯一身份；缺事件就是真超时失败。
"""
import os
import pathlib

from .harness import Harness

# 故意被页错误终止的那一位的出口码（内核 `env::EXIT_FAULT`）。
FAULT_KILLED = 0xFFFFFFFF


def run(conn, capture):
    """喂入帮手指的那个入口：一例 = 一次连接。"""
    harness = Harness(conn, capture)
    trace = pathlib.Path(capture).parent
    try:
        harness.finish(stage(harness))
        harness.write_summary(trace / "scene-memory-transfer-phases.json")
    finally:
        try:
            os.replace(harness.trace.path, trace / "scene-memory-transfer-diagnose.jsonl")
        except FileNotFoundError:
            pass


def stage(harness):
    """逐阶段走完，并把每阶段对**最终读数**的预期交回给 `finish`。"""
    want = {}
    # A：一名 signal 放行两名。
    _, _, [one, two] = harness.arm("A", 1)
    harness.gate(b"A")
    harness.filled("A")
    harness.wakes("A", {one: 1, two: 1})
    harness.done("A")
    want["A"] = {"waits": {one: 1, two: 1}, "wakes": {one: 1, two: 1}, "exits": {one: 0, two: 0}}

    # B：只叫走一名；另一名不靠空间死亡收尾，而是靠后面那一次填回。
    _, _, [gone, keep] = harness.arm("B", 1)
    harness.gate(b"B")
    assert harness.slain("B") == gone, "phase B: the slain one is not the announced task"
    harness.never_exits("B", gone)
    harness.gate(b"b")
    harness.filled("B")
    harness.wakes("B", {gone: 0, keep: 1})
    harness.done("B")
    want["B"] = {
        "waits": {gone: 1, keep: 1},
        "wakes": {gone: 0, keep: 1},
        "exits": {gone: None, keep: 0},
    }

    # C：只撤第一页；另一页得真的**重新入队**才放行第二次填回。
    _, _, [head, tail] = harness.arm("C", 2)
    harness.gate(b"C")
    assert harness.killed("C") == head, "phase C: the killed one is not the announced task"
    harness.fault_killed("C", head)
    harness.fetch(
        "phase C: the other page's waiter queued again",
        lambda traces: traces.room("wait", tail) if len(traces.room("wait", tail)) >= 2 else None,
    )
    harness.gate(b"c")
    harness.filled("C")
    harness.wakes("C", {head: 1, tail: 2})
    harness.done("C")
    want["C"] = {
        "waits": {head: 1, tail: 2},
        "wakes": {head: 1, tail: 2},
        "exits": {head: FAULT_KILLED, tail: 0},
    }
    harness.closed()
    return want
