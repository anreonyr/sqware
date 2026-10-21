"""Synthetic records for each transport confirmation boundary."""

from .ruin import CONSOLE as BASE


def evidence(step):
    party, op, target, boundary = {
        "invalidate": (3, 3, 7, "iommu: binding 1 invalidated"),
        "reset": (4, 2, 8, "recovery: binding 1 stopped"),
        "release": (3, 5, 7, "iommu: binding 1 released"),
    }[step]
    failure = (
        f"confirmation: transport rejected from=4 to={target} op={op} serial=1 references=2 ticks=3\n"
        f"hub: binding 1 step failed party={party} op={op} code=0 (holding)\n"
        f"ctl: confirmation failed party={party} op={op} code=0 state=Closing\n"
        "confirmation: injection removed hits=5 references=2 ticks=5\n"
    )
    console = (
        "system: minted hub tid=4\n"
        "system: minted pci tid=6\n"
        "system: minted iommu tid=7\n"
        "system: minted device-ctl-confirmation tid=9\n"
        + BASE.replace("ruin: requested recovery\n", "")
        .replace(
            "work: rang the in-flight request doorbell\n",
            "work: rang the doorbell\n"
            "ctl: served used=1 len=513 written_ok=true sector_ok=true status=0\n",
        )
        .replace(boundary, failure + boundary)
        .replace(
            "ctl: in-flight binding settled by static Driver Ruin\n"
            "exit tid=9 reason=0x0 note: device-ctl-ruin: done\n"
            "ruin: recovery dead after reply\n"
            "exit tid=1 reason=0x0 note: iommu-ruin: done\n",
            "ctl: confirmation retry settled; original failure preserved\n"
            "exit tid=9 reason=0x0 note: device-ctl-confirmation: done\n"
            "exit tid=1 reason=0x0 note: iommu-confirmation: done\n",
        )
    )
    events = [
        {"when": when, "kind": {"room": {"reap": {"tid": tid}}}}
        for when, tid in ((1, 10), (2, 11), (10, 4), (11, 6), (12, 7), (13, 8), (14, 9))
    ]
    events.append({"when": 15, "kind": {"halt": "halt"}})
    return console, events
