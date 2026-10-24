"""Strict independent verifier for a deployed guard death scene.

The runner is expected to time out (exit 101) because the guarded kernel path
never returns. A timeout alone is not acceptance: this verifier requires the
retained NBD read, the matching Root/Kernel identities, and the non-returning
guard stop sequence.
"""

import argparse
import json
import re
from pathlib import Path

TARGETS = ("pci", "iommu", "recovery")
FORBIDDEN = (
    "iommu: binding 1 invalidated",
    "pci: binding 1 disabling bus-master",
    "pci: endpoint bus-master off=0x0",
    "hub: binding 1 execution domain stopped",
    "recovery: binding 1 stopped",
    "iommu: binding 1 queues stopped",
    "iommu: binding 1 released",
    "recovery: binding 1 detached",
    "hub: binding 1 closed",
    "task: all tasks exited, system halted",
    "critical guard returned after slay",
)


def one(text, pattern):
    matches = list(re.finditer(pattern, text, re.MULTILINE))
    assert len(matches) == 1, f"expected one proof: {pattern}"
    return matches[0]


def phases(records):
    return [record.get("phase") for record in records]


def retained_events(records, label):
    assert phases(records).count("held") == 1, f"{label}: expected one held read"
    index = phases(records).index("held")
    after = phases(records)[index + 1:]
    assert "resuming" not in after, f"{label}: retained read was resumed"
    assert "replied" not in after, f"{label}: retained read completed"
    assert all(phase == "disconnected" for phase in after), f"{label}: unexpected NBD phase {after}"
    held = records[index]
    assert (held.get("offset"), held.get("length")) == (0, 512), f"{label}: not the in-flight sector"
    return held


