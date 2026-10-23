"""Strict independent verifier for the explicit-stop race scene.

The console and structured trace are runner evidence. Negative controls mutate
otherwise-valid evidence; they do not mirror runtime branches.
"""

import json
import re
import sys
from pathlib import Path


def one(text, pattern):
    matches = list(re.finditer(pattern, text, re.MULTILINE))
    assert len(matches) == 1, f"expected one proof: {pattern}"
    return matches[0]


def all_of(text, pattern):
    return list(re.finditer(pattern, text, re.MULTILINE))


def events_of(path):
    raw = Path(path).read_text()
    records = []
    for number, line in enumerate(raw.splitlines(), 1):
        if not line.strip():
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError as error:
            raise AssertionError(f"diagnose line {number} is truncated: {error}") from error
        assert isinstance(record, dict), f"diagnose line {number} is not an object"
        records.append(record)
    assert records, "diagnose trace is empty"
    assert records[-1].get("kind") == {"halt": "halt"}, "diagnose trace did not halt"
    return records


def room(records, operation, task):
    found = []
    for index, record in enumerate(records):
        value = record.get("kind", {}).get("room", {}).get(operation)
        if isinstance(value, dict) and value.get("tid") == task:
            found.append((index, record.get("when")))
    assert len(found) == 1, f"expected one room.{operation} for task {task}"
    return found[0]


