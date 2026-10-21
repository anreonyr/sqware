"""Check every real generation, DMA reply, stop chain and final reclamation."""

import json
import re
import sys
from pathlib import Path

from trace import check as execution
from ruin_check import one, reaps


def check(console_path, events_path):
    console = Path(console_path).read_text()
    events = [json.loads(line) for line in Path(events_path).read_text().splitlines()]
    assert events and events[-1].get("kind") == {"halt": "halt"}, "incomplete trace"
    execution(console_path, events_path, 128)
    generations = re.findall(
        r"^ctl: generation (\d+) serial=(\d+) settled fault=(true|false)$",
        console, re.MULTILINE,
    )
    assert generations == [
        (str(n), str(n), "true" if n % 2 == 0 else "false")
        for n in range(1, 129)
    ], "missing or repeated generation, serial, or retirement mode"
    assert len(re.findall(
        r"^ctl: served used=1 len=513 written_ok=true sector_ok=true status=0$",
        console, re.MULTILINE,
    )) == 128, "every generation must complete real DMA"
    requester = one(console, r"pci: space task=\d+ ecam=0x[0-9a-f]+ requester=(\d+) bar=\d+")[1]
    for n in range(1, 129):
        patterns = (
            rf"hub: binding {n} active \(business released\) iova=0x[0-9a-f]+",
            rf"hub: binding {n} execution domain stopped \(tid=\d+\)",
            rf"iommu: binding {n} invalidated frames held=6 \(not released\)",
            rf"pci: binding {n} disabling bus-master",
            rf"recovery: binding {n} stopped reset=1 status=0 queue_enable=0",
            rf"iommu: binding {n} queues stopped cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 faults=0 last_did=0",
            rf"iommu: binding {n} released ddtp=0x0 cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 frames=0 stopped=true",
            rf"recovery: binding {n} detached",
            rf"hub: binding {n} closed and released device {requester}",
            rf"ctl: generation {n} serial={n} settled fault=(?:true|false)",
        )
        positions = [one(console, pattern).start() for pattern in patterns]
        assert positions == sorted(positions), f"generation {n}: stop chain out of order"
    duration = int(one(console, r"ctl: soak generations=128 elapsed_ms=(\d+)")[1])
    assert duration > 10_000, "must cross the production idle deadline"
    assert "system: forced shutdown admitted" not in console, "active generations treated as idle"
    one(console, r"exit tid=\d+ reason=0x0 note: device-ctl-soak: done")
    one(console, r"exit tid=1 reason=0x0 note: system: done")
    frames = one(console, r"memory: frame_allocations before=(\d+) after=(\d+)")
    assert frames[1] == frames[2], "physical allocation objects leaked"
    retired = reaps(events)
    alphas = [
        int(tid) for tid in re.findall(
            r"^ctl: constructed device-work tid=(\d+) owner=hub$", console, re.MULTILINE,
        )
    ]
    auxiliaries = [
        int(tid) for tid in re.findall(
            r"^ctl: binding \d+ auxiliary=(\d+) alive=true$", console, re.MULTILINE,
        )
    ]
    assert len(set(alphas + auxiliaries)) == 256, "execution task identities reused"
    for service in ("hub", "pci", "iommu", "recovery"):
        task = int(one(console, rf"system: minted {service} tid=(\d+)")[1])
        assert retired[task] > max(retired[tid] for tid in alphas + auxiliaries), (
            f"{service} did not remain resident across all generations"
        )
    print(f"soak: 128 real generations, 64 normal + 64 owner deaths, {duration} ms; frames {frames[1]} → {frames[2]}")


if __name__ == "__main__":
    check(*sys.argv[1:])
