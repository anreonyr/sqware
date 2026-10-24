"""Synthetic records for each transport confirmation boundary."""

from .ruin import CONSOLE as BASE


def evidence(step):
    party, op, target, boundary = {
        "invalidate": (3, 3, 7, "iommu: binding 1 invalidated"),
        "reset": (2, 3, 6, "pci: binding 1 disabling bus-master"),
        "release": (3, 5, 7, "iommu: binding 1 released"),
        "reference": (3, 5, 7, "iommu: binding 1 released"),
    }[step]
    code = 4 if step == "reference" else 0
    stage = {"invalidate": "Invalidate", "reset": "Disable",
             "release": "Release", "reference": "Release"}[step]
    if step == "reset":
        failure = (
            "confirmation: device stop gate armed from=8 pci=6 serial=1\n"
            "confirmation: transport rejected from=8 to=6 op=3 serial=1 references=2 ticks=3\n"
            "recovery: binding 1 stage Disable failed party=2 op=3 code=0\n"
            "ctl: confirmation failed party=2 op=3 code=0 state=Closing\n"
            "confirmation: injection removed hits=5 references=2 ticks=5\n"
        )
    else:
        rejected = (
            "confirmation: reference release rejected from=7 reference=777 pages=6 references=2 ticks=3\n"
            if step == "reference" else
            f"confirmation: transport rejected from=8 to={target} op={op} serial=1 references=2 ticks=3\n"
        )
        failure = (
            rejected
            + f"recovery: binding 1 stage {stage} failed party={party} op={op} code={code}\n"
            + f"ctl: confirmation failed party={party} op={op} code={code} state=Closing\n"
            + (
                "confirmation: failed reference still owned reference=777 pages=6\n"
                if step == "reference" else ""
            )
            + "confirmation: injection removed hits=5 references=2 ticks=5\n"
            + (
                "confirmation: reference released from=7 reference=777 references=1\n"
                if step == "reference" else ""
            )
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
        .replace(
            boundary,
            failure + boundary,
        )
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
