#! /usr/bin/env python3
"""Fresh 失败回退那一景宿主侧的回归：判据那几条规则不许被削弱。

只 mock **宿主那一面**（串行那一条线、导出文件），不模拟 KernelTask、不模拟 QEMU：
真场景的读数在 `trace/scene-memory-transfer-fresh-*.json` 里。
"""

import importlib.util
import json
import pathlib
import re
import socket
import sys
import tempfile
import unittest
from unittest.mock import patch


HERE = pathlib.Path(__file__).parent
SPEC = importlib.util.spec_from_file_location(
    "memory_transfer_fresh", HERE / "fresh.py", submodule_search_locations=[str(HERE)]
)
DRIVER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = DRIVER
SPEC.loader.exec_module(DRIVER)
Traces = sys.modules[f"{SPEC.name}.traces"].Traces

# 一跑真读数（与 `trace/` 里那一份 summary 同源）。
A_READY = (
    "memory-transfer-fresh: phase=A source=0x61000 target=0x62000 view=120 reader=11"
)
A_TRANSFER = "memory-transfer-fresh: phase=A transfer=MemoryFail::Busy kernel=armed"
A_READER = "memory-transfer-fresh: phase=A reader=11 value=0x0"
A_SOURCE = "memory-transfer-fresh: phase=A source=0x61000 sum=0x7f800"
B_READY = "memory-transfer-fresh: phase=B source=0x63000 target=0x64000 view=122"
B_TRANSFER = "memory-transfer-fresh: phase=B transfer=MemoryFail::Busy kernel=armed"
B_LATE = "memory-transfer-fresh: phase=B late=12 value=0x0"
CLOSED = "memory-transfer-fresh: close done"
READER, LATE = 11, 12
A_TARGET, B_TARGET = 0x62000, 0x64000
A_SOURCE_PAGE = 0x61000


def room(kind, hart, **payload):
    return {"h": hart, "kind": {"room": {kind: payload}}}


def over(line, **fields):
    """把这一行里的具名读数换掉（只用于负对照）。"""
    for key, value in fields.items():
        line = re.sub(rf"(?<![a-z_]){key}=[^ ]+", f"{key}={value}", line, count=1)
    return line


def fault(va, kind="Load", resolved=True, hart=2):
    return {
        "h": hart,
        "kind": {"memory": {"page_fault": {"va": va, "fault": kind, "resolved": resolved}}},
    }


def proven():
    """一跑真导出：等待者在那条关系上入队一次、放行一次，两阶段的页都真的物化过。"""
    return [
        room("spawn", 0, tid=READER),
        fault(A_SOURCE_PAGE, "Store"),
        fault(A_TARGET, resolved=False),
        room("wait", READER, tid=READER, key=5),
        room("wake", 0, tid=READER),
        fault(A_TARGET, resolved=True),
        room("exit", 2, tid=READER, reason=0),
        room("reap", 2, tid=READER),
        fault(A_SOURCE_PAGE, "Load"),
        room("spawn", 0, tid=LATE),
        fault(B_TARGET, resolved=True),
        room("exit", 2, tid=LATE, reason=0),
        room("reap", 2, tid=LATE),
    ]


class FreshObserverTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = pathlib.Path(self.directory.name)
        self.path = self.root / "events.jsonl"
        self.capture = self.root / "feed.log"

    def scripted(self, lines):
        """把这几行灌进一条真 socketpair 再关掉那一端：`Console` 只看见一根到头的流。"""
        ours, theirs = socket.socketpair()
        self.addCleanup(ours.close)
        theirs.sendall("".join(line + "\n" for line in lines).encode())
        theirs.close()
        return ours

    def lines(self, **overrides):
        """八行真读数，其中若干行被负对照换过（参数名就是本文件里那几行的名字）。"""
        base = {
            "A_READY": A_READY,
            "A_TRANSFER": A_TRANSFER,
            "A_READER": A_READER,
            "A_SOURCE": A_SOURCE,
            "B_READY": B_READY,
            "B_TRANSFER": B_TRANSFER,
            "B_LATE": B_LATE,
        }
        chosen = dict(base)
        chosen.update(overrides)
        return [chosen[name] for name in base] + [CLOSED]

    def judge(self, lines=None, events=None):
        """一张完整的宿主面：八行读数 ＋ 一份导出。"""
        if events is None:
            events = proven()
        self.path.write_text("".join(json.dumps(item) + "\n" for item in events))
        trace = Traces(self.path)
        trace.poll()
        console = DRIVER.Console(
            self.scripted(lines if lines is not None else self.lines()), str(self.capture)
        )
        return DRIVER.judge(console, trace)

    def test_accepts_the_proven_run(self):
        summary = self.judge()
        self.assertEqual(summary["waiter"]["tid"], READER)
        self.assertEqual(summary["waiter"]["waits"], 1)
        self.assertEqual(summary["waiter"]["wakes"], 1)
        self.assertEqual(summary["waiter"]["parks"], 0)
        self.assertEqual(summary["waiter"]["exits"], ["0x0"])
        self.assertEqual(summary["late"]["waits"], 0)
        self.assertEqual(summary["late"]["exits"], ["0x0"])
        self.assertEqual(summary["source"]["sum"], "0x7f800")

    def test_rejects_a_dirty_target_page(self):
        """读者读出的不是零：目标页不是新清零页（同一张旧承载留在那里）。"""
        with self.assertRaisesRegex(AssertionError, "not a fresh zero page"):
            self.judge(lines=self.lines(A_READER=over(A_READER, value="0x7f800")))

    def test_rejects_a_late_waiter(self):
        """晚到那一台等过：失败之后目标仍是 Reserved（回退没还回 Fresh）。"""
        events = proven() + [room("wait", LATE, tid=LATE, key=7)]
        with self.assertRaisesRegex(AssertionError, "the late reader waited"):
            self.judge(events=events)

    def test_rejects_a_late_park(self):
        events = proven() + [room("park", LATE, tid=LATE, wake_at=1)]
        with self.assertRaisesRegex(AssertionError, "the late reader parked"):
            self.judge(events=events)

    def test_rejects_a_late_reader_that_never_materialized(self):
        """晚到者那页没有已结算的物化缺页：它不是直接物化出来的。"""
        events = [item for item in proven() if item != fault(B_TARGET, resolved=True)]
        with self.assertRaisesRegex(AssertionError, "wanted exactly one settled load"):
            self.judge(events=events)

    def test_rejects_a_waiter_that_never_queued(self):
        """等待者一次也没入队：那次失败根本没有真实等待者。"""
        events = [item for item in proven() if item != room("wait", READER, tid=READER, key=5)]
        with self.assertRaisesRegex(AssertionError, "queued 0 times"):
            self.judge(events=events)

    def test_rejects_a_waiter_that_parked(self):
        events = proven() + [room("park", READER, tid=READER, wake_at=1)]
        with self.assertRaisesRegex(AssertionError, "parked instead of waiting"):
            self.judge(events=events)

    def test_rejects_a_wait_that_was_never_declared_resolved(self):
        """目标页只缺过一次未结算的页：被放行之后没真的重试物化。"""
        events = [item for item in proven() if item != fault(A_TARGET, resolved=True)]
        with self.assertRaisesRegex(AssertionError, "wanted one unsettled load"):
            self.judge(events=events)

    def test_rejects_a_source_without_the_post_withdraw_load(self):
        events = [item for item in proven() if item != fault(A_SOURCE_PAGE, "Load")]
        with self.assertRaisesRegex(AssertionError, "settled Load after the failed move"):
            self.judge(events=events)

    def test_rejects_a_source_refault_on_another_page(self):
        events = [
            fault(A_SOURCE_PAGE + 4 * 4096, "Load") if item == fault(A_SOURCE_PAGE, "Load") else item
            for item in proven()
        ]
        with self.assertRaisesRegex(AssertionError, "settled Load after the failed move"):
            self.judge(events=events)

    def test_rejects_a_source_store_instead_of_a_reload(self):
        events = [
            fault(A_SOURCE_PAGE, "Store") if item == fault(A_SOURCE_PAGE, "Load") else item
            for item in proven()
        ]
        with self.assertRaisesRegex(AssertionError, "settled Load after the failed move"):
            self.judge(events=events)

    def test_rejects_a_duplicate_late_materialization(self):
        with self.assertRaisesRegex(AssertionError, "wanted exactly one settled load"):
            self.judge(events=proven() + [fault(B_TARGET)])

    def test_rejects_a_late_store_instead_of_a_load(self):
        events = [
            fault(B_TARGET, "Store") if item == fault(B_TARGET) else item for item in proven()
        ]
        with self.assertRaisesRegex(AssertionError, "wanted exactly one settled load"):
            self.judge(events=events)

    def test_rejects_an_extra_waiter_fault(self):
        with self.assertRaisesRegex(AssertionError, "wanted one unsettled load"):
            self.judge(events=proven() + [fault(A_TARGET, "Store")])

    def test_rejects_a_waiter_store_instead_of_a_load(self):
        events = [
            fault(A_TARGET, "Store", resolved=False)
            if item == fault(A_TARGET, resolved=False) else item
            for item in proven()
        ]
        with self.assertRaisesRegex(AssertionError, "wanted one unsettled load"):
            self.judge(events=events)

    def test_rejects_a_spurious_late_wake(self):
        with self.assertRaisesRegex(AssertionError, "woke without a wait"):
            self.judge(events=proven() + [room("wake", 0, tid=LATE)])

    def test_rejects_an_early_late_spawn(self):
        spawn = room("spawn", 0, tid=LATE)
        events = [spawn] + [item for item in proven() if item != spawn]
        with self.assertRaisesRegex(AssertionError, "spawned before phase A finished reading"):
            self.judge(events=events)

    def test_rejects_shared_reader_identity(self):
        with self.assertRaisesRegex(AssertionError, "same tid"):
            self.judge(lines=self.lines(B_LATE=over(B_LATE, late=str(READER))))

    def test_rejects_a_missing_late_spawn(self):
        events = [item for item in proven() if item != room("spawn", 0, tid=LATE)]
        with self.assertRaisesRegex(AssertionError, "late reader was never spawned"):
            self.judge(events=events)

    def test_rejects_a_duplicate_late_reap(self):
        with self.assertRaisesRegex(AssertionError, "late reader was not reclaimed once"):
            self.judge(events=proven() + [room("reap", 2, tid=LATE)])

    def test_rejects_a_transfer_that_committed(self):
        committed = over(A_TRANSFER, transfer="MemoryFail::None")
        with self.assertRaisesRegex(AssertionError, "not 'MemoryFail::Busy'"):
            self.judge(lines=self.lines(A_TRANSFER=committed))

    def test_rejects_a_source_that_reads_like_a_fresh_page(self):
        """源页读出来也是一张空页：那一次回退没保住同一个承载。"""
        empty = over(A_SOURCE, sum="0x0")
        with self.assertRaisesRegex(AssertionError, "reads like a fresh zero page"):
            self.judge(lines=self.lines(A_SOURCE=empty))

    def test_rejects_the_same_target_in_both_phases(self):
        """两阶段共用一页：分不清哪一笔事件属于哪一阶段。"""
        shared = over(B_READY, source="0x62000")
        with self.assertRaisesRegex(AssertionError, "two of the four pages are the same"):
            self.judge(lines=self.lines(B_READY=shared))

    def test_rejects_a_late_reader_that_ended_uncleanly(self):
        events = [
            item if item != room("exit", 2, tid=LATE, reason=0) else room("exit", 2, tid=LATE, reason=1)
            for item in proven()
        ]
        with self.assertRaisesRegex(AssertionError, "the late reader ended as"):
            self.judge(events=events)

    def test_run_stages_a_summary_without_publishing_success(self):
        trace_dir = self.root / "trace"
        trace_dir.mkdir()
        capture = trace_dir / "scene-memory-transfer-fresh-feed.log"
        (self.root / "sqware-diagnose.jsonl").write_text(
            "".join(json.dumps(item) + "\n" for item in proven())
        )
        lines = self.lines()
        pending = self.root / "private-pending.json"
        with patch.dict("os.environ", {"SCENE_SUMMARY_PENDING": str(pending)}):
            DRIVER.run(self.scripted(lines), str(capture))
        summary = json.loads(pending.read_text())
        self.assertEqual(summary["scene"], "memory-transfer-fresh")
        self.assertFalse((trace_dir / "scene-memory-transfer-fresh-summary.json").exists())
        self.assertFalse((self.root / "sqware-diagnose.jsonl").exists())
        archived = Traces(trace_dir / "scene-memory-transfer-fresh-diagnose.jsonl")
        archived.poll()
        self.assertEqual(len(archived.events), len(proven()))


if __name__ == "__main__":
    unittest.main()
