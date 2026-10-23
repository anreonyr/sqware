"""Parser negative controls: each case mutates otherwise valid runner evidence."""

import copy
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import daemon_target


def valid_evidence():
    console = "\n".join(
        [
            "boot: entry task=10",
            "target-root: root task=10",
            "target-root: old task=17 team=21",
            "system: static construct name=fixturetarget task=17 team=21 owner=10",
            "system: static construct name=daemon task=18 team=22 owner=10",
            "system: static construct name=daemon-ctl-target task=30 team=35 owner=10",
            "system: activate daemon tid=18",
            "daemon-target: observed old task=17 helper=25 before=100",
            "target-root: request controller=30 old task=17 helper=25 before=100",
            "target-root: kill old task=17 team=21 before=100",
            "system: retired name=fixturetarget task=17 team=21",
            "system: recover admitted target=fixturetarget caller=18",
            "target-root: retired old task=17 team=21",
            "system: static construct name=fixturetarget task=31 team=40 owner=10",
            "target-root: new task=31 team=40",
            "daemon-target: proof old task=17 helper=25 before=100 new task=31 helper=45 after=120",
            "exit tid=30 reason=0x0 note: daemon-ctl-target: done",
            "system: retired name=daemon task=18 team=22",
            "system: retired name=fixturetarget task=31 team=40",
            "exit tid=10 reason=0x0 note: daemon-target: done",
            "task: all tasks exited, system halted",
            "memory: frame_allocations before=87 after=87",
        ]
    )
    events = [
        {"h": 1, "when": 1000, "kind": {"room": {"spawn": {"tid": 17}}}},
        {"h": 1, "when": 1050, "kind": {"room": {"spawn": {"tid": 25}}}},
        {"h": 2, "when": 1100, "kind": {"room": {"spawn": {"tid": 18}}}},
        {"h": 3, "when": 1200, "kind": {"room": {"spawn": {"tid": 30}}}},
        {"h": 1, "when": 2000, "kind": {"room": {"reap": {"tid": 17}}}},
        {"h": 1, "when": 2100, "kind": {"room": {"reap": {"tid": 25}}}},
        {"h": 1, "when": 3000, "kind": {"room": {"spawn": {"tid": 31}}}},
        {"h": 1, "when": 3050, "kind": {"room": {"spawn": {"tid": 45}}}},
        {"h": 3, "when": 3900, "kind": {"room": {"reap": {"tid": 30}}}},
        {"h": 2, "when": 3800, "kind": {"room": {"reap": {"tid": 18}}}},
        {"h": 1, "when": 4000, "kind": {"room": {"reap": {"tid": 31}}}},
        {"h": 1, "when": 4050, "kind": {"room": {"reap": {"tid": 45}}}},
        {"h": 0, "when": 4100, "kind": {"halt": "halt"}},
    ]
    qtest = "test embedded::tests::scene ... ok\ntest result: ok. 1 passed; 0 failed"
    return console, events, qtest, 0


