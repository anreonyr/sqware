"""Login, user identity and terminal session regression in QEMU."""
import re
import socket
import time

def run(conn, capture):
    received = bytearray()
    conn.settimeout(0.25)
    def wait_for(text, count=1, timeout=60):
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
    def credentials(name, password, prompt):
        wait_for(b"login: ", prompt)
        conn.sendall(name + b"\r\n")
        wait_for(b"Password: ", prompt)
        conn.sendall(password + b"\r\n")
    credentials(b"unknown", b"sqware", 1)
    wait_for(b"Login incorrect", 1)
    credentials(b"anran", b"wrong-secret", 2)
    wait_for(b"Login incorrect", 2)
    wait_for(b"login: ", 3)
    conn.sendall(b"anran\r\n")
    wait_for(b"Password: ", 3)
    conn.sendall(b"cancel-secret\x03")
    wait_for(b"login: ", 4)
    conn.sendall(b"anran\r\n")
    wait_for(b"Password: ", 4)
    conn.sendall(b"\x04")
    wait_for(b"login: ", 5)
    # Editing remains canonical while echo is disabled.
    credentials(b"anran", b"discard\x15sqwarx\x7fe", 5)
    wait_for(b"cat: task=", 1)
    # Wait for the complete identity line before providing cat input.
    wait_for(b"principal=", 1)
    conn.sendall(b"first line\n")
    wait_for(b"first line\r\n", 2)
    conn.sendall(b"exit\n")
    wait_for(b"exit\r\n", 2)
    conn.sendall(b"\x04")
    wait_for(b"login: ", 6)
    credentials(b"anran", b"sqware", 6)
    wait_for(b"cat: task=", 2)
    conn.sendall(b"discarded\x03")
    wait_for(b"login: ", 7)
    identities = re.findall(rb"cat: task=(\d+) principal=(\d+):(\d+)", received)
    assert len(identities) == 2, identities
    assert identities[0][0] != identities[1][0], identities
    assert identities[0][1:] == identities[1][1:], identities
    for secret in (b"sqware", b"wrong-secret", b"cancel-secret", b"sqwarx", b"discard\x15"):
        assert secret not in received, f"password echoed: {secret!r}"
    conn.sendall(b"\x04")
    until = time.monotonic() + 20
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
