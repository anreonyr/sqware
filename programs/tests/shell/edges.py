"""Real rollback, broken pipe, peer failure, long input and terminal output."""
import socket
import time

def run(conn, capture):
    received = bytearray()
    conn.settimeout(0.25)
    def wait_for(text, timeout=60):
        until = time.monotonic() + timeout
        while text not in received:
            if time.monotonic() >= until:
                raise AssertionError(f"missing {text!r}; tail {bytes(received[-4096:])!r}")
            try:
                data = conn.recv(65536)
            except socket.timeout:
                continue
            if not data:
                raise AssertionError(f"guest ended before {text!r}")
            received.extend(data)
            with open(capture, "ab") as output:
                output.write(data)
    def evaluate(source, expected):
        received.clear()
        conn.sendall(source.encode() + b"\r")
        wait_for(expected)
        wait_for(b"lisp> ")
    wait_for(b"login: ")
    conn.sendall(b"anran\r")
    wait_for(b"Password: ")
    conn.sendall(b"sqware\r")
    wait_for(b"lisp> ")
    for _ in range(3):
        evaluate('(string-length "' + '中x' * 500 + '")', b"1000\r\nlisp> ")
    evaluate("(define (repeat s n) (if (= n 0) s (repeat (string-append s s) (- n 1))))", b"<function>")
    evaluate("(define bad (command 'spin (list (repeat \"01234567\" 13))))", b"<command>")
    evaluate("(prepare (connect (list bad) '()))", b"invalid launch manifest")
    evaluate("(+ 4 5)", b"9\r\nlisp> ")
    evaluate("(define sink (buffer 'write))", b"<port>")
    evaluate("(define emit (command 'emit '(\"OK\")))", b"<command>")
    evaluate("(define failure (command 'fail '(\"17\")))", b"<command>")
    evaluate("(run (connect (list failure emit) (list (list (list emit 'records) sink))))", b"(completed (17 0))")
    evaluate("(bytes sink)", b"#u8(79 75 10)")
    evaluate("(define tty (terminal 'write))", b"<port>")
    evaluate("(define print (command 'emit '(\"--repeat\" \"2500\" \"abcdefgh\")))", b"<command>")
    evaluate("(run (connect (list print) (list (list (list print 'records) tty))))", b"abcdefgh" * 2500)
    assert b"(completed (0))" in received
    evaluate("(define closed (buffer 'write))", b"<port>")
    evaluate("(define bulk (command 'emit '(\"--repeat\" \"131072\" \"abcdefgh\")))", b"<command>")
    evaluate("(define broken (spawn (connect (list bulk) (list (list (list bulk 'records) closed)))))", b"<job>")
    evaluate("(close closed)", b"()")
    evaluate("(wait broken)", b"(completed (1))")
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
    raise AssertionError("scene did not end")
