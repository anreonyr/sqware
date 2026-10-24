"""Check real command-fetch failure and hardware completion without forged writes."""

import json
from pathlib import Path
import sys

from hardware import CQB, CQCSR, CQT, MEM_FAULT, regions
from ruin_check import identity, one, reaps, tasks
from trace import check as execution


def pending(console, events):
    main, auxiliary, beta = tasks(console)
    retired = reaps(events)
    assert main in retired and auxiliary in retired, "alpha Team not completely reaped"
    assert beta not in retired, "recovery reaped before fence confirmation"
    one(console, r"iommu: binding 1 acquired requester=\d+ pages=6 iova=0x[0-9a-f]+")
    one(console, r"ctl: served used=1 len=513 written_ok=true sector_ok=true status=0")
    one(console, r"iommu: binding 1 invalidate fence expired frames held=6")
    one(console, r"recovery: binding 1 stage Invalidate failed party=3 op=3 code=4")
    for service in ("hub", "pci", "iommu", "recovery", "device-ctl-fault"):
        task = int(one(console, rf"system: minted {service} tid=(\d+)")[1])
        assert task not in retired, f"{service} reaped during hardware failure"
    for forbidden in (
        "iommu: binding 1 invalidated",
        "pci: binding 1 disabling",
        "recovery: binding 1 stopped",
        "iommu: binding 1 queues stopped",
        "iommu: binding 1 released",
        "hub: binding 1 closed",
    ):
        assert forbidden not in console, f"advanced before hardware completion: {forbidden}"


def hardware(data, dts, requester):
    base, bad = regions(dts)
    assert data["base"] == base and data["bad"] == bad, "wrong device tree"
    assert data["original"] != data["broken"]
    assert data["broken"] == ((bad >> 12) << 10) | (data["original"] & 0x1f)
    assert data["fault"] & MEM_FAULT, "no real command memory fault"
    entries = 2 << (data["original"] & 0x1f)
    assert (data["tail"] - data["head"]) % entries >= 3, "missing outstanding commands"
    assert data["directory"] == 3 | (1 << 33) | (requester << 40), "wrong device-directory invalidation"
    assert data["invalidate"] == 1 | (1 << 32) | ((requester + 1) << 12), "wrong address-space invalidation"
    assert data["fence"] & 0x7f == 2
    assert data["fence"] >> 32 == data["cookie"] != 0
    assert data["destination"] % 4 == 0
    assert data["before"] == 0 and data["after"] == data["cookie"]
    writes = []
    completion_reads = []
    reads = {}
    for operation in data["operations"]:
        words = operation["request"].split()
        reply = operation["reply"].split()
        assert reply[0] == "OK", "qtest operation failed"
        address = int(words[1], 0)
        if words[0].startswith("write"):
            assert address in (base + CQB, base + CQCSR, base + CQT), "forged guest-memory write"
            writes.append((words[0], address, int(words[2], 0)))
        else:
            seen = int(reply[1], 0)
            reads.setdefault((words[0], address), []).append(seen)
            if words[0] == "readl" and address == data["destination"]:
                completion_reads.append(seen)
    queue = (data["original"] >> 10) << 12
    last = queue + ((data["tail"] - 1) % entries) * 16
    prior = queue + ((data["tail"] - 2) % entries) * 16
    directory_at = queue + ((data["tail"] - 3) % entries) * 16
    assert reads[("readq", base + CQB)] == [data["original"], data["broken"], data["original"]]
    assert reads[("readq", prior)] == [data["invalidate"]]
    assert reads[("readq", directory_at)] == [data["directory"]]
    assert reads[("readq", last)] == [data["fence"]]
    assert reads[("readq", last + 8)] == [data["destination"] >> 2]
    assert completion_reads[0] == 0 and completion_reads[-1] == data["cookie"]
    assert writes == [
        ("writel", base + CQCSR, 0),
        ("writeq", base + CQB, data["broken"]),
        ("writel", base + CQCSR, 1 | MEM_FAULT),
        ("writel", base + CQCSR, 0),
        ("writeq", base + CQB, data["original"]),
        ("writel", base + CQCSR, 1 | MEM_FAULT | (1 << 9) | (1 << 10)),
        ("writel", base + CQT, data["tail"]),
    ], "unexpected repair sequence"


def check(console_path, events_path, capture_path):
    capture = Path(capture_path)
    console = Path(console_path).read_text()
    requester, key = identity(console)
    events = [json.loads(line) for line in Path(events_path).read_text().splitlines()]
    failed = capture.with_suffix(".failed-console").read_text()
    failed_events = [
        json.loads(line) for line in capture.with_suffix(".failed-diagnose.jsonl").read_text().splitlines()
    ]
    assert events[:len(failed_events)] == failed_events, "failure snapshot is not this trace's prefix"
    pending(failed, failed_events)
    data = json.loads(capture.with_suffix(".hardware.json").read_text())
    hardware(data, capture.with_suffix(".dts").read_text(), int(requester))
    execution(console_path, events_path)
    ordered = (
        r"iommu: binding 1 invalidate fence expired frames held=6",
        r"recovery: binding 1 stage Invalidate failed party=3 op=3 code=4",
        r"iommu: binding 1 invalidated frames held=6 \(not released\)",
        r"pci: binding 1 disabling bus-master",
        r"recovery: binding 1 stopped reset=1 status=0 queue_enable=0",
        r"iommu: binding 1 queues stopped cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 faults=0 last_did=0",
        r"iommu: binding 1 released ddtp=0x0 cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 frames=0 stopped=true",
        r"recovery: binding 1 detached",
        rf"hub: binding 1 closed and released device {key}",
        r"ctl: closed binding 1 failure party=3 op=3 code=4",
        r"recovery: binding 2 active iova=0x[0-9a-f]+ execution=\d+",
        r"ctl: stale iova submitted sentinel_held=true status_held=true",
        rf"hub: binding 2 closed and released device {key}",
        r"exit tid=\d+ reason=0x0 note: device-ctl: done",
        r"exit tid=\d+ reason=0x0 note: system: done",
        r"task: all tasks exited, system halted",
    )
    positions = [one(console, pattern).start() for pattern in ordered]
    assert positions == sorted(positions), "hardware recovery stop chain out of order"
    frames = one(console, r"memory: frame_allocations before=(\d+) after=(\d+)")
    assert frames[1] == frames[2], "physical allocation objects leaked"
    assert events[-1].get("kind") == {"halt": "halt"}, "incomplete trace"
    backend = [
        json.loads(line) for line in capture.with_suffix(".block-events").read_text().splitlines()
    ]
    held = [index for index, record in enumerate(backend) if record["phase"] == "held"]
    assert len(held) == 1
    index = held[0]
    assert [record["phase"] for record in backend[index:index + 3]] == ["held", "resuming", "replied"]
    assert backend[index + 2]["error"] == 0
    print(f"hardware fence: real CQ memory fault and Fence::Expired; same cookie {data['cookie']:#x} completed; frames {frames[1]} → {frames[2]}")


if __name__ == "__main__":
    check(*sys.argv[1:])
