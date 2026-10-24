"""Strict positive/negative checks for the reset-readback verifier."""

import copy
import json
import tempfile
import unittest
from pathlib import Path

from readback_check import check as readback
from fixtures.readback import (
    BLOCK_EVENTS, CONSOLE, DTS, EVENTS, FAILED_CONSOLE, FAILURE, PREFIX, READBACK,
    lines, proof,
)


def run(console=CONSOLE, failed=FAILED_CONSOLE, events=None, prefix=None,
        data=None, dts=DTS, block=None):
    events = EVENTS if events is None else events
    prefix = PREFIX if prefix is None else prefix
    data = READBACK if data is None else data
    block = BLOCK_EVENTS if block is None else block
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        console_path = root / "console.log"
        events_path = root / "events.jsonl"
        capture = root / "capture"
        console_path.write_text(console)
        events_path.write_text(lines(events))
        Path(str(capture) + ".failed-console").write_text(failed)
        Path(str(capture) + ".failed-diagnose.jsonl").write_text(lines(prefix))
        Path(str(capture) + ".readback.json").write_text(json.dumps(data))
        Path(str(capture) + ".dts").write_text(dts)
        Path(str(capture) + ".block-events").write_text(lines(block))
        readback(console_path, events_path, capture)


def op(data, request, reply=None):
    data = copy.deepcopy(data)
    for operation in data["operations"]:
        if operation["request"] == request:
            if reply is not None:
                operation["reply"] = reply
            break
    else:
        raise AssertionError(f"missing fixture operation: {request}")
    return data


