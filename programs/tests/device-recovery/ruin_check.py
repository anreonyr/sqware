"""Check retained ownership during Ruin and the completed native stop chain."""

import json
import re
import sys
from pathlib import Path


def one(console, pattern):
    matches = list(re.finditer("^" + pattern + "$", console, re.MULTILINE))
    assert len(matches) == 1, f"expected one native proof: {pattern}"
    return matches[0]


def tasks(console):
    main = int(one(console, r"ctl: constructed device-work tid=(\d+) owner=hub")[1])
    auxiliary = int(one(console, r"ctl: binding 1 auxiliary=(\d+) alive=true")[1])
    beta = int(one(console, r"system: minted virtio-blk tid=(\d+)")[1])
    assert len({main, auxiliary, beta}) == 3, "execution and recovery must be distinct"
    return main, auxiliary, beta


def identity(console):
    pci = one(console, r"pci: space task=(\d+) ecam=0x[0-9a-f]+ functions=\d+")
    ready = one(console, r"virtio-blk: ready hub=\d+ pci=(\d+) requester=(\d+) key=(\d+)")
    assert ready[1] == pci[1], "Driver ready PCI task mismatch"
    return ready[2], ready[3]


def reaps(events):
    out = {}
    for event in events:
        reap = event.get("kind", {}).get("room", {}).get("reap", {})
        if reap:
            tid = reap["tid"]
            assert tid not in out, f"duplicate reap: {tid}"
            out[tid] = event["when"]
    return out


def pending(console, events, mode="ruin"):
    assert mode in ("ruin", "forced")
    main, auxiliary, beta = tasks(console)
    if mode == "forced":
        one(console, r"ctl: awaiting forced global shutdown with in-flight read")
        assert re.search(r"^system: idle deadline reached; awaiting (?:DMA|retirement) settlement$", console, re.MULTILINE)
        for service in ("hub", "pci", "iommu", "virtio-blk", "device-ctl-forced"):
            task = int(one(console, rf"system: minted {service} tid=(\d+)")[1])
            assert task not in reaps(events), f"{service} reaped before DMA settlement"
    else:
        one(console, r"ruin: requested virtio-blk")
    one(console, rf"hub: binding 1 execution domain stopped \(tid={main}\)")
    one(console, r"iommu: binding 1 invalidated frames held=6 \(not released\)")
    one(console, r"pci: binding 1 disabling bus-master")
    retired = reaps(events)
    assert main in retired and auxiliary in retired, "alpha Team not completely reaped"
    assert beta not in retired, "static Driver reaped while DMA read held"
    for forbidden in (
        "pci: endpoint bus-master off=0x0",
        "virtio-blk: binding 1 stopped",
        "iommu: binding 1 queues stopped",
        "iommu: binding 1 released",
        "virtio-blk: binding 1 detached",
        "hub: binding 1 closed",
        "ruin: virtio-blk dead after reply",
        "system: forced shutdown admitted after DMA settlement",
        "system: forced shutdown admitted after retirement settlement",
        "system: frame Step(Idle)",
        "task: all tasks exited, system halted",
    ):
        assert forbidden not in console, f"premature retirement: {forbidden}"


