#! /usr/bin/env python3
"""热 TLB 那一景宿主侧的回归：判据那几条规则不许被削弱。

只 mock **宿主那一面**（串行那一条线、导出文件），不模拟 KernelTask、不模拟 QEMU：
真场景的读数在 `trace/scene-memory-transfer-tlb-*.json` 里。
"""

import importlib.util
import json
import pathlib
import socket
import sys
import tempfile
import unittest


HERE = pathlib.Path(__file__).parent
SPEC = importlib.util.spec_from_file_location(
    "memory_transfer_tlb", HERE / "tlb.py", submodule_search_locations=[str(HERE)]
)
DRIVER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = DRIVER
SPEC.loader.exec_module(DRIVER)
Traces = sys.modules[f"{SPEC.name}.traces"].Traces

READER = "memory-transfer-tlb: reader tid=2 va=0x58000 view=39 budget=2000000"
# 一跑真读数（`trace/` 里那一份 summary 同源）。
FACTS = (
    "memory-transfer-tlb: reader tid=2 warm=0x7 warm_at=0x1201a sample=0x1206a "
    "sepc=0x1206a cause=13 stval=0x58000 byte=0x0 status=1 deadline=0x2a2a42 "
    "sstatus=0x200000022 stvec=0xfffffffffffff000"
)
CAUGHT = "memory-transfer-tlb: fault caught at the sampled load"
CLOSED = "memory-transfer-tlb: close done"
LAUNCHER, RUNNER = 3, 1


def room(kind, hart, **payload):
    return {"h": hart, "kind": {"room": {kind: payload}}}


def fault(va, kind="Store", hart=LAUNCHER):
    return {
        "h": hart,
        "kind": {"memory": {"page_fault": {"va": va, "fault": kind, "resolved": True}}},
    }


def proven():
    """一跑真导出：拉起那一笔在控制者那颗 hart，收工两笔在读者那颗。"""
    return [
        room("spawn", LAUNCHER, tid=2),
        fault(0x58000),
        room("exit", RUNNER, tid=2, reason=0),
        room("reap", RUNNER, tid=2),
    ]


class TlbObserverTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = pathlib.Path(self.directory.name)
        self.path = self.root / "events.jsonl"
        self.capture = self.root / "feed.log"

    def scripted(self, lines):
        """把这几行灌进一条真 socketpair 再关掉那一端：`Console` 只看见一根到头的流。

        用标准库的 socket，而不是自造一个桩类：这个文件便只剩一个数据结构（这个测试类），
        而 `Console` 那一侧走的是真字节流（`settimeout`/`recv`/EOF 都是真的）。
        """
        ours, theirs = socket.socketpair()
        self.addCleanup(ours.close)
        theirs.sendall("".join(line + "\n" for line in lines).encode())
        theirs.close()
        return ours

    def judge(self, facts=FACTS, verdict=CAUGHT, events=None, launcher=LAUNCHER, runner=RUNNER):
        """一张完整的宿主面：四行读数 ＋ 一份导出（它自己与场景无关）。"""
        if events is None:
            events = [
                room("spawn", launcher, tid=2),
                fault(0x58000),
                room("exit", runner, tid=2, reason=0),
                room("reap", runner, tid=2),
            ]
        self.path.write_text("".join(json.dumps(item) + "\n" for item in events))
        trace = Traces(self.path)
        trace.poll()
        console = DRIVER.Console(self.scripted([READER, facts, verdict, CLOSED]), str(self.capture))
        return DRIVER.judge(console, trace)

    def test_accepts_the_proven_run(self):
        summary = self.judge()
        self.assertEqual(summary["reader"]["status"], 1)
        self.assertEqual(summary["reader"]["cause"], 13)
        self.assertEqual(summary["reader"]["stval"], "0x58000")
        self.assertEqual(summary["kernel_load_faults_at_va"], 0)
        self.assertEqual(summary["boundary"]["launch_harts"], [LAUNCHER])
        self.assertEqual(summary["boundary"]["exit_harts"], [RUNNER])

    def test_accepts_a_boundary_that_names_one_hart(self):
        """边界读数同核**不**是拒绝理由：`SIE` 恢复后读者可以被迁走（决策见驱动那一处注释）。"""
        summary = self.judge(runner=LAUNCHER)
        self.assertEqual(summary["boundary"]["launch_harts"], [LAUNCHER])
        self.assertEqual(summary["boundary"]["exit_harts"], [LAUNCHER])

    def test_rejects_a_stale_read(self):
        stale = FACTS.replace("byte=0x0 status=1", "byte=0x7 status=2")
        with self.assertRaisesRegex(AssertionError, "the hot TLB was not revoked"):
            self.judge(facts=stale)

    def test_rejects_a_reader_timeout(self):
        late = FACTS.replace("cause=13 stval=0x58000", "cause=0 stval=0x0").replace(
            "status=1", "status=3"
        )
        with self.assertRaisesRegex(AssertionError, "never opened"):
            self.judge(facts=late)

    def test_rejects_unpublished_load_addresses(self):
        """cause/stval 都对，但窗口一处地址也没发布（sample=sepc=0）必须拒绝。"""
        unpublished = FACTS.replace("sample=0x1206a", "sample=0x0").replace(
            "sepc=0x1206a", "sepc=0x0"
        )
        with self.assertRaisesRegex(AssertionError, "never published its load addresses"):
            self.judge(facts=unpublished)

    def test_rejects_a_sample_on_the_warm_instruction(self):
        same = FACTS.replace("sample=0x1206a", "sample=0x1201a").replace(
            "sepc=0x1206a", "sepc=0x1201a"
        )
        with self.assertRaisesRegex(AssertionError, "same instruction"):
            self.judge(facts=same)

    def test_rejects_a_trap_elsewhere(self):
        elsewhere = FACTS.replace("sepc=0x1206a", "sepc=0x1201a")
        with self.assertRaisesRegex(AssertionError, "not the sampled load"):
            self.judge(facts=elsewhere)

    def test_rejects_another_cause(self):
        other = FACTS.replace("cause=13", "cause=15")
        with self.assertRaisesRegex(AssertionError, "load page fault"):
            self.judge(facts=other)

    def test_rejects_another_page(self):
        other = FACTS.replace("stval=0x58000", "stval=0x59000")
        with self.assertRaisesRegex(AssertionError, "stval"):
            self.judge(facts=other)

    def test_rejects_a_load_fault_that_reached_the_kernel(self):
        with self.assertRaisesRegex(AssertionError, "reached the kernel"):
            self.judge(events=[room("spawn", LAUNCHER, tid=2), fault(0x58000), fault(0x58000, "Load")])

    def test_reports_the_scenes_own_failure(self):
        with self.assertRaisesRegex(AssertionError, "read a byte"):
            self.judge(
                verdict="memory-transfer-tlb: FAIL the sampled load read a byte: the hot TLB was not revoked"
            )

    def test_run_writes_a_summary_and_archives_the_export(self):
        trace_dir = self.root / "trace"
        trace_dir.mkdir()
        capture = trace_dir / "scene-memory-transfer-tlb-feed.log"
        (self.root / "sqware-diagnose.jsonl").write_text(
            "".join(json.dumps(item) + "\n" for item in proven())
        )
        DRIVER.run(self.scripted([READER, FACTS, CAUGHT, CLOSED]), str(capture))
        summary = json.loads((trace_dir / "scene-memory-transfer-tlb-summary.json").read_text())
        self.assertEqual(summary["scene"], "memory-transfer-tlb")
        self.assertFalse((self.root / "sqware-diagnose.jsonl").exists())
        archived = Traces(trace_dir / "scene-memory-transfer-tlb-diagnose.jsonl")
        archived.poll()
        self.assertEqual(len(archived.events), 4)


if __name__ == "__main__":
    unittest.main()
