"""Negative controls for the deployed guard-death verifier."""

import copy
import unittest

from guard_check import verify
from fixtures.guard import BLOCK, CONSOLE, EVENTS, FEED, HELD_CONSOLE, HELD_EVENTS, QT, READY, TRIGGER


def check(console=CONSOLE, held=HELD_CONSOLE, events=EVENTS, held_events=HELD_EVENTS,
          block=BLOCK, feed=FEED, target="pci", harts=4, code=101, qtest=QT):
    verify(console, held, events, held_events, block, feed, target, harts, code, qtest)


class GuardDeathEvidence(unittest.TestCase):
    def test_success(self):
        check()

    def test_missing_held_read(self):
        with self.assertRaisesRegex(AssertionError, "one held"):
            check(held_events=HELD_EVENTS[:-1])

    def test_wrong_target(self):
        with self.assertRaises(AssertionError):
            check(target="iommu")

    def test_no_peer_quiescence(self):
        with self.assertRaises(AssertionError):
            check(console=CONSOLE.replace("[critical] peer harts quiesced=3\n", ""))

    def test_wrong_hart_count(self):
        with self.assertRaises(AssertionError):
            check(harts=8)

    def test_target_reference_cleanup(self):
        with self.assertRaisesRegex(AssertionError, "reference cleanup"):
            check(console=CONSOLE + "[critical] reference cleanup task=7\n")

    def test_normal_return_is_not_a_guard_death(self):
        for code, qtest in ((0, QT), (101, "test embedded::tests::scene ... ok\n")):
            with self.subTest(code=code), self.assertRaises(AssertionError):
                check(code=code, qtest=qtest)

    def test_missing_kernel_trip(self):
        with self.assertRaises(AssertionError):
            check(console=CONSOLE.replace(
                "[critical] trip task=7 task_team=3 team=3\n", ""))

    def test_trigger_before_ready(self):
        out = CONSOLE.replace(READY, "").replace(TRIGGER, TRIGGER + READY)
        with self.assertRaisesRegex(AssertionError, "out of order|preceded ready"):
            check(console=out)

    def test_kernel_identity_mismatch(self):
        with self.assertRaisesRegex(AssertionError, "kernel trip task"):
            check(console=CONSOLE.replace(
                "[critical] trip task=7 task_team=3 team=3",
                "[critical] trip task=8 task_team=3 team=3"))

    def test_protected_team_mismatch(self):
        with self.assertRaisesRegex(AssertionError, "protected team"):
            check(console=CONSOLE.replace(
                "[critical] trip task=7 task_team=3 team=3",
                "[critical] trip task=7 task_team=4 team=4"))

    def test_pretrigger_release_or_invalidation(self):
        console = CONSOLE.replace(
            TRIGGER, "iommu: binding 1 invalidated frames held=6\n" + TRIGGER)
        with self.assertRaisesRegex(AssertionError, "already advanced"):
            check(console=console)

    def test_resume_or_completion_is_not_retention(self):
        for phase in ("resuming", "replied"):
            held = copy.deepcopy(HELD_EVENTS) + [{"phase": phase, "cookie": 1}]
            with self.subTest(phase=phase), self.assertRaisesRegex(AssertionError, "resumed|completed"):
                check(held_events=held)

    def test_feed_must_record_the_trigger(self):
        for field in ("sent_g", "held_before_trigger", "trigger", "serial_eof"):
            feed = copy.deepcopy(FEED)
            feed[field] = False
            with self.subTest(field=field), self.assertRaises(AssertionError):
                check(feed=feed)


if __name__ == "__main__":
    unittest.main()
