"""一场判定的骨架：等事件 → 发门 → 收官重核 → 写 summary。

三条通道在这里合流：控制台（`console`）报本景自己的口径，结构化导出（`traces`）报内核真写出
的事实，门只由本文件发。每个阶段的名字、VA、View 身份与两名 tid 都在这里配对；
折叠后的 key 只当辅助（真正的身份靠已知 tid ＋ VA ＋ 阶段）。

两条时序纪律：
  · **等事件再发门**：导出里看不到本阶段该看到的入队 / 页错误，就不往下走；
    到点没来就是失败（不降断言、不拿台钟固定延时冒充）。
  · **收官看最后一份完整读数**：阶段里看到一次只当“该往下走了”，
    每阶段的 `waits/wakes/exits` 与“哪些 tid 真被回收了”以 `finish` 那一步的重核为准。
"""
import json
import os
import pathlib
import re
import time

from .console import PREFIX, Console
from .traces import Traces, room_payload

# 整体时限（秒）：只用来把「没来」变成「失败」，不用来定顺序。
ARMED = 45.0
POLL = 0.02

READY = re.compile(r"phase=(\w+) ready va=(0x[0-9a-f]+) view=(\d+) tids=(\d+),(\d+)")
SLAIN = re.compile(r"phase=(\w+) slain tid=(\d+)")
KILLED = re.compile(r"phase=(\w+) killed tid=(\d+)")
FILLED = re.compile(r"phase=(\w+) filled")
DONE = re.compile(r"phase=(\w+) done")
CLOSED = "phase=close done"


