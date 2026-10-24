"""Self-contained reset-readback evidence shaped after the verified native run.

The failed snapshot is a strict prefix of the full execution trace; the doctored
status/queue reads and the real block-hold sequence are explicit records.
"""

import copy
import json

FAILURE = "virtio-blk: binding 1 stage Stop failed party=4 op=2 code=4"

DTS = """
/ {
  soc {
    pci@30000000 {
      compatible = "pci-host-ecam-generic";
      reg = <0x0 0x30000000 0x0 0x10000000>;
    };
  };
};
"""

FAILED_CONSOLE = """boot: entry task=1
system: minted hub tid=4
system: minted iommu tid=5
system: minted pci tid=6
system: minted virtio-blk tid=7
system: minted device-ctl-fault tid=8
pci: space task=6 ecam=0x2c000 functions=65536
virtio-blk: ready hub=4 pci=6 requester=8 key=1
ctl: constructed device-work tid=9 owner=hub
ctl: binding 1 auxiliary=10 alive=true
pci: endpoint requester=8 bar=4 address=0x400000000 size=0x4000 bus-master off=0x0
iommu: binding 1 invalidated frames held=6 (not released)
pci: binding 1 disabling bus-master
""" + FAILURE + "\n"

CONSOLE = """boot: entry task=1
system: minted hub tid=4
system: minted iommu tid=5
system: minted pci tid=6
system: minted virtio-blk tid=7
system: minted device-ctl-fault tid=8
pci: space task=6 ecam=0x2c000 functions=65536
virtio-blk: ready hub=4 pci=6 requester=8 key=1
ctl: constructed device-work tid=9 owner=hub
ctl: binding 1 auxiliary=10 alive=true
pci: endpoint requester=8 bar=4 address=0x400000000 size=0x4000 bus-master off=0x0
iommu: binding 1 invalidated frames held=6 (not released)
pci: binding 1 disabling bus-master
virtio-blk: binding 1 stage Stop failed party=4 op=2 code=4
virtio-blk: binding 1 stopped reset=1 status=0 queue_enable=0
iommu: binding 1 queues stopped cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 faults=0 last_did=0
iommu: binding 1 released ddtp=0x0 cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 frames=0 stopped=true
virtio-blk: binding 1 detached
virtio-blk: binding 1 closed by coordinator
hub: binding 1 closed and released device 1
ctl: binding 1 auxiliary=10 alive=false
ctl: closed binding 1 failure party=4 op=2 code=4
ctl: constructed device-work tid=11 owner=hub
ctl: binding 2 auxiliary=12 alive=true
virtio-blk: binding 2 active iova=0x200200000 execution=11
ctl: stale iova submitted sentinel_held=true status_held=true
hub: binding 2 execution domain stopped (tid=11)
iommu: binding 2 invalidated frames held=6 (not released)
pci: binding 2 disabling bus-master
virtio-blk: binding 2 stopped reset=1 status=0 queue_enable=0
iommu: binding 2 queues stopped cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 faults=127 last_did=8
iommu: binding 2 released ddtp=0x0 cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 frames=0 stopped=true
virtio-blk: binding 2 detached
virtio-blk: binding 2 closed by coordinator
hub: binding 2 closed and released device 1
ctl: binding 2 auxiliary=12 alive=false
exit tid=8 reason=0x0 note: device-ctl: done
system: retired name=device-ctl-fault task=8 team=7
system: retired name=virtio-blk task=7 team=6
system: retired name=iommu task=5 team=4
system: retired name=pci task=6 team=5
system: retired name=hub task=4 team=3
exit tid=1 reason=0x0 note: system: done
task: all tasks exited, system halted
doom: held=1 starved=1 blocked=4 nudged=0
memory: frame_allocations before=153 after=153
"""


def _event(hart, when, room):
    return {"h": hart, "when": when, "kind": {"room": room}}


