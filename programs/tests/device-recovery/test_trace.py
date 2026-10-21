"""Require genuine auxiliary execution evidence, not just remote termination."""

import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest

from trace import check


class ExecutionEvidence(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.console = Path(self.directory.name) / "console"
        self.trace = self.console.with_suffix(".jsonl")
        lines = []
        self.events = []
        for serial, main in ((1, 10), (2, 12)):
            auxiliary = main + 1
            lines.extend((
                f"ctl: constructed device-work tid={main} owner=hub",
                f"ctl: binding {serial} auxiliary={auxiliary} alive=true",
                f"ctl: binding {serial} auxiliary={auxiliary} alive=false",
            ))
            self.events.extend((
                {"h": 0, "kind": {"room": {"wait": {"tid": main}}}},
                {"h": 1, "kind": {"room": {"park": {"tid": auxiliary}}}},
                {"h": 0, "kind": {"room": {"reap": {"tid": main}}}},
                {"h": 0, "kind": {"room": {"reap": {"tid": auxiliary}}}},
            ))
        self.console.write_text("\n".join(lines) + "\n")

    def check(self):
        self.trace.write_text("\n".join(map(json.dumps, self.events)) + "\n")
        with contextlib.redirect_stdout(io.StringIO()):
            check(self.console, self.trace)

    def test_auxiliary_park_is_real_execution_on_that_hart(self):
        self.check()

    def test_remote_exit_and_reap_are_not_execution_evidence(self):
        self.events[1]["kind"]["room"] = {"exit": {"tid": 11, "reason": 0xFFFF_FFFE}}
        with self.assertRaisesRegex(AssertionError, "distinct-Hart"):
            self.check()

    def test_same_hart_does_not_pass(self):
        self.events[1]["h"] = 0
        with self.assertRaisesRegex(AssertionError, "distinct-Hart"):
            self.check()

    def test_reap_is_still_required(self):
        self.events.pop()
        with self.assertRaisesRegex(AssertionError, "reaping"):
            self.check()

    def test_expected_generation_count_is_still_required(self):
        self.console.write_text(self.console.read_text().split("ctl: constructed device-work tid=12")[0])
        with self.assertRaisesRegex(AssertionError, "all generations"):
            self.check()


if __name__ == "__main__":
    unittest.main()
