"""Read-only NBD fixture; a guest DMA read waits for an external receipt."""

import json
import os
import pathlib
import socket
import struct
import sys
import time


def exact(conn, count):
    out = bytearray()
    while len(out) < count:
        data = conn.recv(count - len(out))
        if not data:
            if out:
                raise ValueError("truncated NBD request")
            raise EOFError("NBD client disconnected")
        out.extend(data)
    return bytes(out)


def serve(path, disk):
    path = pathlib.Path(path)
    state = pathlib.Path(str(path) + ".json")
    resume = pathlib.Path(str(path) + ".resume")
    events = pathlib.Path(str(path) + ".events")
    image = pathlib.Path(disk).read_bytes()
    console = pathlib.Path(os.environ["QEMU_TEE_LOG"])

    def record(phase, **fields):
        value = {"phase": phase, **fields}
        pending = state.with_suffix(".pending")
        pending.write_text(json.dumps(value))
        pending.replace(state)
        with events.open("a") as out:
            out.write(json.dumps(value) + "\n")

    with socket.socket(socket.AF_UNIX) as server:
        server.bind(str(path))
        server.listen(1)
        server.settimeout(15)
        record("ready")
        conn, _ = server.accept()
        with conn:
            conn.settimeout(15)
            # Oldstyle NBD: fixed read-only export, no option negotiation.
            conn.sendall(
                b"NBDMAGIC"
                + struct.pack(">QQI", 0x00420281861253, len(image), 3)
                + bytes(124)
            )
            held = False
            while True:
                try:
                    header = exact(conn, 28)
                except EOFError:
                    record("disconnected")
                    return
                magic, flags, op, cookie, offset, length = struct.unpack(">IHHQQI", header)
                if magic != 0x25609513 or flags != 0:
                    raise AssertionError("unexpected NBD request header")
                if op == 2:
                    record("disconnected")
                    return
                if op != 0 or length == 0 or offset + length > len(image):
                    raise AssertionError(f"unexpected NBD operation {op} at {offset}+{length}")
                active = console.exists() and (
                    "hub: binding 1 active (business released)"
                    in console.read_text(errors="replace")
                )
                if not held and active:
                    if (offset, length) != (0, 512):
                        raise AssertionError(f"guest request is not a sector read: {offset}+{length}")
                    held = True
                    record("held", cookie=cookie, offset=offset, length=length)
                    until = time.monotonic() + float(os.environ.get("DMA_HOLD_SECONDS", "10"))
                    while not resume.exists():
                        if time.monotonic() >= until:
                            raise TimeoutError("NBD read never released")
                        time.sleep(0.005)
                    record("resuming", cookie=cookie)
                elif not held:
                    record("boot-read", cookie=cookie, offset=offset, length=length)
                conn.sendall(struct.pack(">IIQ", 0x67446698, 0, cookie) + image[offset:offset + length])
                record("replied", cookie=cookie, offset=offset, length=length, error=0)


if __name__ == "__main__":
    serve(*sys.argv[1:])