PREFIX = [
    _event(0, 1, {"wait": {"tid": 9}}),
    _event(1, 2, {"wait": {"tid": 10}}),
    _event(0, 5, {"reap": {"tid": 9}}),
    _event(1, 6, {"reap": {"tid": 10}}),
]
EVENTS = PREFIX + [
    _event(0, 7, {"wait": {"tid": 11}}),
    _event(1, 8, {"wait": {"tid": 12}}),
    _event(0, 9, {"reap": {"tid": 11}}),
    _event(1, 10, {"reap": {"tid": 12}}),
    _event(0, 12, {"reap": {"tid": 4}}),
    _event(0, 13, {"reap": {"tid": 5}}),
    _event(0, 14, {"reap": {"tid": 6}}),
    _event(0, 15, {"reap": {"tid": 7}}),
    _event(0, 16, {"reap": {"tid": 8}}),
    _event(0, 20, {"reap": {"tid": 1}}),
    {"h": 0, "when": 21, "kind": {"halt": "halt"}},
]

STATUS = "0x400000014"
QUEUE = "0x40000001c"

OPERATIONS = [
    {"request": "readw 0x30008000", "reply": "OK 0x1af4"},
    {"request": "readw 0x30008002", "reply": "OK 0x1042"},
    {"request": "readl 0x30008020", "reply": "OK 0xc"},
    {"request": "readl 0x30008024", "reply": "OK 0x4"},
    {"request": "readb 0x30008034", "reply": "OK 0x40"},
    {"request": "readb 0x30008040", "reply": "OK 0x9"},
    {"request": "readb 0x30008041", "reply": "OK 0x0"},
    {"request": "readb 0x30008043", "reply": "OK 0x1"},
    {"request": "readb 0x30008044", "reply": "OK 0x4"},
    {"request": "readl 0x30008048", "reply": "OK 0x0"},
    {"request": f"readb {STATUS}", "reply": "OK 0xf"},
    {"request": f"writeb {STATUS} 0x1", "reply": "OK"},
    {"request": f"writeb {STATUS} 0x1", "reply": "OK"},
    {"request": f"readb {STATUS}", "reply": "OK 0x1"},
    {"request": f"readw {QUEUE}", "reply": "OK 0x0"},
    {"request": f"writeb {STATUS} 0x0", "reply": "OK"},
    {"request": f"readb {STATUS}", "reply": "OK 0x0"},
    {"request": f"readw {QUEUE}", "reply": "OK 0x0"},
]

READBACK = {
    "operations": OPERATIONS,
    "requester": 8,
    "bar": 4,
    "base": 0x400000000,
    "size": 0x4000,
    "ecam": 0x30000000,
    "ecam_size": 0x10000000,
    "common": 0,
    "status_address": 0x400000014,
    "queue_address": 0x40000001c,
    "initial_status": 15,
    "failed_status": 1,
    "failed_queue": 0,
    "repaired_status": 0,
    "repaired_queue": 0,
}

BLOCK_EVENTS = [
    {"phase": "ready"},
    {"phase": "boot-read", "cookie": 1, "offset": 0, "length": 512},
    {"phase": "replied", "cookie": 1, "offset": 0, "length": 512, "error": 0},
    {"phase": "held", "cookie": 1, "offset": 0, "length": 512},
    {"phase": "resuming", "cookie": 1},
    {"phase": "replied", "cookie": 1, "offset": 0, "length": 512, "error": 0},
    {"phase": "replied", "cookie": 1, "offset": 0, "length": 512, "error": 0},
    {"phase": "disconnected"},
]


def proof():
    return {
        "console": CONSOLE,
        "failed": FAILED_CONSOLE,
        "events": copy.deepcopy(EVENTS),
        "prefix": copy.deepcopy(PREFIX),
        "readback": copy.deepcopy(READBACK),
        "dts": DTS,
        "block": copy.deepcopy(BLOCK_EVENTS),
    }


def lines(records):
    return "".join(json.dumps(record, separators=(",", ":")) + "\n" for record in records)
