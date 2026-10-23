"""Negative controls for the forced-shutdown branch of the shared checker."""

import contextlib
import copy
import io
import unittest

from fixtures.forced import CONSOLE, EVENTS, HELD
from fixtures.ruin import BACKEND
from ruin_check import completed, pending


class ForcedEvidence(unittest.TestCase):
    def test_actual_entry_identity_controls_forced_exit_and_retirement(self):
        console = CONSOLE.replace("entry task=1", "entry task=31")
        events = copy.deepcopy(EVENTS)
        for event in events:
            for record in event.get("kind", {}).get("room", {}).values():
                if record.get("tid") == 1:
                    record["tid"] = 31
        with contextlib.redirect_stdout(io.StringIO()):
            completed(console, events, BACKEND, "forced")

    def test_different_task_cannot_supply_root_exit(self):
        with self.assertRaises(AssertionError):
            completed(CONSOLE.replace("entry task=1", "entry task=31"), EVENTS, BACKEND, "forced")

    def test_success(self):
        pending(HELD, EVENTS[:2], "forced")
        with contextlib.redirect_stdout(io.StringIO()):
            completed(CONSOLE, EVENTS, BACKEND, "forced")

    def test_normal_shutdown_cannot_substitute(self):
        for line in (
            "system: idle deadline reached; awaiting DMA settlement\n",
            "ctl: awaiting forced global shutdown with in-flight read\n",
        ):
            with self.subTest(line=line), self.assertRaises(AssertionError):
                pending(HELD.replace(line, ""), EVENTS[:2], "forced")

    def test_each_provider_survives_the_held_request(self):
        for tid in (4, 6, 7, 8, 9):
            event = {"when": 3, "kind": {"room": {"reap": {"tid": tid}}}}
            with self.subTest(tid=tid), self.assertRaisesRegex(AssertionError, "before DMA"):
                pending(HELD, EVENTS[:2] + [event], "forced")

    def test_admission_before_settlement_is_rejected(self):
        line = "system: forced shutdown admitted after DMA settlement\n"
        with self.assertRaisesRegex(AssertionError, "premature"):
            pending(HELD + line, EVENTS[:2], "forced")
        with self.assertRaisesRegex(AssertionError, "out of order"):
            completed(line + CONSOLE.replace(line, ""), EVENTS, BACKEND, "forced")

    def test_each_provider_must_be_retired(self):
        for tid in (4, 6, 7, 8, 9):
            events = [
                event for event in EVENTS
                if event.get("kind", {}).get("room", {}).get("reap", {}).get("tid") != tid
            ]
            with self.subTest(tid=tid), self.assertRaises(AssertionError):
                completed(CONSOLE, events, BACKEND, "forced")

    def test_unrelated_root_failure_cannot_substitute(self):
        events = copy.deepcopy(EVENTS)
        events[-3]["kind"]["room"]["exit"]["reason"] = 9
        with self.assertRaisesRegex(AssertionError, "self-doom"):
            completed(CONSOLE, events, BACKEND, "forced")

    def test_native_data_proof_is_required(self):
        with self.assertRaises(AssertionError):
            completed(CONSOLE.replace("sector_ok=true", "sector_ok=false"), EVENTS, BACKEND, "forced")


if __name__ == "__main__":
    unittest.main()