def verify(console, held_console, events, held_events, block, feed, target, harts,
           runner_code, qtest_log):
    assert target in TARGETS, "unknown guard target"
    assert harts > 1, "guard death requires peer harts"
    assert runner_code == 101, "raw qtest must remain the non-returning 101"
    assert re.search(r"timed out after \d+s", qtest_log), "VM did not remain halted"
    assert re.search(r"test embedded::tests::scene\s+\.\.\.\s+FAILED", qtest_log), "scene did not fail by halt"
    assert events, "missing structured execution trace"
    root = int(one(console, r"^guard: root task=(\d+)$")[1])

    armed = one(console, rf"^guard: armed target={target} task=(\d+) team=(\d+)$")
    submitted = one(console, r"^guard: submitted controller=(\d+)$")
    ready = one(console, rf"^guard: ready target={target} task=(\d+) team=(\d+)$")
    trigger = one(console, rf"^guard: trigger target={target} task=(\d+) team=(\d+)$")
    assert (armed[1], armed[2]) == (ready[1], ready[2]) == (trigger[1], trigger[2]), (
        "ready/trigger identity changed"
    )
    assert armed.start() < submitted.start() < ready.start(), "arm/submission/ready out of order"
    assert ready.start() < trigger.start(), "trigger preceded ready"
    protected = one(console, r"^guard: critical domains protected=3$")
    assert submitted.start() < protected.start() < ready.start(), "three-domain proof out of order"
    drivers = {
        name: one(console, rf"^system: minted {name} tid=(\d+)$")
        for name in TARGETS
    }
    assert len({record[1] for record in drivers.values()}) == 3, "critical drivers share a task"
    assert str(root) not in {record[1] for record in drivers.values()}, "Root is a critical Driver"
    assert drivers[target][1] == trigger[1], "kernel target does not match the named Driver"
    assert all(record.start() < submitted.start() for record in drivers.values())
    published = one(console, r"^iommu: binding 1 published requester=\d+ held=6 ddtp=0x[0-9a-f]+$")
    master = one(console, r"^pci: endpoint bus-master on=0x4$")
    active = one(console, r"^recovery: binding 1 active iova=0x[0-9a-f]+ execution=\d+$")
    doorbell = one(console, r"^work: rang the in-flight request doorbell$")
    assert published.start() < master.start() < active.start() < doorbell.start() < submitted.start(), (
        "DMA publication/submission out of order"
    )

    trip = one(console, r"^\[critical\] trip task=(\d+) task_team=(\d+) team=(\d+)$")
    assert trip[1] == trigger[1], "kernel trip task is not the trigger target"
    assert trip[2] == trigger[2] == trip[3], "kernel protected team is not the trigger team"
    assert trigger.start() < trip.start(), "kernel trip preceded trigger"

    guarded = one(console, r"^\[critical\] guarded exit reason=0x[0-9a-f]+; no cleanup follows$")
    quiet = one(console, rf"^\[critical\] peer harts quiesced={harts - 1}$")
    assert trip.start() < guarded.start() < quiet.start(), "guard stop proof out of order"

    task = int(trigger[1])
    assert not re.search(rf"^\[critical\] reference cleanup task={task}$", console, re.MULTILINE), (
        "guarded target entered reference cleanup"
    )
    assert "panic" not in console and "test panic:" not in console, "halt contains a panic"
    prefix = console[:trigger.start()]
    suffix = console[trigger.start():]
    for advanced in (
        "iommu: binding 1 invalidated",
        "pci: binding 1 disabling bus-master",
        "pci: endpoint bus-master off=0x0",
        "hub: binding 1 execution domain stopped",
        "recovery: binding 1 stopped",
        "iommu: binding 1 released",
        "hub: binding 1 closed",
        "recovery: binding 1 detached",
    ):
        assert advanced not in prefix, f"pre-trigger binding already advanced: {advanced}"
    for forbidden in FORBIDDEN:
        assert forbidden not in console, f"cleanup/release reached: {forbidden}"
    assert re.search(rf"^system: (?:minted|activate) {target}\b", suffix, re.MULTILINE) is None, (
        f"target {target} was minted or activated again after trigger"
    )

    assert feed.get("sent_g") is True, "host never sent the single G trigger"
    assert feed.get("held_before_trigger") is True, "NBD was not held before trigger"
    assert feed.get("resume") is False, "host resumed the held read"
    assert feed.get("trigger") is True, "host did not observe trigger"
    assert feed.get("serial_eof") is True, "serial did not reach EOF after the halt"
    assert feed.get("reader_error") is None, "serial reader failed"
    held_ready = one(held_console, rf"^guard: ready target={target} task=(\d+) team=(\d+)$")
    assert held_ready.groups() == ready.groups(), "held snapshot belongs to another target"
    assert "guard: trigger" not in held_console and "[critical] trip" not in held_console, (
        "death preceded the host trigger"
    )

    held = retained_events(held_events, "held snapshot")
    final = retained_events(block, "final NBD")
    assert held.get("cookie") == final.get("cookie"), "held request changed identity"
    assert not any(record.get("phase") == "resuming" for record in block), "read was resumed"

    retired = set()
    for event in events:
        reap = event.get("kind", {}).get("room", {}).get("reap", {})
        if reap:
            retired.add(reap.get("tid"))
    assert task not in retired, "guarded target was reaped before CPU quiescence"
    assert not any(
        event.get("kind", {}).get("room", {}).get("exit", {}).get("tid") == root
        for event in events
    ), "root task returned normally"
    assert re.search(rf"^exit tid={root} reason=", console, re.MULTILINE) is None, (
        "root task exited before the protected stop"
    )
    print(
        f"guard {target}: task {task} team {trigger[2]} halted before cleanup; "
        f"NBD held without resume; {harts - 1} peer harts quiesced"
    )


def load_events(path):
    path = Path(path)
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def check(console_path, events_path, capture_path, target, harts, runner_code,
          qtest_log, hold_path=None):
    capture = Path(capture_path)
    feed = json.loads(capture.with_suffix(".feed.json").read_text())
    hold = Path(hold_path) if hold_path else None
    resume = bool(feed.get("resume")) or (hold is not None and Path(str(hold) + ".resume").exists())
    assert not resume, "NBD resume receipt exists"
    verify(
        Path(console_path).read_text(),
        capture.with_suffix(".held-console").read_text(),
        load_events(events_path),
        load_events(capture.with_suffix(".held-events")),
        load_events(capture.with_suffix(".block-events")),
        feed,
        target,
        int(harts),
        int(runner_code),
        Path(qtest_log).read_text(),
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("console")
    parser.add_argument("events")
    parser.add_argument("capture")
    parser.add_argument("target", choices=TARGETS)
    parser.add_argument("harts", type=int)
    parser.add_argument("runner_code", type=int)
    parser.add_argument("qtest_log")
    parser.add_argument("--hold", default=None)
    args = parser.parse_args()
    check(args.console, args.events, args.capture, args.target, args.harts,
          args.runner_code, args.qtest_log, args.hold)
