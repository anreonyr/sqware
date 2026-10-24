"""Long-run proof must retain the same safety checks and external expected count."""

import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest

from fixtures.soak import evidence
from soak import check


class SoakEvidence(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.console_path = Path(self.directory.name) / "console"
        self.events_path = self.console_path.with_suffix(".jsonl")

    def check(self, console, events, expected=128):
        self.console_path.write_text(console)
        self.events_path.write_text("\n".join(map(json.dumps, events)) + "\n")
        with contextlib.redirect_stdout(io.StringIO()):
            check(self.console_path, self.events_path, expected)

    def test_both_declared_generation_counts(self):
        for count in (128, 1024):
            with self.subTest(count=count):
                self.check(*evidence(count), count)

    def test_partial_run_cannot_select_its_own_smaller_target(self):
        with self.assertRaisesRegex(AssertionError, "all generations"):
            self.check(*evidence(128), 1024)

    def test_one_bad_dma_reply_is_rejected(self):
        console, events = evidence(128)
        with self.assertRaisesRegex(AssertionError, "real DMA"):
            self.check(console.replace("sector_ok=true", "sector_ok=false", 1), events)

    def test_all_dma_receipts_cannot_be_moved_before_the_first_active_binding(self):
        console, events = evidence(128)
        served = "ctl: served used=1 len=513 written_ok=true sector_ok=true status=0\n"
        changed = served * 128 + console.replace(served, "")
        with self.assertRaises(AssertionError):
            self.check(changed, events)

    def test_duplicate_dma_in_one_generation_cannot_replace_another_generation(self):
        console, events = evidence(128)
        served = "ctl: served used=1 len=513 written_ok=true sector_ok=true status=0\n"
        first = console.index(served)
        second = console.index(served, first + len(served))
        changed = console[:second] + console[second:].replace(served, "", 1)
        changed = changed[:first] + served + changed[first:]
        with self.assertRaises(AssertionError):
            self.check(changed, events)

    def test_dma_receipt_after_the_final_generation_cannot_replace_a_missing_reply(self):
        console, events = evidence(128)
        served = "ctl: served used=1 len=513 written_ok=true sector_ok=true status=0\n"
        with self.assertRaises(AssertionError):
            self.check(console.replace(served, "", 1) + served, events)

    def test_dma_must_be_after_active_and_before_execution_stop(self):
        console, events = evidence(128)
        served = "ctl: served used=1 len=513 written_ok=true sector_ok=true status=0\n"
        before = "recovery: binding 1 active"
        start = console.index(before)
        changed = console.replace(served, "", 1)
        changed = changed[:start] + served + changed[start:]
        with self.assertRaisesRegex(AssertionError, "out of order"):
            self.check(changed, events)
        stopped = "hub: binding 1 execution domain stopped (tid=22)\n"
        changed = console.replace(served, "", 1).replace(stopped, stopped + served, 1)
        with self.assertRaisesRegex(AssertionError, "out of order"):
            self.check(changed, events)

    def test_truncated_trace_is_rejected(self):
        console, events = evidence(128)
        with self.assertRaisesRegex(AssertionError, "incomplete trace"):
            self.check(console, events[:-1])

    def test_capability_growth_or_duplicate_channels_are_rejected(self):
        console, events = evidence(128)
        with self.assertRaisesRegex(AssertionError, "table grew"):
            self.check(console.replace(
                "generation 2 capabilities=12", "generation 2 capabilities=13",
            ), events)
        for field in ("launch", "command"):
            with self.subTest(field=field), self.assertRaises(AssertionError):
                self.check(console.replace(f"{field}=1", f"{field}=2", 1), events)

    def test_missing_or_misplaced_capability_receipt_is_rejected(self):
        console, events = evidence(128)
        receipt = "ctl: generation 1 capabilities=12 launch=1 command=1\n"
        with self.assertRaises(AssertionError):
            self.check(console.replace(receipt, "", 1), events)
        changed = console.replace(receipt, "", 1).replace(
            "ctl: served used=1", receipt + "ctl: served used=1", 1,
        )
        with self.assertRaisesRegex(AssertionError, "out of order"):
            self.check(changed, events)

    def test_binding_cannot_progress_after_its_receipt(self):
        console, events = evidence(128)
        with self.assertRaisesRegex(AssertionError, "final generation"):
            self.check(console + "hub: binding 1 closed and released device 41\n", events)

    def test_one_resident_cannot_exit_before_the_last_generation(self):
        console, events = evidence(128)
        beta = next(event for event in events if event.get("kind", {}).get("room", {}).get("reap", {}).get("tid") == 8)
        beta["when"] = 0
        with self.assertRaisesRegex(AssertionError, "remain resident"):
            self.check(console, events)

    def test_early_halt_or_physical_leak_cannot_substitute(self):
        console, events = evidence(128)
        with self.assertRaisesRegex(AssertionError, "idle deadline"):
            self.check(console.replace("elapsed_ms=20000", "elapsed_ms=10000"), events)
        with self.assertRaisesRegex(AssertionError, "leaked"):
            self.check(console.replace("after=85", "after=86"), events)


if __name__ == "__main__":
    unittest.main()
