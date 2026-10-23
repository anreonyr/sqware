"""Strict independent verifier for the ordinary target-with-helper restart scene.

Console and structured traces come from the runner. Synthetic records are used
only by the parser's own negative controls.
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
    recorded_root = int(one(console, r"^target-root: root task=([1-9][0-9]*)$")[1])
    assert recorded_root == root, "Root harness identity differs from boot entry"
    exit_root = one(
        console, r"^exit tid=([1-9][0-9]*) reason=0x0 note: daemon-target: done$"
    )
    assert int(exit_root[1]) == root, "Root did not exit successfully as the boot entry"

    daemon = int(one(console, r"^system: activate daemon tid=([1-9][0-9]*)$")[1])
    daemon_construct = one(
        console, r"^system: static construct name=daemon task=(\d+) team=(\d+) owner=(\d+)$"
    )
    assert int(daemon_construct[1]) == daemon, "daemon activation identity differs from construction"
    assert int(daemon_construct[3]) == root, "daemon construction owner is not Root"
    admitted = one(
        console, r"^system: recover admitted target=fixturetarget caller=([1-9][0-9]*)$"
    )
    assert int(admitted[1]) == daemon, "admitted Recover sender is not the daemon main"

    old = one(console, r"^target-root: old task=(\d+) team=(\d+)$")
    old_task, old_team = int(old[1]), int(old[2])
    observed = one(
        console, r"^daemon-target: observed old task=(\d+) helper=(\d+) before=(\d+)$"
    )
    old_helper = int(observed[2])
    before = int(observed[3])
    assert int(observed[1]) == old_task, "controller observed a different old task"
    assert old_helper not in (0, old_task), "old helper is not a distinct live task"
    assert before > 0, "controller did not obtain a real clock value"

    controller = int(
        one(
            console,
            r"^system: static construct name=daemon-ctl-target task=(\d+) team=(\d+) owner=(\d+)$",
        )[1]
    )
    controller_owner = int(
        one(
            console,
            r"^system: static construct name=daemon-ctl-target task=(\d+) team=(\d+) owner=(\d+)$",
        )[3]
    )
    assert controller_owner == root, "controller construction owner is not Root"
    request = one(
        console,
        r"^target-root: request controller=(\d+) old task=(\d+) helper=(\d+) before=(\d+)$",
    )
    assert int(request[1]) == controller, "request did not come from the deployed controller"
    assert int(request[2]) == old_task, "request old task differs from the actual target"
    assert int(request[3]) == old_helper, "request helper differs from the real heartbeat"
    assert int(request[4]) == before, "request time differs from the controller observation"

    kill = one(console, r"^target-root: kill old task=(\d+) team=(\d+) before=(\d+)$")
    assert (int(kill[1]), int(kill[2]), int(kill[3])) == (old_task, old_team, before), (
        "kill did not target the actual old target main"
    )
    assert not all_of(console, r"^target-root: kill old helper="), (
        "Root must kill only the old main; Control retires the whole old Team"
    )

    retired_root = one(console, r"^target-root: retired old task=(\d+) team=(\d+)$")
    assert (int(retired_root[1]), int(retired_root[2])) == (old_task, old_team), (
        "Root recorded another retirement"
    )
    new = one(console, r"^target-root: new task=(\d+) team=(\d+)$")
    new_task, new_team = int(new[1]), int(new[2])
    assert new_task != old_task, "replacement kept the old task id"
    assert new_team != old_team, "replacement kept the old team id"

    proof = one(
        console,
        r"^daemon-target: proof old task=(\d+) helper=(\d+) before=(\d+) new task=(\d+) helper=(\d+) after=(\d+)$",
    )
    assert int(proof[1]) == old_task, "proof stale old task"
    assert int(proof[2]) == old_helper, "proof stale old helper"
    assert int(proof[3]) == before, "proof stale before value"
    assert int(proof[4]) == new_task, "fresh namespace entry belongs to another task"
    new_helper = int(proof[5])
    after = int(proof[6])
    assert new_helper not in (0, old_helper, new_task), "fresh helper is not distinct"
    assert after >= before, "replacement real time went backwards"
    exit_controller = one(
        console, r"^exit tid=(\d+) reason=0x0 note: daemon-ctl-target: done$"
    )
    assert int(exit_controller[1]) == controller, "controller did not exit successfully"

    constructs = all_of(
        console,
        r"^system: static construct name=fixturetarget task=(\d+) team=(\d+) owner=(\d+)$",
    )
    assert len(constructs) == 2, "expected initial and replacement target construction"
    built = {(int(match[1]), int(match[2])): match for match in constructs}
    assert (old_task, old_team) in built, "old target construction missing"
    assert (new_task, new_team) in built, "new target construction missing"
    assert all(int(match[3]) == root for match in constructs), (
        "target construction owner is not Root"
    )

    retirements = all_of(
        console, r"^system: retired name=fixturetarget task=(\d+) team=(\d+)$"
    )
    retired_old = [
        match
        for match in retirements
        if (int(match[1]), int(match[2])) == (old_task, old_team)
    ]
    assert len(retired_old) == 1, "finalized old target retirement missing"
    old_retirement = retired_old[0]
    new_construct = built[(new_task, new_team)]

    old_spawn = room(records, "spawn", old_task)
    old_helper_spawn = room(records, "spawn", old_helper)
    old_reap = room(records, "reap", old_task)
    old_helper_reap = room(records, "reap", old_helper)
    new_spawn = room(records, "spawn", new_task)
    new_helper_spawn = room(records, "spawn", new_helper)
    new_reap = room(records, "reap", new_task)
    new_helper_reap = room(records, "reap", new_helper)
    daemon_spawn = room(records, "spawn", daemon)
    daemon_reap = room(records, "reap", daemon)
    controller_spawn = room(records, "spawn", controller)
    controller_reap = room(records, "reap", controller)

    for name, event in [
        ("old spawn", old_spawn),
        ("old helper spawn", old_helper_spawn),
        ("new spawn", new_spawn),
        ("new helper spawn", new_helper_spawn),
        ("daemon spawn", daemon_spawn),
        ("controller spawn", controller_spawn),
    ]:
        assert event[1] is not None, f"{name} lacks real time"
    assert old_spawn[1] < old_helper_spawn[1], "old helper was not spawned by the old target"
    assert old_helper_spawn[1] < old_reap[1], "old helper did not outlive its main until retirement"
    assert old_helper_spawn[1] < old_helper_reap[1], "old helper reap preceded its birth"
    assert old_reap[1] < new_spawn[1], "old target reap did not precede new target spawn"
    assert old_helper_reap[1] < new_spawn[1], "old helper reap did not precede new target spawn"
    assert new_spawn[1] < new_helper_spawn[1], "new helper did not belong to the new target"
    assert new_helper_spawn[1] < new_helper_reap[1], "new helper was not reaped after its spawn"
    assert daemon_spawn[1] < old_reap[1] < new_spawn[1] < daemon_reap[1], (
        "daemon was not alive across actual recovery"
    )
    assert controller_spawn[1] < old_reap[1] < new_spawn[1] < controller_reap[1], (
        "controller was not alive across actual recovery"
    )
    assert daemon_reap[1] < new_reap[1], "target stopped before its supervisor"
    assert daemon_reap[1] < new_helper_reap[1], "target helper stopped before its supervisor"

    daemon_retirement = one(
        console,
        rf"^system: retired name=daemon task={daemon} team={int(daemon_construct[2])}$",
    )
    new_retirement = one(
        console, rf"^system: retired name=fixturetarget task={new_task} team={new_team}$"
    )
    assert proof.start() < daemon_retirement.start() < new_retirement.start() < exit_root.start(), (
        "supervisor/provider closing order is wrong"
    )
    assert admitted.start() < daemon_retirement.start(), "recovery was admitted during final closing"

    ordered = [
        old.start(),
        observed.start(),
        request.start(),
        kill.start(),
        old_retirement.start(),
        admitted.start(),
        new_construct.start(),
        proof.start(),
        exit_controller.start(),
        exit_root.start(),
    ]
    assert ordered == sorted(ordered), "actual restart proof is out of order"
    assert kill.start() < retired_root.start() < new.start(), (
        "Root retirement record is out of order"
    )
    assert new_construct.start() < new.start(), (
        "Root observed the replacement before its construction"
    )

    assert "task: all tasks exited, system halted" in console, "system did not halt normally"
    frames = one(console, r"^memory: frame_allocations before=(\d+) after=(\d+)$")
    assert frames[1] == frames[2], "final frame baseline was not restored"
    return {
        "root": root,
        "daemon": daemon,
        "controller": controller,
        "old": (old_task, old_team, old_helper),
        "new": (new_task, new_team, new_helper),
        "before": before,
        "after": after,
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
        "daemon-target: old task {old} team {old_team} helper {old_helper} reaped before "
        "new task {new} team {new_team} helper {new_helper}; daemon {daemon}; "
        "time {before} -> {after}; frames {frames}".format(
            old=result["old"][0],
            old_team=result["old"][1],
            old_helper=result["old"][2],
            new=result["new"][0],
            new_team=result["new"][1],
            new_helper=result["new"][2],
            daemon=result["daemon"],
            before=result["before"],
            after=result["after"],
            frames=result["frames"],
        )
    )


if __name__ == "__main__":
    if len(sys.argv) != 5:
        raise SystemExit("usage: daemon_target.py CONSOLE DIAGNOSE QTEST RAW_CODE")
    check(*sys.argv[1:])
