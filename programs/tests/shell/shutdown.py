"""Exit Login while Shell still owns live tasks and shared pages."""
import socket
import time


def run(conn, capture):
    received = bytearray()
    conn.settimeout(0.25)

    def receive():
        try:
            data = conn.recv(65536)
        except socket.timeout:
            return None
        if data:
            received.extend(data)
            with open(capture, "ab") as output:
                output.write(data)
        return data

    def wait_for(text, timeout=60):
        until = time.monotonic() + timeout
        while text not in received:
            if time.monotonic() >= until:
                raise AssertionError(f"missing {text!r}; tail {bytes(received[-4096:])!r}")
            if receive() == b"":
                raise AssertionError(f"guest ended before {text!r}")

    def evaluate(source, expected):
        received.clear()
        conn.sendall(source.encode() + b"\r")
        wait_for(expected + b"\r\nlisp> ")

    wait_for(b"login: ")
    conn.sendall(b"anran\r")
    wait_for(b"Password: ")
    conn.sendall(b"sqware\r")
    wait_for(b"lisp> ")
    evaluate("(define spin (spawn (connect (list (command 'spin '())) '())))", b"<job>")
    evaluate("(define ticks (buffer 'write))", b"<port>")
    evaluate("(define worker (command 'workers '()))", b"<command>")
    evaluate("(define w (spawn (connect (list worker) (list (list (list worker 'ticks) ticks)))))", b"<job>")
    evaluate("(car (status spin))", b"running")
    evaluate("(car (status w))", b"running")
    # Leave both teams and the mapped buffer alive; Shell must reclaim them.
    received.clear()
    conn.sendall(b"\x04")
    wait_for(b"login: ")
    conn.sendall(b"\x04")
    until = time.monotonic() + 20
    while time.monotonic() < until:
        if receive() == b"":
            return
    raise AssertionError("scene did not end after Login EOF")