class ReadbackEvidence(unittest.TestCase):
    def test_static_reaping_root_order_and_cull_activity_are_required(self):
        no_static = [event for event in copy.deepcopy(EVENTS)
                     if event.get("kind") != {"room": {"reap": {"tid": 7}}}]
        early_root = copy.deepcopy(EVENTS)
        for event in early_root:
            if event.get("kind") == {"room": {"reap": {"tid": 1}}}:
                event["when"] = 11
        for changed in (no_static, early_root):
            with self.subTest(events=changed), self.assertRaises(AssertionError):
                run(events=changed)
        with self.assertRaisesRegex(AssertionError, "culling"):
            run(console=CONSOLE.replace("doom: held=1 starved=1 blocked=4 nudged=0",
                                        "doom: held=0 starved=0 blocked=0 nudged=0"))

    def test_raw_status_and_queue_reads_must_follow_their_actual_writes(self):
        for before, after in ((13, 11), (16, 15), (14, 15), (17, 15)):
            data = copy.deepcopy(READBACK)
            item = data["operations"].pop(before)
            data["operations"].insert(after, item)
            with self.subTest(before=before, after=after), self.assertRaises(AssertionError):
                run(data=data)

    def test_complete_legitimate_evidence_is_accepted(self):
        self.assertIn("doom: held=1 starved=1 blocked=4 nudged=0", CONSOLE)
        run()

    def test_missing_stop_unavailable_or_changed_first_failure_is_rejected(self):
        variants = {
            "missing": FAILED_CONSOLE.replace(FAILURE + "\n", ""),
            "changed": FAILED_CONSOLE.replace(FAILURE, FAILURE[:-1] + "5"),
        }
        changed_full = CONSOLE.replace(
            "ctl: closed binding 1 failure party=4 op=2 code=4",
            "ctl: closed binding 1 failure party=5 op=2 code=4",
        )
        for name, failed in variants.items():
            with self.subTest(name=name), self.assertRaises(AssertionError):
                run(failed=failed)
        with self.assertRaises(AssertionError):
            run(console=changed_full)

    def test_failed_status_and_queue_must_be_the_actual_refusal(self):
        for name, mutate in {
            "status": lambda data: {**data, "failed_status": 0},
            "queue": lambda data: {**data, "failed_queue": 1},
        }.items():
            data = mutate(copy.deepcopy(READBACK))
            with self.subTest(name=name), self.assertRaises(AssertionError):
                run(data=data)

    def test_missing_owned_reap_or_early_cleanup_is_rejected(self):
        no_main = [event for event in copy.deepcopy(PREFIX)
                   if event.get("kind") != {"room": {"reap": {"tid": 9}}}]
        no_aux = [event for event in copy.deepcopy(PREFIX)
                  if event.get("kind") != {"room": {"reap": {"tid": 10}}}]
        no_hold = FAILED_CONSOLE.replace(
            "iommu: binding 1 invalidated frames held=6 (not released)\n", "")
        early = FAILED_CONSOLE + "recovery: binding 1 stopped reset=1 status=0 queue_enable=0\n"
        for name, kwargs in {
            "main": {"prefix": no_main},
            "aux": {"prefix": no_aux},
            "hold": {"failed": no_hold},
            "early": {"failed": early},
        }.items():
            with self.subTest(name=name), self.assertRaises(AssertionError):
                run(**kwargs)

    def test_failure_snapshot_must_be_this_run_s_prefix(self):
        prefix = copy.deepcopy(PREFIX)
        prefix[0]["when"] = 99
        with self.assertRaises(AssertionError):
            run(prefix=prefix)

    def test_non_status_write_forged_cookie_or_status_sequence_is_rejected(self):
        forged = copy.deepcopy(READBACK)
        forged["operations"].append({"request": "writel 0x400000018 0xdead", "reply": "OK"})
        bad_write = copy.deepcopy(READBACK)
        bad_write["operations"].insert(
            11, {"request": "writeb 0x400000014 0x2", "reply": "OK"})
        bad_read = op(READBACK, "readb 0x400000014", "OK 0x2")
        for name, data in {
            "forged": forged, "write": bad_write, "read": bad_read,
        }.items():
            with self.subTest(name=name), self.assertRaises(AssertionError):
                run(data=data)

    def test_wrong_physical_ecam_bar_or_capability_is_rejected(self):
        wrong_ecam = DTS.replace("0x30000000 0x0 0x10000000",
                                 "0x30001000 0x0 0x10000000")
        wrong_base = {**copy.deepcopy(READBACK), "base": 0x400002000}
        wrong_bar = op(READBACK, "readl 0x30008020", "OK 0x10")
        wrong_cap = op(READBACK, "readb 0x30008044", "OK 0x3")
        wrong_common = {**copy.deepcopy(READBACK), "common": 0x4000}
        for name, kwargs in {
            "ecam": {"dts": wrong_ecam},
            "base": {"data": wrong_base},
            "bar": {"data": wrong_bar},
            "cap": {"data": wrong_cap},
            "common": {"data": wrong_common},
        }.items():
            with self.subTest(name=name), self.assertRaises(AssertionError):
                run(**kwargs)

    def test_repair_must_read_back_real_zero_status_and_queue(self):
        for name, mutate in {
            "status": lambda data: {**data, "repaired_status": 1},
            "queue": lambda data: {**data, "repaired_queue": 1},
        }.items():
            data = mutate(copy.deepcopy(READBACK))
            with self.subTest(name=name), self.assertRaises(AssertionError):
                run(data=data)

    def test_malformed_capability_chain_or_guest_fault_is_rejected(self):
        loop = op(op(READBACK, "readb 0x30008043", "OK 0x5"),
                  "readb 0x30008041", "OK 0x40")
        fault = CONSOLE.replace(
            "task: all tasks exited, system halted",
            "[ERROR] user page fault killed: Load at VA(0x31002)\n"
            "task: all tasks exited, system halted")
        with self.subTest(name="loop"), self.assertRaises(AssertionError):
            run(data=loop)
        with self.subTest(name="fault"), self.assertRaises(AssertionError):
            run(console=fault)


if __name__ == "__main__":
    unittest.main()
