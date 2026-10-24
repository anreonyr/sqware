"""Synthetic negative controls for the native Ruin evidence checker."""

import contextlib
import copy
import io
import unittest

from ruin_check import completed, pending
from fixtures.ruin import BACKEND, CONSOLE, EVENTS, HELD


class RuinEvidence(unittest.TestCase):
    def test_actual_entry_identity_may_follow_boot_self_checks(self):
        console = CONSOLE.replace("entry task=1", "entry task=31").replace(
            "exit tid=1 reason=0x0 note: iommu-ruin", "exit tid=31 reason=0x0 note: iommu-ruin"
        )
        with contextlib.redirect_stdout(io.StringIO()):
            completed(console, EVENTS, BACKEND)

    def test_missing_ambiguous_or_wrong_entry_identity_is_rejected(self):
        for console in (
            CONSOLE.replace("boot: entry task=1\n", ""),
            CONSOLE + "boot: entry task=1\n",
            CONSOLE.replace("entry task=1", "entry task=31"),
        ):
            with self.subTest(console=console), self.assertRaises(AssertionError):
                completed(console, EVENTS, BACKEND)

    def test_success(self):
        pending(HELD, EVENTS[:2])
        with contextlib.redirect_stdout(io.StringIO()):
            completed(CONSOLE, EVENTS, BACKEND)

    def test_missing_alpha_member(self):
        with self.assertRaisesRegex(AssertionError, "completely reaped"):
            pending(HELD, EVENTS[:1])

    def test_driver_dies_while_held(self):
        with self.assertRaisesRegex(AssertionError, "Driver reaped"):
            pending(HELD, EVENTS[:3])

    def test_premature_stop_release_or_reply(self):
        for line in (
            "pci: endpoint bus-master off=0x0",
            "recovery: binding 1 stopped",
            "iommu: binding 1 released",
            "hub: binding 1 closed",
            "ruin: recovery dead after reply",
            "system: forced shutdown admitted after DMA settlement",
            "system: forced shutdown admitted after retirement settlement",
        ):
            with self.subTest(line=line), self.assertRaisesRegex(AssertionError, "premature"):
                pending(HELD + line + "\n", EVENTS[:2])

    def test_reversed_stop_chain(self):
        log = CONSOLE.replace("ruin: requested recovery\n", "")
        log = "ruin: requested recovery\n" + log
        with self.assertRaisesRegex(AssertionError, "out of order"):
            completed(log, EVENTS, BACKEND)

    def test_wrong_device(self):
        with self.assertRaises(AssertionError):
            completed(CONSOLE.replace("released device 41", "released device 42"), EVENTS, BACKEND)

    def test_queue_fault_or_retained_reference(self):
        for old, new in (("faults=0", "faults=1"), ("frames=0", "frames=1")):
            with self.subTest(field=old), self.assertRaises(AssertionError):
                completed(CONSOLE.replace(old, new), EVENTS, BACKEND)

    def test_backend_success_without_native_dma(self):
        for old, new in (("sector_ok=true", "sector_ok=false"), ("status=0 used", "status=1 used")):
            with self.subTest(field=old), self.assertRaises(AssertionError):
                completed(CONSOLE.replace(old, new), EVENTS, BACKEND)

    def test_physical_leak(self):
        with self.assertRaisesRegex(AssertionError, "not reclaimed"):
            completed(CONSOLE.replace("after=85", "after=86"), EVENTS, BACKEND)

    def test_truncated_trace(self):
        with self.assertRaisesRegex(AssertionError, "incomplete trace"):
            completed(CONSOLE, EVENTS[:-1], BACKEND)

    def test_driver_reaped_before_alpha(self):
        events = copy.deepcopy(EVENTS)
        events[2]["when"] = 0
        with self.assertRaisesRegex(AssertionError, "before alpha"):
            completed(CONSOLE, events, BACKEND)

    def test_disconnect_is_not_a_successful_read(self):
        with self.assertRaisesRegex(AssertionError, "successful reply"):
            completed(CONSOLE, EVENTS, BACKEND[:4] + BACKEND[-1:])

    def test_boot_read_is_not_guest_dma(self):
        with self.assertRaisesRegex(AssertionError, "held guest read"):
            completed(CONSOLE, EVENTS, BACKEND[:3])

    def test_backend_error_or_wrong_request(self):
        for field, value in (("error", 5), ("cookie", 2), ("length", 1024)):
            backend = copy.deepcopy(BACKEND)
            backend[5][field] = value
            with self.subTest(field=field), self.assertRaises(AssertionError):
                completed(CONSOLE, EVENTS, backend)


if __name__ == "__main__":
    unittest.main()
