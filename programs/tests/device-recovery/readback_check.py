"""Require actual MMIO refusal, retained obligations and recovery of the same binding."""

import json
import re
from pathlib import Path
import sys

from readback import FAILURE, ecam_region
from ruin_check import identity, one, reaps, tasks
from trace import check as execution


def check(console_path, events_path, capture_path):
    capture = Path(capture_path)
    console = Path(console_path).read_text()
    assert not re.search(r"user fault killed:|\[ERROR\]|note: (?:work|dual|ctl):", console), "native fault is not successful retirement"
    events = [json.loads(line) for line in Path(events_path).read_text().splitlines()]
    failed = capture.with_suffix(".failed-console").read_text()
    prefix = [json.loads(line) for line in capture.with_suffix(".failed-diagnose.jsonl").read_text().splitlines()]
    assert events[:len(prefix)] == prefix, "failure snapshot is not this run's prefix"
    main, auxiliary, driver = tasks(failed)
    retired = reaps(prefix)
    assert main in retired and auxiliary in retired and driver not in retired
    one(failed, FAILURE)
    one(failed, r"iommu: binding 1 invalidated frames held=6 \(not released\)")
    for forbidden in ("recovery: binding 1 stopped", "iommu: binding 1 queues stopped",
                      "iommu: binding 1 released", "recovery: binding 1 detached", "hub: binding 1 closed"):
        assert forbidden not in failed, f"premature cleanup: {forbidden}"
    data = json.loads(capture.with_suffix(".readback.json").read_text())
    assert (data["ecam"], data["ecam_size"]) == ecam_region(capture.with_suffix(".dts").read_text())
    assert (data["requester"] + 1) << 12 <= data["ecam_size"]
    requester, key = identity(console)
    assert data["requester"] == int(requester)
    placed = one(console, rf"pci: endpoint requester={requester} bar=(\d+) address=0x([0-9a-f]+) size=0x([0-9a-f]+) bus-master off=0x0")
    assert (data["bar"], data["base"], data["size"]) == (int(placed[1]), int(placed[2], 16), int(placed[3], 16))
    assert data["initial_status"] & 4 and data["failed_status"] == 1 and data["failed_queue"] == 0
    assert data["repaired_status"] == data["repaired_queue"] == 0
    assert 0 <= data["common"] <= data["size"] - 0x1e
    assert data["status_address"] == data["base"] + data["common"] + 0x14
    assert data["queue_address"] == data["base"] + data["common"] + 0x1c
    writes = []
    status_reads = []
    reads = {}
    status_positions = []
    write_positions = []
    queue_positions = []
    for index, operation in enumerate(data["operations"]):
        words, reply = operation["request"].split(), operation["reply"].split()
        assert reply[0] == "OK"
        if words[0].startswith("write"):
            assert words[0] == "writeb" and int(words[1], 0) == data["status_address"], "non-MMIO or forged confirmation write"
            writes.append(int(words[2], 0))
            write_positions.append(index)
        elif words[0] == "readb" and int(words[1], 0) == data["status_address"]:
            status_reads.append(int(reply[1], 0))
            status_positions.append(index)
        else:
            reads.setdefault((words[0], int(words[1], 0)), []).append(int(reply[1], 0))
            if words[0] == "readw" and int(words[1], 0) == data["queue_address"]:
                queue_positions.append(index)
    assert len(writes) >= 2 and set(writes[:-1]) == {1} and writes[-1] == 0
    assert status_reads == [data["initial_status"], 1, 0]
    assert status_positions[0] < write_positions[0]
    assert write_positions[-2] < status_positions[1] < write_positions[-1] < status_positions[2], "MMIO refusal/repair order changed"
    assert reads[("readw", data["queue_address"])] == [data["failed_queue"], data["repaired_queue"]]
    assert len(queue_positions) == 2
    assert status_positions[1] < queue_positions[0] < write_positions[-1]
    assert status_positions[2] < queue_positions[1]
    config = data["ecam"] + (data["requester"] << 12)

    def read(op, address):
        values = reads[(op, address)]
        assert len(values) == 1, "ambiguous hardware configuration read"
        return values[0]

    assert read("readw", config) == 0x1af4
    assert read("readw", config + 2) in (0x1001, 0x1042)
    low = read("readl", config + 0x10 + data["bar"] * 4)
    high = read("readl", config + 0x14 + data["bar"] * 4) if low & 6 == 4 else 0
    assert (low & ~0xf) | (high << 32) == data["base"]
    at = read("readb", config + 0x34)
    visited = set()
    while at:
        assert at not in visited and 0x40 <= at <= 0xfc
        visited.add(at)
        vendor = read("readb", config + at)
        if vendor == 9 and read("readb", config + at + 3) == 1:
            assert read("readb", config + at + 4) == data["bar"]
            assert read("readl", config + at + 8) == data["common"]
            break
        at = read("readb", config + at + 1)
    else:
        raise AssertionError("no actual virtio common capability")
    ordered = (
        r"iommu: binding 1 invalidated frames held=6 \(not released\)",
        r"pci: binding 1 disabling bus-master", FAILURE,
        r"recovery: binding 1 stopped reset=1 status=0 queue_enable=0",
        r"iommu: binding 1 queues stopped cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 faults=0 last_did=0",
        r"iommu: binding 1 released ddtp=0x0 cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 frames=0 stopped=true",
        r"recovery: binding 1 detached", rf"hub: binding 1 closed and released device {key}",
        r"ctl: closed binding 1 failure party=4 op=2 code=4",
        r"recovery: binding 2 active iova=0x[0-9a-f]+ execution=\d+",
        r"ctl: stale iova submitted sentinel_held=true status_held=true",
        rf"hub: binding 2 closed and released device {key}",
        r"exit tid=\d+ reason=0x0 note: device-ctl: done",
        r"exit tid=\d+ reason=0x0 note: system: done",
        r"task: all tasks exited, system halted",
    )
    positions = [one(console, pattern).start() for pattern in ordered]
    assert positions == sorted(positions), "reset readback retry changed stop order"
    retired, last = execution(console_path, events_path)
    branches = one(console, r"doom: held=(\d+) starved=(\d+) blocked=(\d+) nudged=(\d+)")
    assert sum(map(int, branches.groups())) > 0, "no actual domain culling observed"
    for task in map(int, re.findall(r"^system: minted \S+ tid=(\d+)$", console, re.MULTILINE)):
        assert task in retired, f"static task {task} was not reaped"
    root = int(one(console, r"boot: entry task=([1-9]\d*)")[1])
    assert root in retired and retired[root] == max(retired.values()), "root retired before all owned tasks"
    frames = one(console, r"memory: frame_allocations before=(\d+) after=(\d+)")
    assert frames[1] == frames[2] and last == {"halt": "halt"}
    backend = [json.loads(line) for line in capture.with_suffix(".block-events").read_text().splitlines()]
    held = [i for i, event in enumerate(backend) if event["phase"] == "held"]
    assert len(held) == 1
    index = held[0]
    assert [row["phase"] for row in backend[index:index + 3]] == ["held", "resuming", "replied"]
    assert backend[index + 2]["error"] == 0
    print(f"reset readback: real status=1 refused, same binding repaired, original failure retained; frames {frames[1]} → {frames[2]}")


if __name__ == "__main__":
    check(*sys.argv[1:])
