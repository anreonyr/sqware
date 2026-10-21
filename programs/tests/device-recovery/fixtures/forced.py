"""Synthetic forced-shutdown evidence, including the production self-doom."""

from .ruin import CONSOLE as RUIN

CONSOLE = (
    "system: minted hub tid=4\n"
    "system: minted pci tid=6\n"
    "system: minted iommu tid=7\n"
    "system: minted device-ctl-forced tid=9\n"
    + RUIN.replace(
        "ruin: requested recovery\n",
        "ctl: awaiting forced global shutdown with in-flight read\n"
        "system: idle deadline reached; awaiting DMA settlement\n",
    ).replace(
        "ctl: in-flight binding settled by static Driver Ruin\n"
        "exit tid=9 reason=0x0 note: device-ctl-ruin: done\n"
        "ruin: recovery dead after reply\n"
        "exit tid=1 reason=0x0 note: iommu-ruin: done\n",
        "ctl: drained read verified; awaiting global retirement\n"
        "system: forced shutdown admitted after DMA settlement\n"
        "system: frame Step(Idle)\n",
    )
)
HELD = CONSOLE.split("pci: endpoint bus-master off=0x0")[0]
EVENTS = [
    {"h": 0, "when": 1, "kind": {"room": {"reap": {"tid": 10}}}},
    {"h": 1, "when": 2, "kind": {"room": {"reap": {"tid": 11}}}},
    *[
        {"h": 0, "when": when, "kind": {"room": {"reap": {"tid": tid}}}}
        for when, tid in enumerate((9, 7, 8, 6, 4), 3)
    ],
    {"h": 0, "when": 8, "kind": {"room": {"exit": {"tid": 1, "reason": 0xFFFF_FFFE}}}},
    {"h": 0, "when": 9, "kind": {"room": {"reap": {"tid": 1}}}},
    {"h": 0, "when": 10, "kind": {"halt": "halt"}},
]