def verify(console, records, qtest, raw_code):
    assert int(raw_code) == 0, "raw qtest exit must be zero"
    assert records, "diagnose trace is empty"
    assert records[-1].get("kind") == {"halt": "halt"}, "diagnose trace did not halt"
    assert "timed out" not in qtest, "qtest timed out"
    assert "FAILED" not in qtest, "qtest scene failed"
    one(qtest, r"^test embedded::tests::scene \.\.\. ok$")
    one(qtest, r"^test result: ok\. 1 passed; 0 failed")
    assert "[panic]" not in console, "console contains a kernel panic"
    assert "test panic" not in console, "console contains a test panic"

    root = int(one(console, r"^boot: entry task=([1-9][0-9]*)$")[1])
    recorded_root = int(one(console, r"^stop-root: root task=([1-9][0-9]*)$")[1])
    assert recorded_root == root, "Root harness identity differs from boot entry"
    exit_root = one(
        console, r"^exit tid=([1-9][0-9]*) reason=0x0 note: daemon-stop: done$"
    )
    assert int(exit_root[1]) == root, "Root did not exit successfully as the boot entry"

    daemon = int(one(console, r"^system: activate daemon tid=([1-9][0-9]*)$")[1])
    daemon_construct = one(
        console, r"^system: static construct name=daemon task=(\d+) team=(\d+) owner=(\d+)$"
    )
    assert int(daemon_construct[1]) == daemon, "daemon activation identity differs from construction"
    assert int(daemon_construct[3]) == root, "daemon construction owner is not Root"

    old = one(console, r"^stop-root: old task=(\d+) team=(\d+)$")
    old_task, old_team = int(old[1]), int(old[2])
    observed = one(
        console, r"^daemon-ctl-stop: observed old task=(\d+) helper=(\d+) before=(\d+)$"
    )
    old_helper = int(observed[2])
    before = int(observed[3])
    assert int(observed[1]) == old_task, "controller observed a different old task"
    assert old_helper not in (0, old_task), "old helper is not a distinct live task"
    assert before > 0, "controller did not obtain a real clock value"

    actor = int(one(console, r"^stop-root: actor task=(\d+) identity=inherited$")[1])
    one(console, rf"^stop-root: actor task={actor} faces=ruin,state$")
    controller = int(
        one(
            console,
            r"^system: static construct name=daemon-ctl-stop task=(\d+) team=(\d+) owner=(\d+)$",
        )[1]
    )
    controller_owner = int(
        one(
            console,
            r"^system: static construct name=daemon-ctl-stop task=(\d+) team=(\d+) owner=(\d+)$",
        )[3]
    )
    assert controller_owner == root, "controller construction owner is not Root"

    outsider = one(
        console,
        r"^daemon-ctl-stop: outsider source=(\d+) observe=(\d+) recover=(\d+) denied=Denied$",
    )
    assert int(outsider[1]) == controller, "outsider source is not the deployed controller"
    assert int(outsider[2]) == root and int(outsider[3]) == root, (
        "outsider face hosts are not actual Control hosts"
    )

    publisher = one(
        console,
        r"^daemon-ctl-stop: publisher task=(\d+) host=(\d+) entry_owner=(\d+) entry_mark=(\d+) expected_mark=(\d+)$",
    )
    assert (int(publisher[1]), int(publisher[2]), int(publisher[3])) == (old_task, root, old_task), (
        "publisher query or entry owner differs from the actual target and Control"
    )
    assert int(publisher[4]) == int(publisher[5]) != 0, "publisher entry mark differs from declaration"

    request = one(
        console,
        r"^stop-root: request controller=(\d+) old task=(\d+) helper=(\d+) before=(\d+) ack=(\d+)$",
    )
    assert int(request[1]) == controller, "request did not come from the deployed controller"
    assert int(request[2]) == old_task, "request old task differs from the actual target"
    assert int(request[3]) == old_helper, "request helper differs from the real heartbeat"
    assert int(request[4]) == before, "request time differs from the controller observation"

    kill = one(console, r"^stop-root: kill old task=(\d+) team=(\d+) before=(\d+)$")
    assert (int(kill[1]), int(kill[2]), int(kill[3])) == (old_task, old_team, before), (
        "kill did not target the actual old target main"
    )
    assert not all_of(console, r"^stop-root: kill old helper="), (
        "Root must kill only the old main; Control retires the whole old Team"
    )

    queued = one(console, r"^stop-root: actor request queued$")
    merged = one(
        console,
        r"^system: ruin merged target=fixturetarget pending_origin=Reaper$",
    )
    received = one(
        console,
        r"^system: lifecycle received target=fixturetarget sender=(\d+) action=ruin$",
    )
    assert int(received[1]) == actor, "merged Ruin sender is not the authorized Root actor"
    intent = one(console, r"^system: lifecycle intent target=fixturetarget run=false$")
    retired = one(console, rf"^system: retired name=fixturetarget task={old_task} team={old_team}$")
    reply = one(console, r"^daemon-stop-actor: target=fixturetarget reply=ok$")
    state = one(console, r"^daemon-stop-actor: target=fixturetarget state=Dead$")
    ack = one(console, r"^stop-root: ack controller=(\d+) run=false state=dead$")
    assert int(ack[1]) == controller, "Root acknowledged another controller"
    controller_ack = one(
        console, rf"^daemon-ctl-stop: ack from={root} old task={old_task} helper={old_helper}$"
    )
    stable = one(
        console,
        r"^daemon-ctl-stop: stable dead checks=(\d+) missing=(\d+) target=fixturetarget elapsed_ns=(\d+)$",
    )
    checks, missing = int(stable[1]), int(stable[2])
    assert checks >= 2, "controller did not poll several daemon intervals"
    assert int(stable[3]) >= 600_000_000, "post-stop observation window is too short"
    assert missing == checks, "target namespace entry reappeared after stop"
    revoked = one(
        console, r"^daemon-ctl-stop: publisher revoked checks=(\d+) host=(\d+) target=(\d+)$"
    )
    assert (int(revoked[1]), int(revoked[2]), int(revoked[3])) == (checks, root, old_task), (
        "publisher authority was not revoked throughout the observation window"
    )
    exit_controller = one(
        console, r"^exit tid=(\d+) reason=0x0 note: daemon-ctl-stop: done$"
    )
    assert int(exit_controller[1]) == controller, "controller did not exit successfully"

    constructs = all_of(
        console,
        r"^system: static construct name=fixturetarget task=(\d+) team=(\d+) owner=(\d+)$",
    )
    assert len(constructs) == 1, "explicit stop constructed a replacement target"
    assert (int(constructs[0][1]), int(constructs[0][2])) == (old_task, old_team), (
        "the only target construction is not the stopped target"
    )
    assert int(constructs[0][3]) == root, "target construction owner is not Root"
    assert not all_of(console, r"^system: recover admitted"), "a Recover was admitted after stop"
    assert not all_of(console, r"^stop-root: new task="), "Root observed a replacement target"

    daemon_retirement = one(
        console,
        rf"^system: retired name=daemon task={daemon} team={int(daemon_construct[2])}$",
    )
    ordered = [
        old.start(),
        outsider.start(),
        observed.start(),
        publisher.start(),
        request.start(),
        kill.start(),
        queued.start(),
        merged.start(),
        received.start(),
        intent.start(),
        retired.start(),
        reply.start(),
        state.start(),
        ack.start(),
        controller_ack.start(),
        stable.start(),
        revoked.start(),
        exit_controller.start(),
        daemon_retirement.start(),
        exit_root.start(),
    ]
    assert ordered == sorted(ordered), "explicit-stop proof is out of order"

    old_spawn = room(records, "spawn", old_task)
    old_helper_spawn = room(records, "spawn", old_helper)
    old_reap = room(records, "reap", old_task)
    old_helper_reap = room(records, "reap", old_helper)
    actor_spawn = room(records, "spawn", actor)
    actor_reap = room(records, "reap", actor)
    daemon_spawn = room(records, "spawn", daemon)
    daemon_reap = room(records, "reap", daemon)
    controller_spawn = room(records, "spawn", controller)
    controller_reap = room(records, "reap", controller)
    assert old_spawn[1] < old_helper_spawn[1], "old helper birth preceded main birth"
    assert old_helper_spawn[1] < old_reap[1] and old_helper_spawn[1] < old_helper_reap[1], (
        "old main/helper reaping preceded the live helper birth"
    )
    assert actor_spawn[1] < old_reap[1], "Root actor did not exist before the old target died"
    assert old_helper_reap[1] < actor_reap[1], "old Team was not fully reaped before the actor ended"
    assert daemon_spawn[1] < old_reap[1] < daemon_reap[1], "daemon was not alive across the stop"
    assert controller_spawn[1] < old_reap[1] < controller_reap[1], (
        "controller was not alive across the stop"
    )

    assert "task: all tasks exited, system halted" in console, "system did not halt normally"
    frames = one(console, r"^memory: frame_allocations before=(\d+) after=(\d+)$")
    assert frames[1] == frames[2], "final frame baseline was not restored"
    return {
        "root": root,
        "daemon": daemon,
        "controller": controller,
        "actor": actor,
        "old": (old_task, old_team, old_helper),
        "before": before,
        "checks": checks,
        "frames": int(frames[1]),
    }


def check(console_path, events_path, qtest_path, raw_code):
    result = verify(
        Path(console_path).read_text(),
        events_of(events_path),
        Path(qtest_path).read_text(),
        raw_code,
    )
    print(
        "daemon-stop: old task {old} team {old_team} helper {old_helper} retired; "
        "actor {actor}; daemon {daemon}; stable checks {checks}; frames {frames}".format(
            old=result["old"][0],
            old_team=result["old"][1],
            old_helper=result["old"][2],
            actor=result["actor"],
            daemon=result["daemon"],
            checks=result["checks"],
            frames=result["frames"],
        )
    )


if __name__ == "__main__":
    if len(sys.argv) != 5:
        raise SystemExit("usage: daemon_stop.py CONSOLE DIAGNOSE QTEST RAW_CODE")
    check(*sys.argv[1:])
