"""Reject missing injection, early progress, lost ownership and incomplete recovery."""

import contextlib
import copy
import io
import unittest

from confirmation import completed
from fixtures.confirmation import evidence


class ConfirmationEvidence(unittest.TestCase):
    def test_all_three_boundaries(self):
        for step in ("invalidate", "reset", "release", "reference"):
            with self.subTest(step=step), contextlib.redirect_stdout(io.StringIO()):
                completed(*evidence(step), step)

    def test_no_injection_or_wrong_target_cannot_pass(self):
        console, events = evidence("release")
        for old, new in (("hits=5", "hits=0"), ("to=7 op=5", "to=8 op=5")):
            with self.subTest(old=old), self.assertRaises(AssertionError):
                completed(console.replace(old, new), events, "release")

    def test_stop_gate_requires_the_actual_device_driver_and_pci_trigger(self):
        console, events = evidence("reset")
        armed = "confirmation: device stop gate armed from=8 pci=6 serial=1\n"
        rejected = "confirmation: transport rejected from=8 to=6 op=3 serial=1 references=2 ticks=3\n"
        variants = (
            console.replace(armed, ""),
            console.replace("gate armed from=8", "gate armed from=4"),
            console.replace("gate armed from=8 pci=6", "gate armed from=8 pci=7"),
            console.replace(rejected, ""),
            console.replace("transport rejected from=8 to=6", "transport rejected from=4 to=6"),
            console.replace("transport rejected from=8 to=6", "transport rejected from=8 to=7"),
            console.replace("op=3 serial=1 references=2", "op=2 serial=1 references=2"),
            console.replace("op=3 serial=1 references=2", "op=3 serial=2 references=2"),
            console.replace("serial=1 references=2", "serial=1 references=1", 1),
        )
        for changed in variants:
            with self.subTest(console=changed), self.assertRaises(AssertionError):
                completed(changed, events, "reset")

        failure = "recovery: binding 1 stage Disable failed party=2 op=3 code=0\n"
        controller = "ctl: confirmation failed party=2 op=3 code=0 state=Closing\n"
        removed = "confirmation: injection removed hits=5 references=2 ticks=5\n"
        early_disable = console.replace(
            "confirmation: device stop gate armed from=8 pci=6 serial=1\n",
            "pci: binding 1 disabling bus-master\n"
            "confirmation: device stop gate armed from=8 pci=6 serial=1\n",
        )
        early_stop = console.replace(removed + "pci: binding 1 disabling bus-master\n"
            "pci: endpoint bus-master off=0x0\n"
            "recovery: binding 1 stopped reset=1 status=0 queue_enable=0\n",
            removed + "pci: binding 1 disabling bus-master\n"
            "pci: endpoint bus-master off=0x0\n",
        ).replace(
            rejected,
            "recovery: binding 1 stopped reset=1 status=0 queue_enable=0\n" + rejected,
        )
        reordered = console.replace(armed + rejected, rejected + armed)
        order_variants = (
            console.replace(failure, ""),
            console.replace("stage Disable failed party=2", "stage Disable failed party=3"),
            console.replace("stage Disable failed party=2 op=3", "stage Disable failed party=2 op=5"),
            console.replace("party=2 op=3 code=0", "party=2 op=3 code=1", 1),
            console.replace(controller, ""),
            console.replace("state=Closing", "state=Closed"),
            console.replace(removed, ""),
            early_disable,
            early_stop,
            reordered,
        )
        for changed in order_variants:
            with self.subTest(console=changed), self.assertRaises(AssertionError):
                completed(changed, events, "reset")

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

    def test_reference_failure_requires_the_same_owned_handle(self):
        console, events = evidence("reference")
        with self.assertRaises(AssertionError):
            completed(
                console.replace("still owned reference=777", "still owned reference=778"),
                events, "reference",
            )
        with self.assertRaises(AssertionError):
            completed(console.replace("reference=777 pages=6", "reference=0 pages=6"), events, "reference")
        with self.assertRaises(AssertionError):
            completed(console.replace("reference=777 references=1", "reference=778 references=1"), events, "reference")

    def test_reference_failure_must_be_a_driver_error_not_transport_error(self):
        console, events = evidence("reference")
        with self.assertRaises(AssertionError):
            completed(console.replace("code=4", "code=0"), events, "reference")


if __name__ == "__main__":
    unittest.main()
