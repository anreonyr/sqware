"""QEMU interaction regression: three cat tasks, EOF, Ctrl-C and foreground return.
Run: nu scripts/qtest.nu --package kernel --scene accept --feed-script programs/tests/terminal/session.py
"""
import socket
import time


def run(conn, capture):
    received = bytearray()
    conn.settimeout(0.25)

    def wait_for(text, count=1, timeout=12):
        until = time.monotonic() + timeout
        while received.count(text) < count:
            if time.monotonic() >= until:
                raise AssertionError(f"missing {text!r} occurrence {count}; received {bytes(received)!r}")
            try:
                data = conn.recv(65536)
            except socket.timeout:
                continue
            if not data:
                raise AssertionError(f"guest ended before {text!r}")
            received.extend(data)
            with open(capture, "ab") as output:
                output.write(data)

    def login(name, count):
        wait_for(b"login: ", count)
        conn.sendall(name + b"\r\n")
        wait_for(b"Hello, " + name + b".\r\n")
        # The greeting precedes the held task's capability installation and release.
        time.sleep(0.15)

    login(b"alice", 1)
    conn.sendall(b"first line\n")
    wait_for(b"first line\r\n", 2)
    conn.sendall(b"exit\n")
    wait_for(b"exit\r\n", 2)
    conn.sendall(b"\x04")
    wait_for(b"login: ", 2)

    login(b"bob", 2)
    conn.sendall(b"discarded\x03")
    wait_for(b"^C\r\n")
    wait_for(b"login: ", 3)

    login(b"carol", 3)
    conn.sendall(b"third line\n")
    wait_for(b"third line\r\n", 2)
    conn.sendall(b"\x04")
    wait_for(b"login: ", 4)
    assert b"terminal:" not in received, "terminal must not emit a banner"
    conn.sendall(b"\x04")
    # Login ends the demo session; terminal is stopped by the scene owner afterward.
    until = time.monotonic() + 12
    while time.monotonic() < until:
        try:
            data = conn.recv(65536)
        except socket.timeout:
            continue
        if not data:
            return
        received.extend(data)
        with open(capture, "ab") as output:
            output.write(data)
    raise AssertionError("scene did not end after login EOF")
