"""Boot Login and exercise the real Lisp Shell."""
import socket
import time

def run(conn, capture):
    received = bytearray()
    conn.settimeout(0.25)
    def wait_for(text, timeout=60):
        until = time.monotonic() + timeout
        while text not in received:
            if time.monotonic() >= until:
                raise AssertionError(f"missing {text!r}; received {bytes(received)!r}")
            try:
                data = conn.recv(65536)
            except socket.timeout:
                continue
            if not data:
                raise AssertionError(f"guest ended before {text!r}; received {bytes(received)!r}")
            received.extend(data)
            with open(capture, "ab") as output:
                output.write(data)
    wait_for(b"login: ")
    conn.sendall(b"anran\r")
    wait_for(b"Password: ")
    conn.sendall(b"sqware\r")
    wait_for(b"lisp> ")
    conn.sendall(b"(+ 1 2)\r")
    wait_for(b"3\r\nlisp> ")
    def evaluate(source, expected):
        received.clear()
        conn.sendall(source.encode() + b"\r")
        wait_for((expected + b"\r\n" if expected is not None else b"\r\n") + b"lisp> ")
    evaluate("(define sink (buffer 'write))", b"<port>")
    evaluate("(define a (command 'emit '(\"hello\")))", b"<command>")
    evaluate("(define b (command 'upper '()))", b"<command>")
    evaluate("(define p (connect (list a b) (list (list (list a 'records) (list b 'source)) (list (list b 'result) sink))))", b"<plan>")
    evaluate("(run p)", b"(completed (0 0))")
    evaluate("(bytes sink)", b"#u8(72 69 76 76 79 10)")
    evaluate("(run (connect (list (command 'fail '(\"17\"))) '()))", b"(completed (17))")
    evaluate("(define busy (spawn (connect (list (command 'spin '())) '())))", b"<job>")
    evaluate("(pause busy)", b"<job>")
    evaluate("(car (status busy))", b"paused")
    evaluate("(resume busy)", b"<job>")
    evaluate("(cancel busy)", b"<job>")
    evaluate("(car (status busy))", b"cancelled")
    evaluate("(define ticks (buffer 'write))", b"<port>")
    evaluate("(define worker (command 'workers '()))", b"<command>")
    evaluate("(define w (spawn (connect (list worker) (list (list (list worker 'ticks) ticks)))))", b"<job>")
    time.sleep(0.15)
    evaluate("(pause w)", b"<job>")
    evaluate("(define before (bytes-length (bytes ticks)))", None)
    time.sleep(0.15)
    evaluate("(= before (bytes-length (bytes ticks)))", b"#t")
    evaluate("(resume w)", b"<job>")
    evaluate("(cancel w)", b"<job>")
    evaluate("(car (status w))", b"cancelled")
    evaluate("(define huge (buffer 'write))", b"<port>")
    evaluate("(define many (command 'emit '(\"--repeat\" \"131072\" \"abcdefgh\")))", b"<command>")
    evaluate("(run (connect (list many) (list (list (list many 'records) huge))))", b"(completed (0))")
    evaluate("(bytes-length (bytes huge))", b"1048576")
    evaluate("(define input (terminal 'read))", b"<port>")
    evaluate("(define output (buffer 'write))", b"<port>")
    evaluate("(define copy (command 'cat '()))", b"<command>")
    evaluate("(define reader (spawn (connect (list copy) (list (list input (list copy 'source)) (list (list copy 'copy) output)))))", b"<job>")
    evaluate("(wait reader)", b"(paused (#f) background-read)")
    received.clear()
    conn.sendall(b"(fg reader)\r")
    time.sleep(0.2)
    conn.sendall(b"typed line\r")
    time.sleep(0.2)
    conn.sendall(b"\x04")
    wait_for(b"(completed (0))\r\nlisp> ")
    evaluate("(bytes-length (bytes output))", b"11")
    evaluate("(define stopped (spawn (connect (list (command 'spin '())) '())))", b"<job>")
    received.clear()
    conn.sendall(b"(fg stopped)\r")
    time.sleep(0.2)
    conn.sendall(b"\x1a")
    wait_for(b"(paused (#f) requested)\r\nlisp> ")
    evaluate("(bg stopped)", b"<job>")
    received.clear()
    conn.sendall(b"(fg stopped)\r")
    time.sleep(0.2)
    conn.sendall(b"\x03")
    wait_for(b"lisp> ")
    evaluate("(car (status stopped))", b"cancelled")
    evaluate("(define (forever n) (forever (+ n 1)))", b"<function>")
    received.clear()
    conn.sendall(b"(forever 0)\r")
    time.sleep(0.2)
    conn.sendall(b"\x03")
    wait_for(b"lisp> ")
    evaluate("(+ 2 3)", b"5")
    received.clear()
    conn.sendall(b"\x04")
    wait_for(b"login: ")
    conn.sendall(b"\x04")
    until = time.monotonic() + 20
    while time.monotonic() < until:
        try:
            data = conn.recv(65536)
        except socket.timeout:
            continue
        if not data:
            return
        with open(capture, "ab") as output:
            output.write(data)
    raise AssertionError("scene did not end after Login EOF")
