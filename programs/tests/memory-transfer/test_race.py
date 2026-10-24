#! /usr/bin/env python3
"""真实多 Hart 交错那一景宿主侧的回归：判据那几条规则不许被削弱。

只 mock **宿主那一面**（串行那一条线、导出文件），不模拟 KernelTask、不模拟 QEMU：
真场景的读数在 `trace/scene-memory-transfer-race-*.json` 里。
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
    "memory_transfer_race", HERE / "race.py", submodule_search_locations=[str(HERE)]
)
DRIVER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = DRIVER
SPEC.loader.exec_module(DRIVER)
Traces = sys.modules[f"{SPEC.name}.traces"].Traces

# 一跑真读数（与 `trace/` 里那一份 summary 同源）。
READY = "memory-transfer-race: phase=fault peer=11 va0=0x1d000 va1=0x1e000 read0=0x7f800"
MOVED = "memory-transfer-race: phase=fault transfer=Ok(()) kernel=armed foreign=1"
READ = "memory-transfer-race: phase=fault read1=0x7f800 read0=0x7f800"
TORN = (
    "memory-transfer-race: phase=teardown transfer=Ok(()) kernel=armed "
    "press=2->1 peer1=1->0 released=0x0 resumed=1"
)
TORN_READ = "memory-transfer-race: phase=teardown read0=0x7f800"
JOINED = "memory-transfer-race: phase=fault joined cause=Reap reason=0x0"
CLOSED = "memory-transfer-race: close done"
PEER, VA0, VA1 = 11, 0x1D000, 0x1E000
# 一跑真导出的折叠 key：判定不认它（身份靠已知 tid），只留作现场。
KEY = 12525070699606571938


def room(kind, hart, **payload):
    return {"h": hart, "kind": {"room": {kind: payload}}}


def fault(va, resolved, hart=1):
    """一次取数缺页；`h` 只是边界读数（判定不认它）。"""
    return {
        "h": hart,
        "kind": {"memory": {"page_fault": {"va": va, "fault": "Load", "resolved": resolved}}},
    }


def over(line, **fields):
    """把这一行里的具名读数换掉（只用于负对照）。"""
    for key, value in fields.items():
        line = re.sub(rf"(?<![a-z_]){key}=[^ ]+", f"{key}={value}", line, count=1)
    return line


def proven():
    """一跑真导出：对端在目标页上先无承载地等一次，被那一次移交放行后物化出同一个承载。"""
    return [
        room("spawn", 1, tid=PEER),
        fault(VA1, False),
        room("wait", 2, tid=PEER, key=KEY),
        room("wake", 3, tid=PEER),
        fault(VA1, True, hart=0),
        room("exit", 0, tid=PEER, reason=0),
        room("reap", 0, tid=PEER),
    ]


class RaceObserverTests(unittest.TestCase):
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

    def lines(self, **overrides):
        """本景真读数那几行，其中若干行被负对照换过（参数名就是本文件里那几行的名字）。"""
        base = {
            "READY": READY,
            "MOVED": MOVED,
            "READ": READ,
            "TORN": TORN,
            "TORN_READ": TORN_READ,
            "JOINED": JOINED,
        }
        chosen = dict(base)
        chosen.update(overrides)
        return [chosen[name] for name in base] + [CLOSED]

    def judge(self, lines=None, events=None, text=None):
        """一张完整的宿主面：五行真读数 ＋ 一份导出。"""
        if text is None:
            if events is None:
                events = proven()
            text = "".join(json.dumps(item) + "\n" for item in events)
        self.path.write_text(text)
        trace = Traces(self.path)
        console = DRIVER.Console(
            self.scripted(lines if lines is not None else self.lines()), str(self.capture)
        )
        return DRIVER.judge(console, trace)

    def test_accepts_the_proven_run(self):
        summary = self.judge()
        peer = summary["peer"]
        self.assertEqual(summary["scene"], "memory-transfer-race")
        self.assertEqual(summary["marker"], "memory-transfer-race:")
        self.assertIn("release_debug_assertions", summary)
        self.assertEqual(peer["tid"], PEER)
        self.assertEqual(peer["va0"], "0x1d000")
        self.assertEqual(peer["va1"], "0x1e000")
        self.assertEqual(peer["read0"], "0x7f800")
        self.assertEqual(peer["read1"], "0x7f800")
        self.assertEqual(peer["kernel"], "armed")
        self.assertEqual(peer["foreign"], 1)
        self.assertEqual(peer["waits"], 1)
        self.assertEqual(peer["wakes"], 1)
        self.assertEqual(peer["parks"], 0)
        self.assertEqual(peer["exits"], ["0x0"])
        self.assertEqual(peer["reaps"], 1)
        self.assertEqual(
            [(entry["fault"], entry["resolved"]) for entry in peer["faults_at_va1"]],
            [("Load", False), ("Load", True)],
        )
        self.assertEqual(peer["faults_at_va0"], [])
        teardown = summary["teardown"]
        self.assertEqual(teardown["kernel"], "armed")
        self.assertEqual(teardown["press_before"], 2)
        self.assertEqual(teardown["press_after"], 1)
        self.assertEqual(teardown["peer_before"], 1)
        self.assertEqual(teardown["peer_after"], 0)
        self.assertEqual(teardown["released"], "0x0")
        self.assertEqual(teardown["resumed"], 1)
        self.assertEqual(teardown["read0"], "0x7f800")
        self.assertEqual(summary["events"], len(proven()))

    def test_accepts_readings_that_moved_harts(self):
        """两笔事件的 hart 只是边界读数：任务可以在事件之间被迁走，故它不参与判定。"""
        summary = self.judge(events=[dict(item, h=7) for item in proven()])
        self.assertEqual(summary["peer"]["tid"], PEER)

    def test_accepts_a_settled_fault_on_page_zero(self):
        """第 0 页上已结算的缺页可以有（这条关系被不被提前发布不是本景要证的事）。"""
        events = proven()
        events.insert(1, fault(VA0, True, hart=2))
        summary = self.judge(events=events)
        self.assertEqual(len(summary["peer"]["faults_at_va0"]), 1)

    def test_rejects_a_peer_that_waited_on_page_zero(self):
        """第 0 页上一次未结算的缺页就是“在这页上等过”：对端第一遍没读到内容。"""
        events = proven()
        events.insert(1, fault(VA0, False, hart=2))
        with self.assertRaisesRegex(AssertionError, "waited on page 0"):
            self.judge(events=events)

    def test_rejects_a_fixture_that_timed_out(self):
        """夹具没在窗口里等到登记（多 Hart 只有一颗时必然到点）：那一档就叫 timeout。"""
        with self.assertRaisesRegex(AssertionError, "kernel=timeout"):
            self.judge(lines=self.lines(MOVED=over(MOVED, kernel="timeout")))

    def test_rejects_a_timeout_that_saw_no_registration(self):
        """到点那一档连一次真等待都没有：导出的形状再“顺”也不许把它收成通过。"""
        events = [
            room("spawn", 1, tid=PEER),
            fault(VA1, True, hart=0),
            room("exit", 0, tid=PEER, reason=0),
            room("reap", 0, tid=PEER),
        ]
        with self.assertRaisesRegex(AssertionError, "kernel=timeout"):
            self.judge(lines=self.lines(MOVED=over(MOVED, kernel="timeout")), events=events)

    def test_rejects_a_source_without_a_foreign_relation(self):
        """夹具没在别人空间里看到压着源页的翻译：这次移交没从别人那儿撕下东西。"""
        with self.assertRaisesRegex(AssertionError, "foreign=0"):
            self.judge(lines=self.lines(MOVED=over(MOVED, foreign="0")))

    def test_rejects_a_different_read_back(self):
        """对端读到的不是它自己报的那份花纹：搬过去的不是同一张承载。"""
        with self.assertRaisesRegex(AssertionError, r"read 0x7f000, not the 0x7f800"):
            self.judge(lines=self.lines(READ=over(READ, read1="0x7f000")))

    def test_rejects_a_zero_read_back(self):
        """新清零页的读数：承载没搬过去（那一读落在一张空页上）。"""
        with self.assertRaisesRegex(AssertionError, "read a zero page after the transfer"):
            self.judge(lines=self.lines(READ=over(READ, read1="0x0")))

    def test_rejects_a_teardown_that_timed_out(self):
        """夹具没在窗口里看到那一次拆图落地（多 Hart 不足时必然到点）：那一档就叫 timeout。"""
        with self.assertRaisesRegex(AssertionError, "kernel=timeout"):
            self.judge(lines=self.lines(TORN=over(TORN, kernel="timeout")))

    def test_rejects_a_window_that_never_saw_both_relations(self):
        """窗口里压着源页的关系不足两条：那次清理没有“已经取得、还没落地”的现场。"""
        with self.assertRaisesRegex(AssertionError, "wanted two"):
            self.judge(lines=self.lines(TORN=over(TORN, press="1->1")))

    def test_rejects_a_teardown_that_did_not_drop_the_relation(self):
        """放行之后那条关系还在：拆图没在同一个窗口里真落地。"""
        with self.assertRaisesRegex(AssertionError, "wanted one"):
            self.judge(lines=self.lines(TORN=over(TORN, press="2->2")))

    def test_rejects_a_foreign_pte_that_survived_the_teardown(self):
        """别人空间那个 VA 上的翻译还在：那不是干净的拆图。"""
        with self.assertRaisesRegex(AssertionError, "survived the teardown"):
            self.judge(lines=self.lines(TORN=over(TORN, peer1="1->1")))

    def test_rejects_a_foreign_pte_that_was_never_there(self):
        """窗口里从没看到那枚翻译：这次移交没有从别人空间里撕下一格的现场。"""
        with self.assertRaisesRegex(AssertionError, "before the teardown"):
            self.judge(lines=self.lines(TORN=over(TORN, peer1="0->0")))

    def test_rejects_a_pause_that_never_resumed(self):
        """夹具放行了但那一次拆图没从暂停里回来：它没真的走过 finish 那一段。"""
        with self.assertRaisesRegex(AssertionError, "never resumed"):
            self.judge(lines=self.lines(TORN=over(TORN, resumed="0")))

    def test_rejects_a_failed_release(self):
        """对端那一次 `Release` 没成功：结果码按真读数核。"""
        with self.assertRaisesRegex(AssertionError, "release answered"):
            self.judge(lines=self.lines(TORN=over(TORN, released="0xffffffffffffffff")))

    def test_rejects_a_zero_read_after_the_teardown(self):
        """拆图之后第 0 页读成零：承载没在两次移交之间留住。"""
        with self.assertRaisesRegex(AssertionError, "reads zeros after the teardown"):
            self.judge(
                lines=self.lines(
                    TORN_READ="memory-transfer-race: phase=teardown read0=0x0"
                )
            )

    def test_rejects_a_teardown_read_of_the_wrong_pattern(self):
        """第 0 页上的花纹不对：搬过去的不是同一份承载。"""
        with self.assertRaisesRegex(AssertionError, "not the 0x7f800 it should"):
            self.judge(
                lines=self.lines(
                    TORN_READ="memory-transfer-race: phase=teardown read0=0x7f000"
                )
            )

    def test_rejects_a_target_page_that_never_waited(self):
        """目标页上只有一次已结算的物化缺页（对端没等过）：冷缺页那一手没发生。"""
        events = [item for item in proven() if item != fault(VA1, False)]
        with self.assertRaisesRegex(AssertionError, "wanted one unsettled load"):
            self.judge(events=events)

    def test_rejects_a_peer_that_never_registered(self):
        """导出里一次真登记都没有（控制台却报 armed）：那一格靠不住，本景当场红。"""
        events = [
            item
            for item in proven()
            if item not in (room("wait", 2, tid=PEER, key=KEY), room("wake", 3, tid=PEER))
        ]
        with self.assertRaisesRegex(AssertionError, "queued 0 times"):
            self.judge(events=events)

    def test_rejects_a_wake_that_precedes_the_registration(self):
        """放行早于登记：把它叫醒的不是这一次等待。"""
        wait = room("wait", 2, tid=PEER, key=KEY)
        wake = room("wake", 3, tid=PEER)
        events = [item for item in proven() if item not in (wait, wake)]
        events[1:1] = [wake, wait]
        with self.assertRaisesRegex(AssertionError, "out of order"):
            self.judge(events=events)

    def test_rejects_a_second_registration(self):
        """那一次等待不是唯一那一笔：它对不上“恰一次真登记”。"""
        with self.assertRaisesRegex(AssertionError, "queued 2 times"):
            self.judge(events=proven() + [room("wait", 3, tid=PEER, key=KEY)])

    def test_rejects_a_second_reap(self):
        """对端被回收了不止一次：那一条 tid 上的结束不是唯一那一次。"""
        with self.assertRaisesRegex(AssertionError, "reclaimed 2 times"):
            self.judge(events=proven() + [room("reap", 3, tid=PEER)])

    def test_rejects_a_peer_that_ended_uncleanly(self):
        """对端不是带着 EXIT_OK 走的：它的出口码按真读数核。"""
        unclean = room("exit", 0, tid=PEER, reason=1)
        events = [
            unclean if item == room("exit", 0, tid=PEER, reason=0) else item
            for item in proven()
        ]
        with self.assertRaisesRegex(AssertionError, "ended as"):
            self.judge(events=events)

    def test_rejects_a_truncated_final_line(self):
        """末行没写完（生产者带着半个 JSON 走了）：换行前不当作收完。"""
        items = [json.dumps(item) for item in proven()]
        text = "".join(item + "\n" for item in items[:-1]) + items[-1][:12]
        with self.assertRaisesRegex(AssertionError, "incomplete final JSON"):
            self.judge(text=text)

    def test_rejects_a_complete_line_that_is_not_json(self):
        """完整的一行不是 JSON：乱行不许被跳过，当场失败。"""
        text = "".join(json.dumps(item) + "\n" for item in proven()) + "not JSON\n"
        with self.assertRaisesRegex(AssertionError, "complete line is not JSON"):
            self.judge(text=text)

    def test_run_stages_a_summary_without_publishing_success(self):
        trace_dir = self.root / "target" / "nested" / "trace"
        trace_dir.mkdir(parents=True)
        capture = trace_dir / "scene-memory-transfer-race-feed.log"
        (self.root / "sqware-diagnose.jsonl").write_text(
            "".join(json.dumps(item) + "\n" for item in proven())
        )
        pending = self.root / "private-pending.json"
        source = self.root / "programs" / "tests" / "memory-transfer" / "race.py"
        with patch.dict("os.environ", {"SCENE_SUMMARY_PENDING": str(pending)}), \
                patch.object(DRIVER, "__file__", str(source)):
            DRIVER.run(self.scripted(self.lines()), str(capture))
        summary = json.loads(pending.read_text())
        self.assertEqual(summary["scene"], "memory-transfer-race")
        self.assertFalse((trace_dir / "scene-memory-transfer-race-summary.json").exists())
        self.assertFalse((self.root / "sqware-diagnose.jsonl").exists())
        archived = Traces(trace_dir / "scene-memory-transfer-race-diagnose.jsonl")
        archived.poll()
        self.assertEqual(len(archived.events), len(proven()))


if __name__ == "__main__":
    unittest.main()
