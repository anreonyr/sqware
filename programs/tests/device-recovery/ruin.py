"""Hold an actual guest block read across static Driver Ruin."""

import os
import json
import pathlib
import threading
import time

from .ruin_check import pending


def run(conn, capture, mode="ruin"):
    assert mode in ("ruin", "forced")
    capture = pathlib.Path(capture)
    console = pathlib.Path(os.environ["QEMU_TEE_LOG"])
    hold = pathlib.Path(os.environ["DMA_HOLD"])
    state = pathlib.Path(str(hold) + ".json")
    resume = pathlib.Path(str(hold) + ".resume")
    errors = []

    def read():
        try:
            with capture.open("ab", buffering=0) as out:
                while data := conn.recv(65536):
                    out.write(data)
        except OSError as error:
            errors.append(error)

    reader = threading.Thread(target=read, daemon=True)
    reader.start()

    def wait(predicate, message, seconds=None):
        seconds = seconds or (30 if mode == "forced" else 10)
        until = time.monotonic() + seconds
        while not predicate():
            if errors:
                raise errors[0]
            if time.monotonic() >= until:
                raise AssertionError(message)
            time.sleep(0.005)

    def text(path):
        return path.read_text(errors="replace") if path.exists() else ""

    def held():
        return state.exists() and json.loads(state.read_text()).get("phase") == "held"

    try:
        wait(held, "NBD server did not receive and hold the guest read")
        wait(
            lambda: "work: rang the in-flight request doorbell" in text(console),
            "guest did not submit its block request",
        )
        wait(
            lambda: "pci: binding 1 disabling bus-master" in text(console),
            "Ruin did not reach PCI disable",
        )
        if not held():
            raise AssertionError("guest read was no longer held at PCI disable")
        if "iommu: binding 1 invalidated frames held=6 (not released)" not in text(console):
            raise AssertionError("missing invalidation confirmation with retained frame references")
        until = time.monotonic() + 0.1
        while time.monotonic() < until:
            log = text(console)
            if not held():
                raise AssertionError("guest read completed without a resume receipt")
            for forbidden in [
                "pci: endpoint bus-master off=0x0",
                "recovery: binding 1 stopped",
                "iommu: binding 1 released",
                "hub: binding 1 closed",
                "ruin: recovery dead after reply",
                "system: forced shutdown admitted after DMA settlement",
                "system: forced shutdown admitted after retirement settlement",
                "task: all tasks exited, system halted",
            ]:
                if forbidden in log:
                    raise AssertionError(f"premature retirement while read suspended: {forbidden}")
            time.sleep(0.005)
        log = text(console)
        trace = pathlib.Path("sqware-diagnose.jsonl").read_text().rpartition("\n")[0] + "\n"
        pending(log, [json.loads(line) for line in trace.splitlines()], mode)
        capture.with_suffix(".held-console").write_text(log)
        capture.with_suffix(".held-diagnose.jsonl").write_text(trace)
        resume.write_text(f"{mode} pending with frame references held\n")
        completion = (
            "system: frame Step(Idle)" if mode == "forced" else "ruin: recovery dead after reply"
        )
        wait(lambda: completion in text(console), f"{mode} did not complete after resume")
        reader.join(timeout=10)
        if reader.is_alive():
            raise AssertionError(f"guest did not terminate after {mode}")
        if errors:
            raise errors[0]
        capture.with_suffix(".hold").write_text(
            f"guest DMA read held during {mode} (boot reads excluded)\n"
            "PCI disable pending while suspended; IOMMU retains 6 frame references\n"
            f"{mode} completed only after request resumed\n"
        )
    except Exception:
        capture.with_suffix(".observed-console").write_text(
            f"console={console}\n" + text(console)
        )
        raise
    finally:
        if state.exists():
            capture.with_suffix(".block-events").write_text(
                pathlib.Path(str(hold) + ".events").read_text()
            )
