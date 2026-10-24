"""Break the real command queue, then let hardware complete the same IOFENCE."""

import json
import os
from pathlib import Path
import re
import socket
import subprocess
import threading
import time

CQB, CQH, CQT, CQCSR = 0x18, 0x20, 0x24, 0x48
ENABLE, ON, BUSY, MEM_FAULT = 1, 1 << 16, 1 << 17, 1 << 8


def regions(dts):
    leaves = re.findall(r"[^{}]+\{([^{}]*)\}", dts)

    def window(body):
        cells = re.search(r"\breg = <([^>]+)>;", body)[1].split()
        assert len(cells) == 4, "QEMU virt fixture requires 64-bit address/size cells"
        words = [int(cell, 0) for cell in cells]
        return (words[0] << 32) | words[1], (words[2] << 32) | words[3]

    iommus = [window(body) for body in leaves if '"riscv,iommu"' in body]
    memories = [window(body) for body in leaves if 'device_type = "memory"' in body]
    assert len(iommus) == len(memories) == 1, "ambiguous QEMU device tree"
    memory_base, memory_size = memories[0]
    assert memory_size > 0 and (memory_base + memory_size) % 4096 == 0
    return iommus[0][0], memory_base + memory_size


def run(conn, capture):
    capture = Path(capture)
    console = Path(os.environ["QEMU_TEE_LOG"])
    hold = Path(os.environ["DMA_HOLD"])
    state = Path(str(hold) + ".json")
    errors = []
    evidence = {}

    def read():
        try:
            with capture.open("ab", buffering=0) as out:
                while data := conn.recv(65536):
                    out.write(data)
        except OSError as error:
            errors.append(error)

    reader = threading.Thread(target=read, daemon=True)
    reader.start()

    def text():
        return console.read_text(errors="replace") if console.exists() else ""

    def wait(predicate, message, seconds=8):
        until = time.monotonic() + seconds
        while not predicate():
            if errors:
                raise errors[0]
            assert time.monotonic() < until, message
            time.sleep(.005)

    def held():
        return state.exists() and json.loads(state.read_text())["phase"] == "held"

    try:
        dts = subprocess.check_output(
            ["dtc", "-I", "dtb", "-O", "dts", os.environ["DMA_DTB"]],
            text=True, stderr=subprocess.DEVNULL,
        )
        capture.with_suffix(".dts").write_text(dts)
        base, bad = regions(dts)
        wait(held, "backend did not hold the normal guest DMA read")
        wait(lambda: "work: rang the doorbell" in text(), "missing real doorbell")
        with socket.socket(socket.AF_UNIX) as control:
            control.settimeout(2)
            control.connect(os.environ["DMA_CONTROL"])
            stream = control.makefile("rwb", buffering=0)
            operations = []
            evidence["operations"] = operations

            def command(op, address, value=None):
                request = f"{op} {address:#x}" + (f" {value:#x}" if value is not None else "")
                stream.write((request + "\n").encode())
                reply = stream.readline().decode().strip()
                operations.append({"request": request, "reply": reply})
                assert reply.startswith("OK"), f"qtest failed: {request}: {reply}"
                return int(reply.split()[1], 0) if len(reply.split()) == 2 else None

            original = command("readq", base + CQB)
            csr = command("readl", base + CQCSR)
            assert csr & (ENABLE | ON) == ENABLE | ON, "queue not published"
            command("writel", base + CQCSR, 0)
            wait(lambda: command("readl", base + CQCSR) & (ENABLE | ON | BUSY) == 0, "queue still on")
            broken = ((bad >> 12) << 10) | (original & 0x1f)
            command("writeq", base + CQB, broken)
            command("writel", base + CQCSR, ENABLE | MEM_FAULT)
            assert command("readq", base + CQB) == broken
            evidence.update(base=base, bad=bad, original=original, broken=broken)
            Path(str(hold) + ".resume").write_text("real command queue fault configured\n")
            wait(
                lambda: "virtio-blk: binding 1 stage Invalidate failed party=3 op=3 code=4" in text(),
                "native invalidation did not report its real fence failure",
            )
            log = text()
            assert "iommu: binding 1 invalidate fence expired frames held=6" in log
            for forbidden in ("pci: binding 1 disabling", "virtio-blk: binding 1 stopped", "iommu: binding 1 released"):
                assert forbidden not in log, f"advanced without fence confirmation: {forbidden}"
            capture.with_suffix(".failed-console").write_text(log)
            trace = Path("sqware-diagnose.jsonl").read_text().rpartition("\n")[0] + "\n"
            capture.with_suffix(".failed-diagnose.jsonl").write_text(trace)
            tail = command("readl", base + CQT)
            head = command("readl", base + CQH)
            fault = command("readl", base + CQCSR)
            assert fault & MEM_FAULT, "hardware did not report command fetch failure"
            assert head != tail, "no actual outstanding commands"
            entries = 2 << (original & 0x1f)
            queue = (original >> 10) << 12
            last = queue + ((tail - 1) % entries) * 16
            prior = queue + ((tail - 2) % entries) * 16
            directory_at = queue + ((tail - 3) % entries) * 16
            directory = command("readq", directory_at)
            invalidate = command("readq", prior)
            fence = command("readq", last)
            destination = command("readq", last + 8) << 2
            cookie = fence >> 32
            assert directory & 0x7f == 3 and invalidate & 0x7f == 1 and fence & 0x7f == 2, "not IODIR then IOTINVAL then IOFENCE"
            before = command("readl", destination)
            assert before == 0 and cookie != 0, "completion was already present"
            evidence.update(head=head, tail=tail, fault=fault, directory=directory, invalidate=invalidate,
                            fence=fence, destination=destination, cookie=cookie, before=before)
            command("writel", base + CQCSR, 0)
            wait(lambda: command("readl", base + CQCSR) & (ENABLE | ON | BUSY) == 0, "repair queue still on")
            command("writeq", base + CQB, original)
            command("writel", base + CQCSR, ENABLE | MEM_FAULT | (1 << 9) | (1 << 10))
            assert command("readq", base + CQB) == original, "queue base not restored"
            command("writel", base + CQT, tail)
            wait(lambda: command("readl", destination) == cookie, "same IOFENCE did not complete")
            evidence["after"] = cookie
        wait(lambda: "task: all tasks exited, system halted" in text(), "recovery did not finish")
        reader.join(timeout=5)
        assert not reader.is_alive(), "serial reader did not finish"
    finally:
        capture.with_suffix(".hardware.json").write_text(json.dumps(evidence, indent=2))
        if state.exists():
            capture.with_suffix(".block-events").write_text(Path(str(hold) + ".events").read_text())
