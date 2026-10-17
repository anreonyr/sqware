"""导出读取与最终断言的宿主回归，不代替真实 Task 场景。"""

import importlib.util
import json
import pathlib
import sys
import tempfile
import time
import unittest
from unittest.mock import Mock


HERE = pathlib.Path(__file__).parent
SPEC = importlib.util.spec_from_file_location(
    "memory_transfer_observer", HERE / "driver.py", submodule_search_locations=[str(HERE)]
)
DRIVER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = DRIVER
SPEC.loader.exec_module(DRIVER)
Traces = sys.modules[f"{SPEC.name}.traces"].Traces
Harness = DRIVER.Harness


def event(kind, **payload):
    return {"kind": {"room": {kind: payload}}}


class ObserverTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.path = pathlib.Path(self.directory.name) / "events.jsonl"

    def write(self, events):
        self.path.write_text("".join(json.dumps(item) + "\n" for item in events))

    def test_reads_events_written_before_construction(self):
        self.write([event("wait", tid=3, key=5)])
        traces = Traces(self.path)
        traces.poll()
        self.assertEqual(traces.room("wait", 3), [{"tid": 3, "key": 5}])

    def test_preserves_an_early_partial_line(self):
        line = json.dumps(event("wait", tid=3, key=5))
        self.path.write_text(line[:12])
        traces = Traces(self.path)
        traces.poll()
        self.assertEqual(traces.events, [])
        with self.path.open("a") as output:
            output.write(line[12:] + "\n")
        traces.finish()
        self.assertEqual(len(traces.room("wait", 3)), 1)

    def test_rejects_a_bad_complete_line(self):
        self.path.write_text("not JSON\n")
        with self.assertRaisesRegex(AssertionError, "complete line is not JSON"):
            Traces(self.path).poll()

    def consumed(self):
        self.write([event("wait", tid=3, key=5)])
        traces = Traces(self.path)
        traces.poll()
        return traces

    def test_rejects_truncation_after_consumption(self):
        traces = self.consumed()
        self.path.write_text("")
        with self.assertRaisesRegex(AssertionError, "replaced or truncated"):
            traces.poll()

    def test_rejects_replacement_after_consumption(self):
        traces = self.consumed()
        replacement = self.path.with_suffix(".new")
        replacement.write_text(self.path.read_text())
        replacement.replace(self.path)
        with self.assertRaisesRegex(AssertionError, "replaced or truncated"):
            traces.poll()

    def test_rejects_disappearance_after_consumption(self):
        traces = self.consumed()
        self.path.unlink()
        with self.assertRaisesRegex(AssertionError, "disappeared"):
            traces.poll()

    def test_rejects_an_incomplete_final_line(self):
        self.path.write_text(json.dumps(event("wait", tid=3, key=5)))
        with self.assertRaisesRegex(AssertionError, "incomplete final JSON"):
            Traces(self.path).finish()

    def test_finishes_a_complete_export(self):
        traces = self.consumed()
        traces.finish()
        self.assertEqual(len(traces.events), 1)

    def harness(self, extra=(), omit_reap=False):
        events = []
        for tid in (3, 4):
            events.extend(
                [event("wait", tid=tid, key=5), event("wake", tid=tid), event("exit", tid=tid, reason=0)]
            )
            if not omit_reap:
                events.append(event("reap", tid=tid))
        self.write(events + list(extra))
        harness = Harness.__new__(Harness)
        harness.trace = Traces(self.path)
        harness.trace.poll()
        harness.phases = {"A": {"tids": [3, 4], "key": 5}}
        return harness

    def spec(self):
        return {"waits": {3: 1, 4: 1}, "wakes": {3: 1, 4: 1}, "exits": {3: 0, 4: 0}}

    def test_settles_exact_counts(self):
        harness = self.harness()
        harness.settle("A", self.spec())
        self.assertEqual(harness.phases["A"]["on_key"], [3, 4])

    def test_rejects_a_late_extra_wake(self):
        harness = self.harness([event("wake", tid=3)])
        with self.assertRaisesRegex(AssertionError, "woke 2 times"):
            harness.settle("A", self.spec())

    def test_rejects_a_missing_reap(self):
        harness = self.harness(omit_reap=True)
        with self.assertRaisesRegex(AssertionError, "reaped 0 times"):
            harness.settle("A", self.spec())

    def test_rejects_duplicate_reclamation(self):
        harness = self.harness([event("reap", tid=3)])
        with self.assertRaisesRegex(AssertionError, "reaped 2 times"):
            harness.settle("A", self.spec())

    def test_finish_waits_for_eof_before_settling(self):
        harness = self.harness()
        harness.console = Mock(closed=False)
        harness.until = time.monotonic() + 1

        def drain():
            if harness.console.drain.call_count == 3:
                harness.console.closed = True

        harness.console.drain.side_effect = drain
        harness.finish({"A": self.spec()})
        self.assertTrue(harness.console.closed)
        self.assertEqual(harness.console.drain.call_count, 3)


if __name__ == "__main__":
    unittest.main()