class Harness:
    """本景那三个阶段的判定面：两边一起看，门只从这里发出去。"""

    def __init__(self, conn, capture):
        self.console = Console(conn, capture)
        self.capture = pathlib.Path(capture)
        self.trace = Traces(self._diagnose())
        self.phases = {}
        self.until = time.monotonic() + ARMED

    def _diagnose(self):
        # 导出名固定、写在 qemu 的工作目录（仓根）；捕获文件在 <仓根>/trace/ 下。
        return self.capture.resolve().parent.parent / "sqware-diagnose.jsonl"

    def _budget(self, seconds):
        return max(0.0, min(seconds, self.until - time.monotonic()))

    def fetch(self, what, ready, seconds=10.0):
        """等导出里真的出现该看到的事实；到点没来就是失败。

        同时把控制台那一条线上的字节收进来：那一台要是已经走了，当场说清楚，
        不要在这里空等到总时限。
        """
        end = time.monotonic() + self._budget(seconds)
        while True:
            self.console.drain()
            self.trace.poll()
            got = ready(self.trace)
            if got is not None:
                return got
            if time.monotonic() >= end:
                raise AssertionError(f"{what}: no such event; lines={self.console.tail()!r}")
            time.sleep(POLL)

    def line(self, pattern, seconds=30.0):
        """下一条本景口径里匹配 `pattern` 的行。"""
        while True:
            got = self.console.line(self._budget(seconds))
            if got is None:
                raise AssertionError(
                    f"{pattern.pattern!r}: no such console line; lines={self.console.tail()!r}"
                )
            text = got.decode("utf-8", "replace")
            if text.startswith(PREFIX) and (found := pattern.search(text[len(PREFIX) :])):
                return found

    # ---- 一个阶段的四手 ----

    def arm(self, phase, pages, seconds=30.0):
        """本阶段就绪：记下它的 VA / View / 两名 tid，并等两名都真的入队。"""
        got = self.line(READY, seconds)
        name = got.group(1)
        assert name == phase, f"phase {phase}: console announced {name}"
        va = int(got.group(2), 16)
        view = int(got.group(3))
        tids = [int(got.group(4)), int(got.group(5))]
        waits = self.fetch(
            f"phase {phase}: both waiters queued",
            lambda traces: [traces.room("wait", tid) for tid in tids]
            if all(traces.room("wait", tid) for tid in tids)
            else None,
        )
        keys = {entry["key"] for per in waits for entry in per}
        assert len(keys) == 1, f"phase {phase}: the two waiters sit on {keys}"
        self.fetch(
            f"phase {phase}: a fault on the announced VA",
            lambda traces: traces.faults_at(va, pages) or None,
        )
        self.phases[phase] = {
            "va": va,
            "view": view,
            "tids": tids,
            "key": keys.pop(),
            "faults": len(self.trace.faults_at(va, pages)),
        }
        return va, view, tids

    def gate(self, byte):
        """开本阶段的下一手，开关是宿主这才发出去的那一枚字节。"""
        self.console.send(byte)

    def filled(self, phase):
        got = self.line(FILLED)
        assert got.group(1) == phase, f"phase {phase}: filled said {got.group(1)}"

    def slain(self, phase):
        got = self.line(SLAIN)
        assert got.group(1) == phase, f"phase {phase}: slain said {got.group(1)}"
        return int(got.group(2))

    def killed(self, phase):
        got = self.line(KILLED)
        assert got.group(1) == phase, f"phase {phase}: killed said {got.group(1)}"
        return int(got.group(2))

    def done(self, phase):
        got = self.line(DONE)
        assert got.group(1) == phase, f"phase {phase}: done said {got.group(1)}"

    # ---- 阶段里那几笔早核（让失败停近现场；收官的 `settle` 再重核一次） ----

    def wakes(self, phase, want, seconds=15.0):
        """先等出“每一位至少放了这么多次”当早核；精确次数由收官的 `settle` 定。"""
        for tid, count in want.items():
            self.fetch(
                f"phase {phase}: tid {tid} woken {count} times",
                lambda traces, tid=tid, count=count: True
                if len(traces.room("wake", tid)) >= count
                else None,
                seconds,
            )

    def never_exits(self, phase, tid):
        """这一名没自己走过出口——正证是控制者报的那条 `Reaped{cause}` 回执。"""
        self.trace.poll()
        assert not self.trace.room("exit", tid), f"phase {phase}: tid {tid} exited on its own"

    def fault_killed(self, phase, tid):
        """真终止那一位：它的 `stval` 必须正是本阶段公告的那个页起点。"""
        va = self.phases[phase]["va"]
        got = self.fetch(
            f"phase {phase}: tid {tid} killed by a page fault at {va:#x}",
            lambda traces: next(
                (e for e in traces.room("fault_killed", tid) if e["stval"] == va), None
            ),
        )
        self.phases[phase]["fault_killed"] = {"tid": tid, "stval": hex(got["stval"])}

    def closed(self):
        """收尾那一手：三段来源都收完了（程序自己那一步失败会让整景红）。"""
        while True:
            got = self.console.line(self._budget(20.0))
            if got is None:
                raise AssertionError(f"no close marker; lines={self.console.tail()!r}")
            text = got.decode("utf-8", "replace")
            if text.startswith(PREFIX) and text[len(PREFIX) :] == CLOSED:
                return

    # ---- 收官 ----

    def finish(self, want):
        """整场跑完以后的最后一步：回收到位、再按最终完整读数重核每阶段。

        `want` 是“阶段名 → {waits/wakes/exits}”（`driver` 随阶段逐段交上来）：
        `exits` 里那一格是 `None` 就是“这一名不该自己走过出口”。
        """
        reaps = [tid for spec in want.values() for tid in spec["waits"]]
        while not self.console.closed:
            self.console.drain()
            self.trace.poll()
            if self.console.closed:
                break
            if time.monotonic() >= self.until:
                raise AssertionError("the guest never closed the scene connection")
            time.sleep(POLL)
        self.trace.finish()
        missing = [tid for tid in reaps if not self.trace.room("reap", tid)]
        assert not missing, f"these fault tasks were never reclaimed: {missing}"
        for phase, spec in want.items():
            self.settle(phase, spec)

    def settle(self, phase, spec):
        """每阶段按最终读数重核：入队次数、放行次数、离场原因、回收与等待位身份。"""
        state = self.phases[phase]
        final = state.setdefault("final", {})
        for tid in state["tids"]:
            entry = final.setdefault(str(tid), {})
            entry["waits"] = len(self.trace.room("wait", tid))
            entry["wakes"] = len(self.trace.room("wake", tid))
            entry["reaps"] = len(self.trace.room("reap", tid))
            entry["exits"] = [hex(e["reason"]) for e in self.trace.room("exit", tid)]
            entry["fault_killed"] = [hex(e["stval"]) for e in self.trace.room("fault_killed", tid)]
            assert entry["waits"] == spec["waits"][tid], (
                f"phase {phase}: tid {tid} queued {entry['waits']} times, wanted {spec['waits'][tid]}"
            )
            assert entry["wakes"] == spec["wakes"][tid], (
                f"phase {phase}: tid {tid} woke {entry['wakes']} times, wanted {spec['wakes'][tid]}"
            )
            wanted = spec["exits"][tid]
            assert entry["exits"] == ([] if wanted is None else [hex(wanted)]), (
                f"phase {phase}: tid {tid} ended as {entry['exits']}, wanted {wanted}"
            )
            assert entry["reaps"] == 1, (
                f"phase {phase}: tid {tid} reaped {entry['reaps']} times, wanted 1"
            )
        key = state["key"]
        on_key = {
            entry["tid"]
            for event in self.trace.events
            for entry in [room_payload(event, "wait") or {}]
            if entry.get("key") == key
        }
        assert on_key == set(state["tids"]), (
            f"phase {phase}: the wait slot named {sorted(on_key)}, wanted {state['tids']}"
        )
        state["on_key"] = sorted(on_key)

    def summary(self):
        return {
            "scene": "memory-transfer",
            "marker": PREFIX.strip(),
            "release_debug_assertions": os.environ.get(
                "CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS", ""
            ),
            "events": len(self.trace.events),
            "phases": self.phases,
        }

    def write_summary(self, output):
        text = json.dumps(self.summary(), indent=2, sort_keys=True)
        pathlib.Path(output).write_text(text + "\n")
