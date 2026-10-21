"""Reject missing injection, early progress, lost ownership and incomplete recovery."""

import contextlib
import copy
import io
import unittest

from confirmation import completed
from fixtures.confirmation import evidence


class ConfirmationEvidence(unittest.TestCase):
    def test_all_three_boundaries(self):
        for step in ("invalidate", "reset", "release"):
            with self.subTest(step=step), contextlib.redirect_stdout(io.StringIO()):
                completed(*evidence(step), step)

    def test_no_injection_or_wrong_target_cannot_pass(self):
        console, events = evidence("release")
        for old, new in (("hits=5", "hits=0"), ("to=7 op=5", "to=8 op=5")):
            with self.subTest(old=old), self.assertRaises(AssertionError):
                completed(console.replace(old, new), events, "release")

    def test_kernel_reference_proof_is_required_at_both_boundaries(self):
        console, events = evidence("release")
        for at in (0, 1):
            parts = console.split("references=2")
            parts[at] += "references=1"
            changed = "".join(parts[:at + 1]) + "references=2".join(parts[at + 1:])
            with self.subTest(at=at), self.assertRaises(AssertionError):
                completed(changed, events, "release")

    def test_failure_cannot_be_hidden_or_relabelled(self):
        console, events = evidence("reset")
        with self.assertRaises(AssertionError):
            completed(console.replace("code=0 state=Closing", "code=1 state=Closed"), events, "reset")
        with self.assertRaises(AssertionError):
            completed(console.replace("ctl: confirmation retry settled; original failure preserved\n", ""), events, "reset")

    def test_advance_before_confirmation_is_rejected(self):
        console, events = evidence("release")
        line = next(line for line in console.splitlines(True) if line.startswith("iommu: binding 1 released"))
        with self.assertRaisesRegex(AssertionError, "without confirmation"):
            completed(line + console.replace(line, ""), events, "release")

    def test_each_resident_must_survive_the_failure_interval(self):
        console, events = evidence("release")
        for index in range(2, 7):
            changed = copy.deepcopy(events)
            changed[index]["when"] = 4
            with self.subTest(index=index), self.assertRaisesRegex(AssertionError, "while confirmation"):
                completed(console, changed, "release")

    def test_alpha_team_must_already_be_reaped(self):
        console, events = evidence("invalidate")
        events[1]["when"] = 4
        with self.assertRaisesRegex(AssertionError, "alpha Team"):
            completed(console, events, "invalidate")

    def test_leak_or_truncated_export_is_rejected(self):
        console, events = evidence("release")
        with self.assertRaisesRegex(AssertionError, "leaked"):
            completed(console.replace("after=85", "after=86"), events, "release")
        with self.assertRaisesRegex(AssertionError, "incomplete trace"):
            completed(console, events[:-1], "release")

    def test_native_fault_is_not_accepted_as_cleanup(self):
        console, events = evidence("release")
        with self.assertRaisesRegex(AssertionError, "native fault"):
            completed(console + "user fault killed: tid=10\n", events, "release")


if __name__ == "__main__":
    unittest.main()
