"""Verify two real PCI requesters, Domain-qualified IOVA and independent DMA retirement."""

import re
import sys
from pathlib import Path

from ruin_check import one
from trace import check as executions


def check(console_path, events_path, generations=1):
    generations = int(generations)
    assert generations in (1, 1024), "unsupported dual-device generation count"
    console = Path(console_path).read_text()
    assert not re.search(r"user fault killed:|\[ERROR\]|note: (?:work|dual|ctl):", console), "native fault is not successful retirement"
    retired, last = executions(console_path, events_path, 2 * generations)
    assert last == {"halt": "halt"}, "incomplete execution trace"
    ready = re.findall(
        r"^virtio-blk: ready hub=(\d+) pci=(\d+) requester=(\d+) key=(\d+)$",
        console, re.MULTILINE,
    )
    assert len(ready) == 2, "both physical devices must enroll"
    assert len({row[0] for row in ready}) == len({row[1] for row in ready}) == 1
    assert len({row[2] for row in ready}) == len({row[3] for row in ready}) == 2
    assert all(0 <= int(row[2]) < 64 and int(row[3]) > 0 for row in ready)
    pci = one(console, r"pci: space task=(\d+) ecam=0x[0-9a-f]+ functions=\d+")
    assert all(row[1] == pci[1] for row in ready), "foreign PCI provider"
    domains = re.findall(
        r"^iommu: domain (\d+) established owner=(\d+) requester=(\d+)$",
        console, re.MULTILINE,
    )
    assert len(domains) == 2 and len({row[0] for row in domains}) == 2
    assert len({row[1] for row in domains}) == 1
    driver = one(console, r"system: minted virtio-blk tid=(\d+)")[1]
    assert all(row[1] == driver for row in domains), "Domain owner is not the actual Driver"
    assert {row[2] for row in domains} == {row[2] for row in ready}
    windows = one(console, r"dual: windows a=0x([0-9a-f]+) b=0x([0-9a-f]+) bytes=24576")
    a_window, b_window = (int(windows[n], 16) for n in (1, 2))
    assert abs(a_window - b_window) >= 24576, "controller DMA views overlap"
    bars = re.findall(
        r"^pci: endpoint requester=(\d+) bar=(\d+) address=0x([0-9a-f]+) size=0x([0-9a-f]+) bus-master off=0x0$",
        console, re.MULTILINE,
    )
    by_requester = {}
    for requester, bar, base, size in bars:
        value = (int(bar), int(base, 16), int(size, 16))
        assert by_requester.setdefault(requester, value) == value, "cached BAR changed"
    assert set(by_requester) == {row[2] for row in ready}
    (_, left, left_size), (_, right, right_size) = by_requester.values()
    assert left + left_size <= right or right + right_size <= left, "physical BAR windows overlap"

    active = list(re.finditer(
        r"^dual: generation (\d+) active a=(\d+) b=(\d+) iova=0x([0-9a-f]+)$",
        console, re.MULTILINE,
    ))
    settled = list(re.finditer(
        r"^dual: generation (\d+) settled a=(\d+) b=(\d+) capabilities=(\d+) launch=1 command=2 a_fault=(true|false) b_fault=(true|false)$",
        console, re.MULTILINE,
    ))
    assert len(active) == len(settled) == generations
    baseline = None
    previous_iova = None
    for generation, (opening, closing) in enumerate(zip(active, settled), 1):
        a, b = 2 * generation - 1, 2 * generation
        assert tuple(map(int, opening.groups()[:3])) == (generation, a, b)
        assert tuple(map(int, closing.groups()[:3])) == (generation, a, b)
        iova = int(opening[4], 16)
        assert iova > 0 and iova % 4096 == 0
        assert previous_iova is None or iova > previous_iova, "retired IOVA reused"
        previous_iova = iova
        count = int(closing[4])
        assert count == 17, "unexpected controller capability count"
        baseline = count if baseline is None else baseline
        assert count == baseline, "controller capability table grew"
        assert closing[5] == ("true" if generation % 2 == 0 else "false")
        assert closing[6] == ("false" if generation % 2 == 0 else "true")
        assert opening.end() < closing.start()
        if generation < generations:
            assert closing.end() < active[generation].start()
        mappings = []
        for serial in (a, b):
            mapping = one(console, rf"iommu: domain (\d+) binding {serial} owner=(\d+) requester=(\d+) iova=0x([0-9a-f]+) bytes=24576")
            assert int(mapping[4], 16) == iova, "equal IOVA must come from actual allocations"
            assert (mapping[1], mapping[2], mapping[3]) in domains
            assert mapping.end() < opening.start()
            mappings.append(mapping)
        assert mappings[0][1] != mappings[1][1] and mappings[0][3] != mappings[1][3]
        keys = {row[2]: row[3] for row in ready}
        a_key, b_key = keys[mappings[0][3]], keys[mappings[1][3]]
        segment = console[opening.end():closing.start()]
        ordered = [
            rf"dual: read serial={a} pattern=0 round=0 used=1 len=513 sector_ok=true status=0",
            rf"dual: read serial={b} pattern=165 round=0 used=1 len=513 sector_ok=true status=0",
            rf"dual: isolated a={a} b={b} a_preserved=true",
            rf"work: binding {a} " + ("exiting without Close" if generation % 2 == 0 else r"requesting Close::Normal"),
            rf"hub: binding {a} execution domain stopped \(tid=\d+\)",
            rf"iommu: binding {a} invalidated frames held=6 \(not released\)",
            rf"pci: binding {a} disabling bus-master",
            rf"virtio-blk: binding {a} stopped reset=1 status=0 queue_enable=0",
            rf"iommu: binding {a} quiesced shared engine on .* engine_off=false",
            rf"iommu: binding {a} released shared .* frames=0 engine_off=false",
            rf"virtio-blk: binding {a} detached",
            rf"hub: binding {a} closed and released device {a_key}",
            rf"dual: read serial={b} pattern=165 round=1 used=2 len=513 sector_ok=true status=0",
            rf"work: binding {b} " + (r"requesting Close::Normal" if generation % 2 == 0 else "exiting without Close"),
            rf"hub: binding {b} execution domain stopped \(tid=\d+\)",
            rf"iommu: binding {b} invalidated frames held=6 \(not released\)",
            rf"pci: binding {b} disabling bus-master",
            rf"virtio-blk: binding {b} stopped reset=1 status=0 queue_enable=0",
            rf"iommu: binding {b} queues stopped cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 faults=0 last_did=0",
            rf"iommu: binding {b} released ddtp=0x0 cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 frames=0 stopped=true",
            rf"virtio-blk: binding {b} detached",
            rf"hub: binding {b} closed and released device {b_key}",
        ]
        positions = [one(segment, pattern).start() for pattern in ordered]
        assert positions == sorted(positions), f"generation {generation}: close isolation/order violated"
    assert len(re.findall(r"^dual: read serial=", console, re.MULTILINE)) == 3 * generations
    one(console, rf"dual: completed generations={generations}")
    one(console, r"exit tid=\d+ reason=0x0 note: device-ctl-dual: done")
    one(console, r"exit tid=1 reason=0x0 note: system: done")
    # These are cumulative cull-branch counters, not live-task gauges.
    branches = one(console, r"doom: held=(\d+) starved=(\d+) blocked=(\d+) nudged=(\d+)")
    assert sum(map(int, branches.groups())) > 0, "no actual domain culling observed"
    frames = one(console, r"memory: frame_allocations before=(\d+) after=(\d+)")
    assert frames[1] == frames[2], "physical allocations did not return to baseline"
    work_tasks = set(map(int, re.findall(r"^ctl: constructed device-work tid=(\d+) owner=hub$", console, re.MULTILINE)))
    auxiliaries = set(map(int, re.findall(r"^ctl: binding \d+ auxiliary=(\d+) alive=true$", console, re.MULTILINE)))
    assert len(work_tasks | auxiliaries) == 4 * generations
    for task in map(int, re.findall(r"^system: minted \S+ tid=(\d+)$", console, re.MULTILINE)):
        assert task in retired, f"static task {task} was not reaped"
    for service in ("hub", "pci", "iommu", "virtio-blk"):
        task = int(one(console, rf"system: minted {service} tid=(\d+)")[1])
        assert retired[task] > max(retired[tid] for tid in work_tasks | auxiliaries)
    assert 1 in retired and retired[1] == max(retired.values()), "root retired before all owned tasks"
    print(f"dual: {generations} pairs, independent physical BARs/Domains, equal IOVA, {3 * generations} real reads; frames {frames[1]} → {frames[2]}")


if __name__ == "__main__":
    check(*sys.argv[1:])
