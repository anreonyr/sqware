"""Self-contained dual-device evidence shaped after the verified native run.

Cumulative doom counters are deliberately nonzero: they are cull-branch counters,
not a live-task gauge.
"""

import copy
import json

CONSOLE = """boot: entry task=1
system: minted hub tid=4
system: minted iommu tid=5
system: minted pci tid=6
system: minted virtio-blk tid=7
system: minted device-ctl-dual tid=8
pci: space task=6 ecam=0x2c000 functions=65536
iommu: domain 1 established owner=7 requester=8
virtio-blk: ready hub=4 pci=6 requester=8 key=1
iommu: domain 2 established owner=7 requester=16
virtio-blk: ready hub=4 pci=6 requester=16 key=2
virtio-blk: serving hub=4 pci=6 devices=2
ctl: constructed device-work tid=9 owner=hub
ctl: binding 1 auxiliary=10 alive=true
pci: endpoint requester=8 bar=4 address=0x400000000 size=0x4000 bus-master off=0x0
iommu: domain 1 binding 1 owner=7 requester=8 iova=0x200000000 bytes=24576
pci: endpoint requester=16 bar=4 address=0x400004000 size=0x4000 bus-master off=0x0
iommu: domain 2 binding 2 owner=7 requester=16 iova=0x200000000 bytes=24576
ctl: constructed device-work tid=11 owner=hub
ctl: binding 2 auxiliary=12 alive=true
dual: windows a=0x33000 b=0x39000 bytes=24576
dual: generation 1 active a=1 b=2 iova=0x200000000
dual: read serial=1 pattern=0 round=0 used=1 len=513 sector_ok=true status=0
dual: read serial=2 pattern=165 round=0 used=1 len=513 sector_ok=true status=0
dual: isolated a=1 b=2 a_preserved=true
work: binding 1 requesting Close::Normal
hub: binding 1 execution domain stopped (tid=9)
iommu: binding 1 invalidated frames held=6 (not released)
pci: binding 1 disabling bus-master
virtio-blk: binding 1 stopped reset=1 status=0 queue_enable=0
iommu: binding 1 quiesced shared engine on cqcsr=0x10001 fqcsr=0x10001 pqcsr=0x0 queue_enable=1 faults=0 last_did=0 engine_off=false
iommu: binding 1 released shared ddtp=0x23fd6002 cqcsr=0x10001 fqcsr=0x10001 pqcsr=0x0 queue_enable=1 frames=0 engine_off=false
virtio-blk: binding 1 detached
hub: binding 1 closed and released device 1
ctl: binding 1 auxiliary=10 alive=false
dual: read serial=2 pattern=165 round=1 used=2 len=513 sector_ok=true status=0
work: binding 2 exiting without Close
hub: binding 2 execution domain stopped (tid=11)
iommu: binding 2 invalidated frames held=6 (not released)
pci: binding 2 disabling bus-master
virtio-blk: binding 2 stopped reset=1 status=0 queue_enable=0
iommu: binding 2 queues stopped cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 faults=0 last_did=0
iommu: binding 2 released ddtp=0x0 cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 frames=0 stopped=true
virtio-blk: binding 2 detached
hub: binding 2 closed and released device 2
ctl: binding 2 auxiliary=12 alive=false
dual: generation 1 settled a=1 b=2 capabilities=17 launch=1 command=2 a_fault=false b_fault=true
dual: completed generations=1
exit tid=8 reason=0x0 note: device-ctl-dual: done
system: retired name=device-ctl-dual task=8 team=7
system: retired name=virtio-blk task=7 team=6
system: retired name=iommu task=5 team=4
system: retired name=pci task=6 team=5
system: retired name=hub task=4 team=3
exit tid=1 reason=0x0 note: system: done
task: all tasks exited, system halted
doom: held=1 starved=0 blocked=5 nudged=0
memory: frame_allocations before=85 after=85
"""


def _event(hart, when, room):
    return {"h": hart, "when": when, "kind": {"room": room}}


EVENTS = [
    _event(0, 1, {"wait": {"tid": 9}}),
    _event(1, 2, {"wait": {"tid": 10}}),
    _event(0, 3, {"wait": {"tid": 11}}),
    _event(1, 4, {"wait": {"tid": 12}}),
    _event(0, 5, {"reap": {"tid": 9}}),
    _event(1, 6, {"reap": {"tid": 10}}),
    _event(0, 7, {"reap": {"tid": 11}}),
    _event(1, 8, {"reap": {"tid": 12}}),
    _event(0, 10, {"reap": {"tid": 4}}),
    _event(0, 11, {"reap": {"tid": 5}}),
    _event(0, 12, {"reap": {"tid": 6}}),
    _event(0, 13, {"reap": {"tid": 7}}),
    _event(0, 14, {"reap": {"tid": 8}}),
    _event(0, 20, {"reap": {"tid": 1}}),
    {"h": 0, "when": 21, "kind": {"halt": "halt"}},
]


def proof():
    return {"console": CONSOLE, "events": copy.deepcopy(EVENTS)}


def lines(events):
    return "".join(json.dumps(event, separators=(",", ":")) + "\n" for event in events)
