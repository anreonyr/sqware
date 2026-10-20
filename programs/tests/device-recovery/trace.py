#!/usr/bin/env python3
"""Check real multi-Hart execution of both dynamic DMA binding generations."""

import json
import re
import sys
from pathlib import Path


def check(console_path, events_path):
    console = Path(console_path).read_text()
    events = [json.loads(line) for line in Path(events_path).read_text().splitlines()]
    mains = re.findall(r"ctl: constructed device-work tid=(\d+) owner=hub", console)
    assert len(mains) == 2, "both generations must be real dynamic executions"
    for serial, main in enumerate(map(int, mains), 1):
        probes = re.findall(
            rf"ctl: binding {serial} auxiliary=(\d+) alive=(true|false)", console
        )
        assert len(probes) == 2, f"binding {serial}: missing native liveness checks"
        assert probes[0][1] == "true" and probes[1] == (probes[0][0], "false")
        auxiliary = int(probes[0][0])
        main_harts = set()
        auxiliary_harts = set()
        reaped = set()
        for event in events:
            room = event.get("kind", {}).get("room", {})
            for kind in ("wait", "park", "exit"):
                if room.get(kind, {}).get("tid") == main:
                    main_harts.add(event["h"])
            switch = room.get("switch", {})
            if auxiliary in (switch.get("prev_tid"), switch.get("next_tid")):
                auxiliary_harts.add(event["h"])
            if room.get("reap", {}).get("tid") in (main, auxiliary):
                reaped.add(room["reap"]["tid"])
        assert reaped == {main, auxiliary}, f"binding {serial}: incomplete domain reaping"
        assert any(a != b for a in main_harts for b in auxiliary_harts), (
            f"binding {serial}: no distinct-Hart execution evidence"
        )
        print(
            f"binding {serial}: main={main} harts={sorted(main_harts)} "
            f"auxiliary={auxiliary} harts={sorted(auxiliary_harts)} reaped=true"
        )


if __name__ == "__main__":
    check(*sys.argv[1:])
