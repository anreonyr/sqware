"""Perturb real virtio status across reset, retain the failure, then repair MMIO."""

import json
import os
from pathlib import Path
import re
import socket
import subprocess
import threading
import time

FAILURE = "virtio-blk: binding 1 stage Stop failed party=4 op=2 code=4"


def ecam_region(dts):
    nodes = re.findall(r"[^{}]+\{([^{}]*)\}", dts)
    windows = []
    for body in nodes:
        if '"pci-host-ecam-generic"' not in body:
            continue
        cells = re.search(r"\breg = <([^>]+)>;", body)[1].split()
        assert len(cells) == 4, "expected 64-bit ECAM geometry"
        words = [int(cell, 0) for cell in cells]
        windows.append(((words[0] << 32) | words[1], (words[2] << 32) | words[3]))
    assert len(windows) == 1, "ambiguous physical ECAM"
    return windows[0]


def run(conn, capture):
    capture = Path(capture)
    console = Path(os.environ["QEMU_TEE_LOG"])
    hold = Path(os.environ["DMA_HOLD"])
    state = Path(str(hold) + ".json")
    evidence = {"operations": []}
    errors = []

    def read():
        try:
            with capture.open("ab", buffering=0) as output:
                while data := conn.recv(65536):
                    output.write(data)
        except OSError as error:
            errors.append(error)

    reader = threading.Thread(target=read, daemon=True)
    reader.start()

    def text():
        return console.read_text(errors="replace") if console.exists() else ""

    def wait(predicate, reason, seconds=8):
        until = time.monotonic() + seconds
        while not predicate():
            if errors:
                raise errors[0]
            assert time.monotonic() < until, reason
            time.sleep(.001)

    try:
        wait(lambda: state.exists() and json.loads(state.read_text())["phase"] == "held",
             "backend did not hold actual DMA")
        log = text()
        dts = subprocess.check_output(
            ["dtc", "-I", "dtb", "-O", "dts", os.environ["DMA_DTB"]],
            text=True, stderr=subprocess.DEVNULL,
        )
        capture.with_suffix(".dts").write_text(dts)
        ecam, ecam_size = ecam_region(dts)
        placed = re.search(r"pci: endpoint requester=(\d+) bar=(\d+) address=0x([0-9a-f]+) size=0x([0-9a-f]+)", log)
        requester, bar, base, size = int(placed[1]), int(placed[2]), int(placed[3], 16), int(placed[4], 16)
        assert (requester + 1) << 12 <= ecam_size, "requester outside physical ECAM"
        with socket.socket(socket.AF_UNIX) as control:
            control.settimeout(2)
            control.connect(os.environ["DMA_CONTROL"])
            stream = control.makefile("rwb", buffering=0)

            def command(op, address, value=None):
                request = f"{op} {address:#x}" + (f" {value:#x}" if value is not None else "")
                stream.write((request + "\n").encode())
                reply = stream.readline().decode().strip()
                evidence["operations"].append({"request": request, "reply": reply})
                assert reply.startswith("OK"), f"qtest failed: {request}: {reply}"
                return int(reply.split()[1], 0) if len(reply.split()) == 2 else None

            config = ecam + (requester << 12)
            assert command("readw", config) == 0x1af4
            assert command("readw", config + 2) in (0x1001, 0x1042)
            low = command("readl", config + 0x10 + bar * 4)
            high = command("readl", config + 0x14 + bar * 4) if low & 6 == 4 else 0
            assert (low & ~0xf) | (high << 32) == base, "BAR does not decode the reported physical address"
            at = command("readb", config + 0x34)
            visited = set()
            common = None
            while at:
                assert at not in visited and 0x40 <= at <= 0xfc, "bad PCI capability chain"
                visited.add(at)
                vendor = command("readb", config + at)
                if vendor == 9 and command("readb", config + at + 3) == 1:
                    assert command("readb", config + at + 4) == bar
                    common = command("readl", config + at + 8)
                    break
                at = command("readb", config + at + 1)
            assert common is not None and common + 0x1e <= size
            status = base + common + 0x14
            queue = base + common + 0x1c
            evidence.update(requester=requester, bar=bar, base=base, size=size, ecam=ecam, ecam_size=ecam_size,
                            common=common, status_address=status, queue_address=queue)
            evidence["initial_status"] = command("readb", status)
            assert evidence["initial_status"] & 4, "device not DRIVER_OK during real DMA"
            Path(str(hold) + ".resume").write_text("reset-readback injector ready\n")
            wait(lambda: "pci: binding 1 disabling bus-master" in text(), "no actual close")
            # Real status is held nonzero after the guest reset write. The test
            # succeeds only if production readback actually reports Unavailable.
            until = time.monotonic() + 2
            while FAILURE not in text():
                assert "virtio-blk: binding 1 stopped" not in text(), "missed actual reset/readback window"
                assert time.monotonic() < until, "actual reset readback did not reject status"
                command("writeb", status, 1)
            evidence["failed_status"] = command("readb", status)
            evidence["failed_queue"] = command("readw", queue)
            assert evidence["failed_status"] == 1
            failed = text()
            assert "iommu: binding 1 invalidated frames held=6 (not released)" in failed
            for forbidden in ("virtio-blk: binding 1 stopped", "iommu: binding 1 queues stopped",
                              "iommu: binding 1 released", "virtio-blk: binding 1 detached",
                              "hub: binding 1 closed"):
                assert forbidden not in failed, f"advanced without reset confirmation: {forbidden}"
            capture.with_suffix(".failed-console").write_text(failed)
            trace = Path("sqware-diagnose.jsonl").read_text().rpartition("\n")[0] + "\n"
            capture.with_suffix(".failed-diagnose.jsonl").write_text(trace)
            command("writeb", status, 0)
            evidence["repaired_status"] = command("readb", status)
            evidence["repaired_queue"] = command("readw", queue)
            assert evidence["repaired_status"] == evidence["repaired_queue"] == 0
        wait(lambda: "task: all tasks exited, system halted" in text(), "readback repair did not settle")
        reader.join(timeout=5)
        assert not reader.is_alive(), "serial reader did not finish"
    finally:
        capture.with_suffix(".readback.json").write_text(json.dumps(evidence, indent=2))
        if state.exists():
            capture.with_suffix(".block-events").write_text(Path(str(hold) + ".events").read_text())
