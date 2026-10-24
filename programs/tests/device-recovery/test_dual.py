"""Strict positive/negative checks for the dual-device verifier."""

import copy
import tempfile
import unittest
from pathlib import Path

from dual_check import check as dual
from fixtures.dual import CONSOLE, EVENTS, lines

B_READ = "dual: read serial=2 pattern=165 round=1 used=2 len=513 sector_ok=true status=0\n"


def run(console=CONSOLE, events=None, generations=1):
    with tempfile.TemporaryDirectory() as tmp:
        console_path = Path(tmp) / "console.log"
        events_path = Path(tmp) / "events.jsonl"
        console_path.write_text(console)
        events_path.write_text(lines(EVENTS if events is None else events))
        dual(console_path, events_path, generations)


class DualEvidence(unittest.TestCase):
    def test_first_generation_capability_leak_is_rejected(self):
        with self.assertRaisesRegex(AssertionError, "capability count"):
            run(CONSOLE.replace("capabilities=17", "capabilities=18"))

    def test_complete_legitimate_evidence_is_accepted_with_cumulative_doom(self):
        self.assertIn("doom: held=1 starved=0 blocked=5 nudged=0", CONSOLE)
        run()

    def test_one_device_or_one_domain_is_not_dual_evidence(self):
        variants = {
            "device": CONSOLE.replace(
                "recovery: ready hub=4 pci=6 requester=16 key=2\n", ""),
            "domain": CONSOLE.replace(
                "iommu: domain 2 established owner=7 requester=16\n", ""),
        }
        for name, console in variants.items():
            with self.subTest(name=name), self.assertRaises(AssertionError):
                run(console)

    def test_unequal_iova_is_rejected(self):
        console = CONSOLE.replace(
            "iommu: domain 2 binding 2 owner=7 requester=16 iova=0x200000000",
            "iommu: domain 2 binding 2 owner=7 requester=16 iova=0x200100000",
        )
        with self.assertRaises(AssertionError):
            run(console)

    def test_overlapping_bar_or_missing_a_data_preservation_is_rejected(self):
        variants = {
            "bar": CONSOLE.replace(
                "pci: endpoint requester=16 bar=4 address=0x400004000",
                "pci: endpoint requester=16 bar=4 address=0x400002000"),
            "preserved": CONSOLE.replace(
                "dual: isolated a=1 b=2 a_preserved=true",
                "dual: isolated a=1 b=2 a_preserved=false"),
        }
        for name, console in variants.items():
            with self.subTest(name=name), self.assertRaises(AssertionError):
                run(console)

    def test_post_close_b_read_must_exist_and_follow_a_close(self):
        missing = CONSOLE.replace(B_READ, "")
        misordered = CONSOLE.replace(B_READ, "").replace(
            "dual: isolated a=1 b=2 a_preserved=true\n",
            "dual: isolated a=1 b=2 a_preserved=true\n" + B_READ,
        )
        for name, console in {"missing": missing, "misordered": misordered}.items():
            with self.subTest(name=name), self.assertRaises(AssertionError):
                run(console)

    def test_work_exit_and_actual_stop_proof_are_required(self):
        variants = {
            "work-mode": CONSOLE.replace(
                "work: binding 1 requesting Close::Normal",
                "work: binding 1 exiting without Close"),
            "b-work-mode": CONSOLE.replace(
                "work: binding 2 exiting without Close",
                "work: binding 2 requesting Close::Normal"),
            "no-stop": CONSOLE.replace(
                "recovery: binding 1 stopped reset=1 status=0 queue_enable=0\n", ""),
            "engine-not-shared": CONSOLE.replace(
                "iommu: binding 1 released shared ddtp=0x23fd6002 cqcsr=0x10001 "
                "fqcsr=0x10001 pqcsr=0x0 queue_enable=1 frames=0 engine_off=false",
                "iommu: binding 1 released shared ddtp=0x23fd6002 cqcsr=0x10001 "
                "fqcsr=0x10001 pqcsr=0x0 queue_enable=1 frames=0 engine_off=true"),
        }
        for name, console in variants.items():
            with self.subTest(name=name), self.assertRaises(AssertionError):
                run(console)

    def test_foreign_key_or_domain_owner_is_rejected(self):
        variants = {
            "key": CONSOLE.replace(
                "recovery: ready hub=4 pci=6 requester=16 key=2",
                "recovery: ready hub=4 pci=6 requester=16 key=9"),
            "owner": CONSOLE.replace(
                "iommu: domain 2 established owner=7 requester=16",
                "iommu: domain 2 established owner=8 requester=16"),
        }
        for name, console in variants.items():
            with self.subTest(name=name), self.assertRaises(AssertionError):
                run(console)

    def test_missing_work_or_auxiliary_reap_and_root_ordering_are_rejected(self):
        no_aux = copy.deepcopy(EVENTS)
        no_aux = [event for event in no_aux
                  if event.get("kind") != {"room": {"reap": {"tid": 12}}}]
        early_root = copy.deepcopy(EVENTS)
        for event in early_root:
            if event.get("kind") == {"room": {"reap": {"tid": 1}}}:
                event["when"] = 9
        no_service = [event for event in copy.deepcopy(EVENTS)
                      if event.get("kind") != {"room": {"reap": {"tid": 5}}}]
        for name, events in {
            "aux": no_aux, "service": no_service, "root": early_root,
        }.items():
            with self.subTest(name=name), self.assertRaises(AssertionError):
                run(events=events)

    def test_changed_baseline_guest_fault_or_live_doom_gauge_is_rejected(self):
        variants = {
            "frames": CONSOLE.replace(
                "memory: frame_allocations before=85 after=85",
                "memory: frame_allocations before=85 after=86"),
            "fault": CONSOLE.replace(
                "task: all tasks exited, system halted",
                "user fault killed: Load at VA(0x31002)\n"
                "task: all tasks exited, system halted"),
            "doom": CONSOLE.replace(
                "doom: held=1 starved=0 blocked=5 nudged=0",
                "doom: held=0 starved=0 blocked=0 nudged=0"),
        }
        for name, console in variants.items():
            with self.subTest(name=name), self.assertRaises(AssertionError):
                run(console)


if __name__ == "__main__":
    unittest.main()
