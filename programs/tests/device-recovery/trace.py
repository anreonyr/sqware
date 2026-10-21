#!/usr/bin/env python3
"""Check real multi-Hart execution of every dynamic DMA binding generation."""

import json
import re
import sys
from pathlib import Path


def check(console_path, events_path, generations=2):
    generations = int(generations)
    console = Path(console_path).read_text()
    mains = re.findall(r"ctl: constructed device-work tid=(\d+) owner=hub", console)
    assert len(mains) == generations, "all generations must be real dynamic executions"
    probes_by_serial = {}
    for serial, auxiliary, alive in re.findall(
        r"ctl: binding (\d+) auxiliary=(\d+) alive=(true|false)", console,
    ):
        probes_by_serial.setdefault(int(serial), []).append((auxiliary, alive))
    pairs = []
    harts = {}
    for serial, main in enumerate(map(int, mains), 1):
        probes = probes_by_serial.get(serial, [])
        assert len(probes) == 2, f"binding {serial}: missing native liveness checks"
        assert probes[0][1] == "true" and probes[1] == (probes[0][0], "false")
        auxiliary = int(probes[0][0])
        assert main not in harts and auxiliary not in harts and main != auxiliary, "execution task identity reused"
        pairs.append((serial, main, auxiliary))
        harts[main] = set()
        harts[auxiliary] = set()
    retired = {}
    last = None
    with Path(events_path).open() as source:
        for line in source:
            event = json.loads(line)
            last = event.get("kind")
            room = event.get("kind", {}).get("room", {})
            for kind in ("wait", "park"):
                tid = room.get(kind, {}).get("tid")
                if tid in harts:
                    harts[tid].add(event["h"])
            switch = room.get("switch", {})
            for tid in (switch.get("prev_tid"), switch.get("next_tid")):
                if tid in harts:
                    harts[tid].add(event["h"])
            reap = room.get("reap", {})
            if reap:
                tid = reap["tid"]
                assert tid not in retired, f"duplicate reap: {tid}"
                retired[tid] = event["when"]
    for serial, main, auxiliary in pairs:
        main_harts, auxiliary_harts = harts[main], harts[auxiliary]
        assert main in retired and auxiliary in retired, f"binding {serial}: incomplete domain reaping"
        assert any(a != b for a in main_harts for b in auxiliary_harts), (
            f"binding {serial}: no distinct-Hart execution evidence"
        )
        print(
            f"binding {serial}: main={main} harts={sorted(main_harts)} "
            f"auxiliary={auxiliary} harts={sorted(auxiliary_harts)} reaped=true"
        )
    return retired, last


if __name__ == "__main__":
    check(*sys.argv[1:])
