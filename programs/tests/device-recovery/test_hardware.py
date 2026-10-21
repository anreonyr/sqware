"""Require real queue fault, pending ownership and unmodified completion memory."""

import unittest

from hardware import regions
from hardware_check import hardware, pending
from fixtures.hardware import DTS, PENDING, REAPED, proof


class HardwareEvidence(unittest.TestCase):
    def test_success(self):
        hardware(proof(), DTS)
        pending(PENDING, REAPED)

    def test_device_tree_is_the_address_source(self):
        self.assertEqual(regions(DTS), (0x3010000, 0x90000000))
        with self.assertRaises(AssertionError):
            hardware(proof(), DTS.replace("0x3010000", "0x3020000"))
        with self.assertRaises(AssertionError):
            regions(DTS.replace('device_type = "memory"', 'device_type = "other"'))

    def test_no_actual_hardware_fault(self):
        data = proof()
        data["fault"] = 1
        with self.assertRaisesRegex(AssertionError, "memory fault"):
            hardware(data, DTS)

    def test_no_outstanding_commands(self):
        data = proof()
        data["head"] = data["tail"]
        with self.assertRaisesRegex(AssertionError, "outstanding"):
            hardware(data, DTS)

    def test_stale_or_missing_completion(self):
        for field, value in (("before", 0x10f00fef), ("after", 0)):
            data = proof()
            data[field] = value
            with self.subTest(field=field), self.assertRaises(AssertionError):
                hardware(data, DTS)

    def test_completion_memory_must_not_be_written_by_the_host(self):
        data = proof()
        data["operations"].append({
            "request": f"writel {data['destination']:#x} {data['cookie']:#x}", "reply": "OK",
        })
        with self.assertRaisesRegex(AssertionError, "forged"):
            hardware(data, DTS)

    def test_raw_command_read_must_match_the_record(self):
        data = proof()
        data["operations"][6]["reply"] = "OK 0x2"
        with self.assertRaises(AssertionError):
            hardware(data, DTS)

    def test_raw_completion_read_must_match_the_record(self):
        data = proof()
        data["operations"][-1]["reply"] = "OK 0x0"
        with self.assertRaises(AssertionError):
            hardware(data, DTS)

    def test_original_queue_must_be_restored(self):
        data = proof()
        data["operations"][10]["request"] = f"writeq {data['base'] + 0x18:#x} {data['broken']:#x}"
        with self.assertRaisesRegex(AssertionError, "repair"):
            hardware(data, DTS)

    def test_qtest_errors_are_not_success(self):
        data = proof()
        data["operations"][0]["reply"] = "FAIL"
        with self.assertRaisesRegex(AssertionError, "qtest"):
            hardware(data, DTS)

    def test_missing_alpha_member(self):
        with self.assertRaisesRegex(AssertionError, "completely reaped"):
            pending(PENDING, REAPED[:1])

    def test_every_resident_survives_the_failed_fence(self):
        for tid in (4, 6, 7, 8, 9):
            events = REAPED + [{"when": 3, "kind": {"room": {"reap": {"tid": tid}}}}]
            with self.subTest(tid=tid), self.assertRaises(AssertionError):
                pending(PENDING, events)

    def test_no_progress_before_confirmation(self):
        for line in (
            "iommu: binding 1 invalidated",
            "pci: binding 1 disabling",
            "recovery: binding 1 stopped",
            "iommu: binding 1 queues stopped",
            "iommu: binding 1 released",
            "hub: binding 1 closed",
        ):
            with self.subTest(line=line), self.assertRaisesRegex(AssertionError, "advanced"):
                pending(PENDING + line + "\n", REAPED)

    def test_real_expiration_is_required(self):
        line = "iommu: binding 1 invalidate fence expired frames held=6\n"
        with self.assertRaises(AssertionError):
            pending(PENDING.replace(line, ""), REAPED)


if __name__ == "__main__":
    unittest.main()
