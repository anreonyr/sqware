"""Check injected transport failure, retained ownership and the same binding's retry."""

import json
import re
import sys
from pathlib import Path

from ruin_check import identity, one, reaps, tasks

STEPS = {
    "invalidate": (3, 3, "iommu", 1),
    "reset": (2, 3, "pci", 2),
    "release": (3, 5, "iommu", 6),
    "reference": (3, 5, "iommu", 6),
}


def completed(console, events, step):
    party, op, target, boundary = STEPS[step]
    main, auxiliary, beta = tasks(console)
    stage = {
        "invalidate": "Invalidate", "reset": "Disable",
        "release": "Release", "reference": "Release",
    }[step]
    services = {
        service: int(one(console, rf"system: minted {service} tid=(\d+)")[1])
        for service in ("hub", "pci", "iommu", "recovery", "device-ctl-confirmation")
    }
    _, key = identity(console)
    chain = [
        rf"hub: binding 1 execution domain stopped \(tid={main}\)",
        r"iommu: binding 1 invalidated frames held=6 \(not released\)",
        r"pci: binding 1 disabling bus-master",
        r"pci: endpoint bus-master off=0x0",
        r"recovery: binding 1 stopped reset=1 status=0 queue_enable=0",
        r"iommu: binding 1 queues stopped cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 faults=0 last_did=0",
        r"iommu: binding 1 released ddtp=0x0 cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 frames=0 stopped=true",
        r"recovery: binding 1 detached",
        rf"hub: binding 1 closed and released device {key}",
    ]
    opened = one(console, r"recovery: binding 1 active iova=0x[0-9a-f]+ execution=\d+").start()
    served = one(console,
        r"ctl: served used=1 len=513 written_ok=true sector_ok=true status=0").start()

    if step == "reset":
        armed = one(console,
            rf"confirmation: device stop gate armed from={services['recovery']} "
            rf"pci={services['pci']} serial=1")
        rejected = one(console,
            rf"confirmation: transport rejected from={services['recovery']} "
            rf"to={services['pci']} op=3 serial=1 references=2 ticks=(\d+)")
        failure = one(console,
            r"recovery: binding 1 stage Disable failed party=2 op=3 code=0")
        controller = one(console,
            r"ctl: confirmation failed party=2 op=3 code=0 state=Closing")
        removed = one(console,
            r"confirmation: injection removed hits=(\d+) references=2 ticks=(\d+)")
        injected_tick = int(rejected[1])
        positions = [
            opened,
            served,
            *[one(console, pattern).start() for pattern in chain[:boundary]],
            armed.start(),
            rejected.start(),
            failure.start(),
            controller.start(),
            removed.start(),
            *[one(console, pattern).start() for pattern in chain[boundary:]],
            one(console, r"ctl: confirmation retry settled; original failure preserved").start(),
            one(console, r"exit tid=\d+ reason=0x0 note: device-ctl-confirmation: done").start(),
            one(console, r"exit tid=\d+ reason=0x0 note: iommu-confirmation: done").start(),
            one(console, r"task: all tasks exited, system halted").start(),
        ]
        assert armed.start() < rejected.start() < failure.start() < controller.start() < removed.start(), (
            "PCI Disable gate/failure order is not causal"
        )
    else:
        if step == "reference":
            rejected = one(
                console,
                rf"confirmation: reference release rejected from={services['iommu']} "
                r"reference=(\d+) pages=6 references=2 ticks=(\d+)",
            )
            assert int(rejected[1]) > 0, "missing real reference handle"
            retained = one(
                console,
                rf"confirmation: failed reference still owned reference={rejected[1]} pages=6",
            )
            released = one(
                console,
                rf"confirmation: reference released from={services['iommu']} "
                rf"reference={rejected[1]} references=1",
            )
            injected_tick = int(rejected[2])
            code = 4
        else:
            rejected = one(
                console,
                rf"confirmation: transport rejected from={services['recovery']} "
                rf"to={services[target]} op={op} serial=1 references=2 ticks=(\d+)",
            )
            injected_tick = int(rejected[1])
            code = 0
        removed = one(console, r"confirmation: injection removed hits=(\d+) references=2 ticks=(\d+)")
        positions = [
            opened,
            served,
            *[one(console, pattern).start() for pattern in chain[:boundary]],
            rejected.start(),
            one(console,
                rf"recovery: binding 1 stage {stage} failed party={party} op={op} code={code}"
            ).start(),
            one(console,
                rf"ctl: confirmation failed party={party} op={op} code={code} state=Closing"
            ).start(),
            removed.start(),
            *([released.start()] if step == "reference" else []),
            *[one(console, pattern).start() for pattern in chain[boundary:]],
            one(console, r"ctl: confirmation retry settled; original failure preserved").start(),
            one(console, r"exit tid=\d+ reason=0x0 note: device-ctl-confirmation: done").start(),
            one(console, r"exit tid=\d+ reason=0x0 note: iommu-confirmation: done").start(),
            one(console, r"task: all tasks exited, system halted").start(),
        ]
        if step == "reference":
            assert rejected.start() < retained.start() < removed.start(), (
                "reference lost before removal"
            )

    assert int(removed[1]) > 0, "no actual injection"
    assert injected_tick < int(removed[2]), "invalid injection interval"
    assert positions == sorted(positions), "stop chain advanced without confirmation"
    one(console, rf"ctl: binding 1 auxiliary={auxiliary} alive=false")
    frames = one(console, r"memory: frame_allocations before=(\d+) after=(\d+)")
    assert frames[1] == frames[2], "physical allocation objects leaked"
    assert events and events[-1].get("kind") == {"halt": "halt"}, "incomplete trace"
    retired = reaps(events)
    assert max(retired[main], retired[auxiliary]) < injected_tick, "alpha Team not stopped"
    for service, task in services.items():
        assert retired[task] > int(removed[2]), f"{service} retired while confirmation unavailable"
    assert beta == services["recovery"]
    assert not re.search(r"^exit tid=\d+ reason=0xffffffff", console, re.MULTILINE), "fixture failed"
    assert "user fault killed:" not in console, "unexpected native fault"
    cause = (
        "PCI Disable transport failure" if step == "reset"
        else "Reference Release Busy" if step == "reference"
        else "transport failure"
    )
    print(f"confirmation {step}: real {cause}, 2 IOMMU references retained; same binding retried; frames {frames[1]} → {frames[2]}")


if __name__ == "__main__":
    completed(
        Path(sys.argv[1]).read_text(),
        [json.loads(line) for line in Path(sys.argv[2]).read_text().splitlines()],
        sys.argv[3],
    )