def completed(console, events, backend, mode="ruin"):
    assert mode in ("ruin", "forced")
    root = int(one(console, r"boot: entry task=([1-9]\d*)")[1])
    main, auxiliary, beta = tasks(console)
    requester, key = identity(console)
    ordered = (
        r"iommu: binding 1 published requester=" + requester + r" held=6 ddtp=0x[0-9a-f]+",
        r"pci: endpoint bus-master on=0x4",
        r"virtio-blk: binding 1 active iova=0x[0-9a-f]+ execution=\d+",
        r"work: rang the in-flight request doorbell",
        (
            r"ctl: awaiting forced global shutdown with in-flight read"
            if mode == "forced" else r"ruin: requested virtio-blk"
        ),
        rf"hub: binding 1 execution domain stopped \(tid={main}\)",
        r"iommu: binding 1 invalidated frames held=6 \(not released\)",
        r"pci: binding 1 disabling bus-master",
        r"pci: endpoint bus-master off=0x0",
        r"virtio-blk: binding 1 stopped reset=1 status=0 queue_enable=0",
        r"iommu: binding 1 queues stopped cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 faults=0 last_did=0",
        r"iommu: binding 1 released ddtp=0x0 cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 frames=0 stopped=true",
        r"virtio-blk: binding 1 detached",
        rf"hub: binding 1 closed and released device {key}",
        *(
            (
                r"system: forced shutdown admitted after (?:DMA|retirement) settlement",
                r"system: frame Step\(Idle\)",
            )
            if mode == "forced" else (
                r"ruin: virtio-blk dead after reply",
                rf"exit tid={root} reason=0x0 note: iommu-ruin: done",
            )
        ),
        r"task: all tasks exited, system halted",
    )
    positions = [one(console, pattern).start() for pattern in ordered]
    assert positions == sorted(positions), "native stop chain out of order"
    one(console, rf"ctl: binding 1 auxiliary={auxiliary} alive=false")
    one(console, r"ctl: drained read sector_ok=true status=0 used=\d+ len=\d+")
    if mode == "forced":
        one(console, r"ctl: drained read verified; awaiting global retirement")
    else:
        one(console, r"ctl: in-flight binding settled by static Driver Ruin")
        one(console, r"exit tid=\d+ reason=0x0 note: device-ctl-ruin: done")
    frames = one(console, r"memory: frame_allocations before=(\d+) after=(\d+)")
    assert frames[1] == frames[2], "physical allocation objects not reclaimed"
    assert events and events[-1].get("kind") == {"halt": "halt"}, "incomplete trace"
    retired = reaps(events)
    assert all(tid in retired for tid in (main, auxiliary, beta)), "missing native reap"
    assert max(retired[main], retired[auxiliary]) < retired[beta], "Driver reaped before alpha Team"
    if mode == "forced":
        for service in ("hub", "pci", "iommu", "virtio-blk", "device-ctl-forced"):
            task = int(one(console, rf"system: minted {service} tid=(\d+)")[1])
            assert task in retired, f"missing global retirement: {service}"
            assert max(retired[main], retired[auxiliary]) < retired[task]
        assert retired.get(root, 0) > max(retired[main], retired[auxiliary], retired[beta])
        exits = [
            event["kind"]["room"]["exit"]["reason"]
            for event in events
            if event.get("kind", {}).get("room", {}).get("exit", {}).get("tid") == root
        ]
        assert exits == [0xFFFF_FFFE], "root must take the production self-doom path"

    held = [i for i, event in enumerate(backend) if event["phase"] == "held"]
    assert len(held) == 1, "expected exactly one held guest read"
    index = held[0]
    request = backend[index]
    assert (request["offset"], request["length"]) == (0, 512), "not the guest sector"
    assert [event["phase"] for event in backend[index:index + 3]] == [
        "held", "resuming", "replied",
    ], "held read must receive a successful reply after the receipt"
    reply = backend[index + 2]
    assert backend[index + 1]["cookie"] == request["cookie"] == reply["cookie"]
    assert (reply["offset"], reply["length"], reply["error"]) == (0, 512, 0)
    assert all(event["phase"] == "disconnected" for event in backend[index + 3:])
    print(f"{mode}: alpha Team {main}/{auxiliary} reaped before Driver {beta}; frames {frames[1]} → {frames[2]}")


def check(console_path, events_path, capture_path, mode="ruin"):
    capture = Path(capture_path)
    events = [json.loads(line) for line in Path(events_path).read_text().splitlines()]
    held_events = [
        json.loads(line)
        for line in capture.with_suffix(".held-diagnose.jsonl").read_text().splitlines()
    ]
    assert events[:len(held_events)] == held_events, "held snapshot is not this trace's prefix"
    pending(capture.with_suffix(".held-console").read_text(), held_events, mode)
    backend = [
        json.loads(line)
        for line in capture.with_suffix(".block-events").read_text().splitlines()
    ]
    completed(Path(console_path).read_text(), events, backend, mode)


if __name__ == "__main__":
    check(*sys.argv[1:])