class DaemonTargetVerifier(unittest.TestCase):
    def verify(self, console, events, qtest, raw=0):
        return daemon_target.verify(console, events, qtest, raw)

    def test_valid_evidence_passes(self):
        self.verify(*valid_evidence())

    def test_both_old_members_retire_before_replacement_in_either_order(self):
        console, events, qtest, raw = valid_evidence()
        observed = daemon_target.one(
            console, r"^daemon-target: observed old task=(\d+) helper=(\d+) before=(\d+)$"
        )
        old_task, old_helper = int(observed[1]), int(observed[2])
        old_reap_time = daemon_target.room(events, "reap", old_task)[1]
        changed = 0
        for event in events:
            if event.get("kind", {}).get("room", {}).get("reap", {}).get("tid") == old_helper:
                event["when"] = old_reap_time - 1
                changed += 1
        self.assertEqual(changed, 1)
        events.sort(key=lambda event: event["when"])
        self.verify(console, events, qtest, raw)

    def test_wrong_recover_caller(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console.replace("caller=18", "caller=99"),
            events,
            qtest,
            raw,
        )

    def test_missing_daemon_activation(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console.replace("system: activate daemon tid=18\n", ""),
            events,
            qtest,
            raw,
        )

    def test_duplicate_daemon_activation(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console + "\nsystem: activate daemon tid=18",
            events,
            qtest,
            raw,
        )

    def test_stale_heartbeat_task(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console.replace("observed old task=17", "observed old task=99"),
            events,
            qtest,
            raw,
        )

    def test_request_helper_mismatch(self):
        console, events, qtest, raw = valid_evidence()
        console = console.replace(
            "target-root: request controller=30 old task=17 helper=25",
            "target-root: request controller=30 old task=17 helper=99",
        )
        self.assertRaises(AssertionError, self.verify, console, events, qtest, raw)

    def test_old_helper_is_old_main(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console.replace("observed old task=17 helper=25", "observed old task=17 helper=17"),
            events,
            qtest,
            raw,
        )

    def test_retirement_after_new_construction(self):
        console, events, qtest, raw = valid_evidence()
        retired = "system: retired name=fixturetarget task=17 team=21"
        moved = console.replace(retired + "\n", "")
        marker = "system: static construct name=fixturetarget task=31 team=40 owner=10"
        moved = moved.replace(marker, marker + "\n" + retired)
        self.assertRaises(AssertionError, self.verify, moved, events, qtest, raw)

    def test_admission_before_finalized_retirement(self):
        console, events, qtest, raw = valid_evidence()
        retired = "system: retired name=fixturetarget task=17 team=21"
        admitted = "system: recover admitted target=fixturetarget caller=18"
        moved = console.replace(retired + "\n" + admitted, admitted + "\n" + retired)
        self.assertRaises(AssertionError, self.verify, moved, events, qtest, raw)

    def test_old_target_reap_after_new_spawn(self):
        console, events, qtest, raw = valid_evidence()
        events = copy.deepcopy(events)
        events[4]["when"] = 3500
        self.assertRaises(AssertionError, self.verify, console, events, qtest, raw)

    def test_old_helper_reap_after_new_spawn(self):
        console, events, qtest, raw = valid_evidence()
        events = copy.deepcopy(events)
        events[5]["when"] = 3500
        self.assertRaises(AssertionError, self.verify, console, events, qtest, raw)

    def test_missing_old_helper_reap(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(AssertionError, self.verify, console, events[:5] + events[6:], qtest, raw)

    def test_missing_new_helper_spawn(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(AssertionError, self.verify, console, events[:7] + events[8:], qtest, raw)

    def test_proof_reuses_old_helper(self):
        console, events, qtest, raw = valid_evidence()
        console = console.replace("new task=31 helper=45", "new task=31 helper=25")
        self.assertRaises(AssertionError, self.verify, console, events, qtest, raw)

    def test_fresh_helper_is_new_main(self):
        console, events, qtest, raw = valid_evidence()
        console = console.replace("new task=31 helper=45", "new task=31 helper=31")
        self.assertRaises(AssertionError, self.verify, console, events, qtest, raw)

    def test_root_kills_helper(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console + "\ntarget-root: kill old helper=25",
            events,
            qtest,
            raw,
        )

    def test_replacement_keeps_old_team(self):
        console, events, qtest, raw = valid_evidence()
        console = console.replace("new task=31 team=40", "new task=31 team=21")
        self.assertRaises(AssertionError, self.verify, console, events, qtest, raw)

    def test_missing_proof(self):
        console, events, qtest, raw = valid_evidence()
        console = "\n".join(
            line for line in console.splitlines() if not line.startswith("daemon-target: proof")
        )
        self.assertRaises(AssertionError, self.verify, console, events, qtest, raw)

    def test_wrong_construction_owner(self):
        console, events, qtest, raw = valid_evidence()
        console = console.replace(
            "name=fixturetarget task=31 team=40 owner=10",
            "name=fixturetarget task=31 team=40 owner=99",
        )
        self.assertRaises(AssertionError, self.verify, console, events, qtest, raw)

    def test_missing_controller_exit(self):
        console, events, qtest, raw = valid_evidence()
        console = "\n".join(
            line for line in console.splitlines() if "daemon-ctl-target: done" not in line
        )
        self.assertRaises(AssertionError, self.verify, console, events, qtest, raw)

    def test_frame_regression(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console.replace("after=87", "after=88"),
            events,
            qtest,
            raw,
        )

    def test_panic(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console + "\n[panic] at kernel/src/entry/mod.rs:1",
            events,
            qtest,
            raw,
        )

    def test_timeout_and_nonzero_raw_exit(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console,
            events,
            qtest.replace("... ok", "... FAILED") + "\ntimed out after 20s",
            101,
        )

    def test_empty_evidence(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(AssertionError, self.verify, console, [], qtest, raw)

    def test_truncated_diagnose_line(self):
        console, events, qtest, raw = valid_evidence()
        with tempfile.TemporaryDirectory() as at:
            path = Path(at) / "truncated.jsonl"
            path.write_text(
                "\n".join(daemon_target.json.dumps(event) for event in events[:-1])
                + '\n{"h":1'
            )
            self.assertRaises(AssertionError, daemon_target.events_of, path)

    def test_missing_halt_record(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(AssertionError, self.verify, console, events[:-1], qtest, raw)

    def test_missing_actual_daemon_birth(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(AssertionError, self.verify, console, events[:2] + events[3:], qtest, raw)

    def test_daemon_dies_before_replacement_spawn(self):
        console, events, qtest, raw = valid_evidence()
        events = copy.deepcopy(events)
        events[9]["when"] = 2900
        self.assertRaises(AssertionError, self.verify, console, events, qtest, raw)

    def test_provider_closes_before_daemon(self):
        console, events, qtest, raw = valid_evidence()
        daemon = "system: retired name=daemon task=18 team=22"
        target = "system: retired name=fixturetarget task=31 team=40"
        console = console.replace(daemon + "\n" + target, target + "\n" + daemon)
        self.assertRaises(AssertionError, self.verify, console, events, qtest, raw)

    def test_controller_reap_missing(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(AssertionError, self.verify, console, events[:8] + events[9:], qtest, raw)


if __name__ == "__main__":
    unittest.main()
