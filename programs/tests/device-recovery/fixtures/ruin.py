"""Synthetic console, kernel trace and backend records for checker tests."""

CONSOLE = """boot: entry task=1
system: minted recovery tid=8
ctl: constructed device-work tid=10 owner=hub
ctl: binding 1 auxiliary=11 alive=true
pci: space task=7 ecam=0x29000 requester=8 bar=4
iommu: binding 1 published requester=8 held=6 ddtp=0x1234
pci: endpoint bus-master on=0x4
hub: binding 1 active (business released) iova=0x200200000
work: rang the in-flight request doorbell
ruin: requested recovery
hub: binding 1 execution domain stopped (tid=10)
iommu: binding 1 invalidated frames held=6 (not released)
pci: binding 1 disabling bus-master
pci: endpoint bus-master off=0x0
recovery: binding 1 stopped reset=1 status=0 queue_enable=0
iommu: binding 1 queues stopped cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 faults=0 last_did=0
iommu: binding 1 released ddtp=0x0 cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 frames=0 stopped=true
recovery: binding 1 detached
hub: binding 1 closed and released device 8
ctl: drained read sector_ok=true status=0 used=0 len=0
ctl: binding 1 auxiliary=11 alive=false
ctl: in-flight binding settled by static Driver Ruin
exit tid=9 reason=0x0 note: device-ctl-ruin: done
ruin: recovery dead after reply
exit tid=1 reason=0x0 note: iommu-ruin: done
task: all tasks exited, system halted
memory: frame_allocations before=85 after=85
"""
HELD = CONSOLE.split("pci: endpoint bus-master off=0x0")[0]
EVENTS = [
    {"h": 0, "when": 1, "kind": {"room": {"reap": {"tid": 10}}}},
    {"h": 1, "when": 2, "kind": {"room": {"reap": {"tid": 11}}}},
    {"h": 0, "when": 3, "kind": {"room": {"reap": {"tid": 8}}}},
    {"h": 0, "when": 4, "kind": {"halt": "halt"}},
]
BACKEND = [
    {"phase": "ready"},
    {"phase": "boot-read", "cookie": 1, "offset": 0, "length": 512},
    {"phase": "replied", "cookie": 1, "offset": 0, "length": 512, "error": 0},
    {"phase": "held", "cookie": 1, "offset": 0, "length": 512},
    {"phase": "resuming", "cookie": 1},
    {"phase": "replied", "cookie": 1, "offset": 0, "length": 512, "error": 0},
    {"phase": "disconnected"},
]
