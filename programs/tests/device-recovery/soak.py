"""Check every real generation, DMA reply, stop chain and final reclamation."""

import re
import sys
from pathlib import Path

from trace import check as execution
from ruin_check import identity, one


def check(console_path, events_path, expected=128):
    expected = int(expected)
    assert expected in (128, 1024), "unsupported generation count"
    console = Path(console_path).read_text()
    retired, last = execution(console_path, events_path, expected)
    assert last == {"halt": "halt"}, "incomplete trace"
    receipts = list(re.finditer(
        r"^ctl: generation (\d+) serial=(\d+) settled fault=(true|false)$",
        console, re.MULTILINE,
    ))
    assert [receipt.groups() for receipt in receipts] == [
        (str(n), str(n), "true" if n % 2 == 0 else "false")
        for n in range(1, expected + 1)
    ], "missing or repeated generation, serial, or retirement mode"
    assert len(re.findall(
        r"^ctl: served used=1 len=513 written_ok=true sector_ok=true status=0$",
        console, re.MULTILINE,
    )) == expected, "every generation must complete real DMA"
    _, key = identity(console)
    start = 0
    capabilities = None
    for n, receipt in enumerate(receipts, 1):
        segment = console[start:receipt.end()]
        start = receipt.end()
        assert all(int(serial) == n for serial in re.findall(
            r"^(?:hub|pci|iommu|recovery): binding (\d+) ", segment, re.MULTILINE,
        )), f"generation {n}: another binding crossed its interval"
        patterns = (
            rf"recovery: binding {n} active iova=0x[0-9a-f]+ execution=\d+",
            r"ctl: served used=1 len=513 written_ok=true sector_ok=true status=0",
            rf"hub: binding {n} execution domain stopped \(tid=\d+\)",
            rf"iommu: binding {n} invalidated frames held=6 \(not released\)",
            rf"pci: binding {n} disabling bus-master",
            rf"recovery: binding {n} stopped reset=1 status=0 queue_enable=0",
            rf"iommu: binding {n} queues stopped cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 faults=0 last_did=0",
            rf"iommu: binding {n} released ddtp=0x0 cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 frames=0 stopped=true",
            rf"recovery: binding {n} detached",
            rf"hub: binding {n} closed and released device {key}",
            rf"ctl: generation {n} capabilities=\d+ launch=1 command=1",
            rf"ctl: generation {n} serial={n} settled fault=(?:true|false)",
        )
        positions = [one(segment, pattern).start() for pattern in patterns]
        assert positions == sorted(positions), f"generation {n}: stop chain out of order"
        count = int(one(segment, rf"ctl: generation {n} capabilities=(\d+) launch=1 command=1")[1])
        if capabilities is None:
            capabilities = count
        assert count == capabilities, f"generation {n}: capability table grew"
    assert not re.search(
        r"^(?:hub|pci|iommu|recovery): binding \d+ ", console[start:], re.MULTILINE,
    ), "binding progressed after the final generation"
    duration = int(one(console, rf"ctl: soak generations={expected} elapsed_ms=(\d+)")[1])
    assert duration > 10_000, "must cross the production idle deadline"
    assert "system: forced shutdown admitted" not in console, "active generations treated as idle"
    one(console, r"exit tid=\d+ reason=0x0 note: device-ctl-soak: done")
    one(console, r"exit tid=1 reason=0x0 note: system: done")
    frames = one(console, r"memory: frame_allocations before=(\d+) after=(\d+)")
    assert frames[1] == frames[2], "physical allocation objects leaked"
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
    assert len(set(alphas + auxiliaries)) == 2 * expected, "execution task identities reused"
    for service in ("hub", "pci", "iommu", "recovery"):
        task = int(one(console, rf"system: minted {service} tid=(\d+)")[1])
        assert retired[task] > max(retired[tid] for tid in alphas + auxiliaries), (
            f"{service} did not remain resident across all generations"
        )
    print(f"soak: {expected} real generations, {expected // 2} normal + {expected // 2} owner deaths, {duration} ms; frames {frames[1]} → {frames[2]}")


if __name__ == "__main__":
    check(*sys.argv[1:])
