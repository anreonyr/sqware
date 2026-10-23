"""Parser negative controls: each case mutates otherwise valid runner evidence."""

import copy
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import daemon_stop


def valid_evidence():
    console = "\n".join(
        [
            "boot: entry task=10",
            "stop-root: root task=10",
            "stop-root: old task=17 team=21",
            "system: static construct name=fixturetarget task=17 team=21 owner=10",
            "system: static construct name=daemon task=18 team=22 owner=10",
            "system: static construct name=daemon-ctl-stop task=30 team=35 owner=10",
            "stop-root: actor task=29 identity=inherited",
            "stop-root: actor task=29 faces=ruin,state",
            "system: activate daemon tid=18",
            "daemon-ctl-stop: outsider source=30 observe=10 recover=10 denied=Denied",
            "daemon-ctl-stop: observed old task=17 helper=25 before=100",
            "stop-root: request controller=30 old task=17 helper=25 before=100 ack=44",
            "stop-root: kill old task=17 team=21 before=100",
            "stop-root: actor request queued",
            "system: ruin merged target=fixturetarget pending_origin=Reaper",
            "system: lifecycle received target=fixturetarget sender=29 action=ruin",
            "system: lifecycle intent target=fixturetarget run=false",
            "system: retired name=fixturetarget task=17 team=21",
            "daemon-stop-actor: target=fixturetarget reply=ok",
            "daemon-stop-actor: target=fixturetarget state=Dead",
            "stop-root: ack controller=30 run=false state=dead",
            "daemon-ctl-stop: ack from=10 old task=17 helper=25",
            "daemon-ctl-stop: stable dead checks=12 missing=12 target=fixturetarget elapsed_ns=650000000",
            "exit tid=30 reason=0x0 note: daemon-ctl-stop: done",
            "system: retired name=daemon task=18 team=22",
            "exit tid=10 reason=0x0 note: daemon-stop: done",
            "task: all tasks exited, system halted",
            "memory: frame_allocations before=87 after=87",
        ]
    )
    events = [
        {"h": 1, "when": 1000, "kind": {"room": {"spawn": {"tid": 17}}}},
        {"h": 1, "when": 1050, "kind": {"room": {"spawn": {"tid": 25}}}},
        {"h": 1, "when": 1100, "kind": {"room": {"spawn": {"tid": 29}}}},
        {"h": 2, "when": 1150, "kind": {"room": {"spawn": {"tid": 18}}}},
        {"h": 3, "when": 1200, "kind": {"room": {"spawn": {"tid": 30}}}},
        {"h": 1, "when": 3000, "kind": {"room": {"reap": {"tid": 17}}}},
        {"h": 1, "when": 3100, "kind": {"room": {"reap": {"tid": 25}}}},
        {"h": 1, "when": 4000, "kind": {"room": {"reap": {"tid": 29}}}},
        {"h": 3, "when": 4100, "kind": {"room": {"reap": {"tid": 30}}}},
        {"h": 2, "when": 4200, "kind": {"room": {"reap": {"tid": 18}}}},
        {"h": 0, "when": 4300, "kind": {"halt": "halt"}},
    ]
    qtest = "test embedded::tests::scene ... ok\ntest result: ok. 1 passed; 0 failed"
    return console, events, qtest, 0


class DaemonStopVerifier(unittest.TestCase):
    def verify(self, console, events, qtest, raw=0):
        return daemon_stop.verify(console, events, qtest, raw)

    def test_valid_evidence_passes(self):
        self.verify(*valid_evidence())

    def test_short_observation_window_is_rejected(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError, self.verify,
            console.replace("elapsed_ns=650000000", "elapsed_ns=100000000"),
            events, qtest, raw,
        )

    def test_outsider_faces_must_belong_to_actual_root(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError, self.verify,
            console.replace("observe=10 recover=10", "observe=99 recover=99"),
            events, qtest, raw,
        )

    def test_outsider_source_is_not_controller(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console.replace("outsider source=30", "outsider source=99"),
            events,
            qtest,
            raw,
        )

    def test_outsider_not_denied(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console.replace("denied=Denied", "denied=OK"),
            events,
            qtest,
            raw,
        )

    def test_merge_is_not_reaper_origin(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console.replace("pending_origin=Reaper", "pending_origin=External"),
            events,
            qtest,
            raw,
        )

    def test_merge_sender_is_not_actor(self):
        console, events, qtest, raw = valid_evidence()
        console = console.replace(
            "lifecycle received target=fixturetarget sender=29",
            "lifecycle received target=fixturetarget sender=99",
        )
        self.assertRaises(AssertionError, self.verify, console, events, qtest, raw)

    def test_intent_not_stopped(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console.replace("intent target=fixturetarget run=false", "intent target=fixturetarget run=true"),
            events,
            qtest,
            raw,
        )

    def test_missing_actor_reply(self):
        console, events, qtest, raw = valid_evidence()
        console = "\n".join(
            line for line in console.splitlines() if "daemon-stop-actor: target=fixturetarget reply=ok" not in line
        )
        self.assertRaises(AssertionError, self.verify, console, events, qtest, raw)

    def test_actor_state_is_not_dead(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console.replace("state=Dead", "state=Ready"),
            events,
            qtest,
            raw,
        )

    def test_missing_root_ack(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console.replace("stop-root: ack controller=30", "stop-root: ack controller=99"),
            events,
            qtest,
            raw,
        )

    def test_namespace_reappears(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console.replace("missing=12", "missing=11"),
            events,
            qtest,
            raw,
        )

    def test_too_few_stable_checks(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console.replace("checks=12 missing=12", "checks=1 missing=1"),
            events,
            qtest,
            raw,
        )

    def test_second_target_construction(self):
        console, events, qtest, raw = valid_evidence()
        extra = "system: static construct name=fixturetarget task=31 team=40 owner=10"
        self.assertRaises(
            AssertionError, self.verify, console + "\n" + extra, events, qtest, raw
        )

    def test_recovery_after_stop(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console + "\nsystem: recover admitted target=fixturetarget caller=18",
            events,
            qtest,
            raw,
        )

    def test_replacement_observed(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(
            AssertionError,
            self.verify,
            console + "\nstop-root: new task=31 team=40",
            events,
            qtest,
            raw,
        )

    def test_old_helper_never_reaped(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(AssertionError, self.verify, console, events[:6] + events[7:], qtest, raw)

    def test_daemon_dies_before_old_reap(self):
        console, events, qtest, raw = valid_evidence()
        events = copy.deepcopy(events)
        events[9]["when"] = 2500
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
                "\n".join(daemon_stop.json.dumps(event) for event in events[:-1])
                + '\n{"h":1'
            )
            self.assertRaises(AssertionError, daemon_stop.events_of, path)

    def test_missing_halt_record(self):
        console, events, qtest, raw = valid_evidence()
        self.assertRaises(AssertionError, self.verify, console, events[:-1], qtest, raw)


if __name__ == "__main__":
    unittest.main()
