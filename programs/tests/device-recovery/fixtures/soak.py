"""Synthetic complete generations for validating the bounded and long-run checker."""


def evidence(count):
    lines = [
        "system: minted hub tid=4",
        "system: minted pci tid=6",
        "system: minted iommu tid=7",
        "system: minted virtio-blk tid=8",
        "pci: space task=6 ecam=0x29000 functions=1",
        "pci: ready functions=1",
        "virtio-blk: ready hub=4 pci=6 requester=8 key=41",
    ]
    events = []
    for serial in range(1, count + 1):
        main, auxiliary = 20 + serial * 2, 21 + serial * 2
        lines.extend((
            f"ctl: constructed device-work tid={main} owner=hub",
            f"ctl: binding {serial} auxiliary={auxiliary} alive=true",
            f"virtio-blk: binding {serial} active iova={0x200000000 + serial * 0x200000:#x} execution={main}",
            "ctl: served used=1 len=513 written_ok=true sector_ok=true status=0",
            f"hub: binding {serial} execution domain stopped (tid={main})",
            f"iommu: binding {serial} invalidated frames held=6 (not released)",
            f"pci: binding {serial} disabling bus-master",
            f"virtio-blk: binding {serial} stopped reset=1 status=0 queue_enable=0",
            f"iommu: binding {serial} queues stopped cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 faults=0 last_did=0",
            f"iommu: binding {serial} released ddtp=0x0 cqcsr=0x0 fqcsr=0x0 pqcsr=0x0 queue_enable=0 frames=0 stopped=true",
            f"virtio-blk: binding {serial} detached",
            f"hub: binding {serial} closed and released device 41",
            f"ctl: binding {serial} auxiliary={auxiliary} alive=false",
            f"ctl: generation {serial} capabilities=12 launch=1 command=1",
            f"ctl: generation {serial} serial={serial} settled fault={'true' if serial % 2 == 0 else 'false'}",
        ))
        events.extend((
            {"h": 0, "kind": {"room": {"wait": {"tid": main}}}},
            {"h": 1, "kind": {"room": {"park": {"tid": auxiliary}}}},
            {"h": 0, "kind": {"room": {"reap": {"tid": main}}}},
            {"h": 1, "kind": {"room": {"reap": {"tid": auxiliary}}}},
        ))
    lines.extend((
        f"ctl: soak generations={count} elapsed_ms=20000",
        "exit tid=9 reason=0x0 note: device-ctl-soak: done",
        "exit tid=1 reason=0x0 note: system: done",
        "task: all tasks exited, system halted",
        "memory: frame_allocations before=85 after=85",
    ))
    events.extend(
        {"h": 0, "kind": {"room": {"reap": {"tid": tid}}}} for tid in (4, 6, 7, 8, 9, 1)
    )
    events.append({"h": 0, "kind": {"halt": "halt"}})
    for when, event in enumerate(events):
        event["when"] = when
    return "\n".join(lines) + "\n", events
