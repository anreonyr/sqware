"""Feed the single guard-death trigger after the retained DMA read is armed.

The QEMU runner is expected to time out because the guarded kernel stop never
returns. This driver only records what the host actually saw; the independent
verifier decides acceptance.
"""

import json
import os
from pathlib import Path
import re
import threading
import time


def run(conn, capture):
    capture = Path(capture)
    console = Path(os.environ["QEMU_TEE_LOG"])
    hold = Path(os.environ["DMA_HOLD"])
    state = Path(str(hold) + ".json")
    events = Path(str(hold) + ".events")
    resume = Path(str(hold) + ".resume")
    errors = []
    closed = threading.Event()
    eof = threading.Event()
    feed = {
        "sent_g": False,
        "held_before_trigger": False,
        "resume": False,
        "trigger": False,
        "serial_eof": False,
    }
    held_console = ""
    held_events = ""

    def read():
        try:
            with capture.open("ab", buffering=0) as out:
                while True:
                    data = conn.recv(65536)
                    if not data:
                        eof.set()
                        break
                    out.write(data)
        except OSError as error:
            errors.append(error)
        finally:
            closed.set()

    reader = threading.Thread(target=read, daemon=True)
    reader.start()

    def text():
        return console.read_text(errors="replace") if console.exists() else ""

    def wait(predicate, message, seconds=30):
        until = time.monotonic() + seconds
        while not predicate():
            if errors:
                raise errors[0]
            if closed.is_set() and message != "serial EOF":
                raise AssertionError(f"{message}: serial closed first")
            if time.monotonic() >= until:
                raise AssertionError(message)
            time.sleep(0.005)

    def held():
        try:
            return json.loads(state.read_text()).get("phase") == "held"
        except (OSError, ValueError):
            return False

    try:
        wait(held, "NBD did not hold the guest read")
        wait(lambda: "work: rang the in-flight request doorbell" in text(),
             "missing in-flight doorbell")
        wait(lambda: re.search(r"^guard: ready target=(pci|iommu|recovery) task=\d+ team=\d+$",
                               text(), re.MULTILINE) is not None,
             "missing guard ready")
        if resume.exists():
            raise AssertionError("NBD resume receipt appeared before trigger")
        if not held():
            raise AssertionError("held read was released before trigger")
        held_console = text()
        held_events = events.read_text() if events.exists() else ""
        capture.with_suffix(".held-console").write_text(held_console)
        capture.with_suffix(".held-events").write_text(held_events)
        feed["held_before_trigger"] = True
        conn.sendall(b"G")
        feed["sent_g"] = True
        wait(lambda: "guard: trigger " in text(), "guard trigger did not follow G", 10)
        feed["trigger"] = True
        wait(lambda: "[critical] guarded exit" in text(), "guarded exit did not follow trigger", 10)
        wait(closed.is_set, "serial EOF", 60)
        if errors:
            raise errors[0]
        if not eof.is_set():
            raise AssertionError("serial closed without EOF")
        feed["serial_eof"] = True
    finally:
        feed["resume"] = resume.exists()
        feed["reader_error"] = str(errors[0]) if errors else None
        if not held_console:
            held_console = text()
        if not held_events and events.exists():
            held_events = events.read_text()
        capture.with_suffix(".held-console").write_text(held_console)
        capture.with_suffix(".held-events").write_text(held_events)
        if events.exists():
            capture.with_suffix(".block-events").write_text(events.read_text())
        capture.with_suffix(".feed.json").write_text(json.dumps(feed, indent=2))
        reader.join(timeout=2)
