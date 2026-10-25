"""Run after cargo image product release and cargo build -p kernel --release."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time


def main():
    root = Path(__file__).resolve().parents[3]
    with tempfile.TemporaryDirectory(prefix="sqware-idle-") as tmp:
        elf = root / "target/riscv64gc-unknown-none-elf/release/shuyra"
        args = subprocess.check_output([
            "nu", "scripts/qemu-args.nu", str(elf), "--serial",
            f"unix:{tmp}/serial,server=on,wait=off",
        ], cwd=root, text=True).splitlines()
        qemu = subprocess.Popen([
            "qemu-system-riscv64", *args, "-qmp",
            f"unix:{tmp}/qmp,server=on,wait=off",
        ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        serial = socket.socket(socket.AF_UNIX)
        monitor = socket.socket(socket.AF_UNIX)
        try:
            until = time.monotonic() + 5
            while True:
                try:
                    serial.connect(f"{tmp}/serial")
                    monitor.connect(f"{tmp}/qmp")
                    break
                except (FileNotFoundError, ConnectionRefusedError):
                    if time.monotonic() > until:
                        raise
                    time.sleep(0.05)
            serial.settimeout(0.25)
            qmp = monitor.makefile("rwb", buffering=0)
            qmp.readline()

            def command(name, arguments=None):
                request = {"execute": name}
                if arguments is not None:
                    request["arguments"] = arguments
                qmp.write((json.dumps(request) + "\n").encode())
                while True:
                    result = json.loads(qmp.readline())
                    if "return" in result or "error" in result:
                        return result

            command("qmp_capabilities")

            def wait_for(text, timeout=30):
                received = bytearray()
                until = time.monotonic() + timeout
                while text not in received:
                    if time.monotonic() > until:
                        registers = command("human-monitor-command", {"command-line": "info registers -a"})
                        raise AssertionError(f"missing {text!r}; tail {bytes(received[-4096:])!r}; {registers}")
                    try:
                        data = serial.recv(65536)
                    except socket.timeout:
                        continue
                    if not data:
                        raise AssertionError(f"guest ended; tail {bytes(received[-4096:])!r}")
                    received.extend(data)
                return bytes(received)

            def evaluate(source, expected):
                serial.sendall(source.encode() + b"\r")
                return wait_for(expected + b"\r\nlisp> ")

            def usage(label):
                def ticks():
                    fields = Path(f"/proc/{qemu.pid}/stat").read_text().split()
                    return int(fields[13]) + int(fields[14])
                time.sleep(0.25)
                before = ticks()
                start = time.monotonic()
                time.sleep(2)
                cpu = (ticks() - before) / os.sysconf("SC_CLK_TCK") / (time.monotonic() - start) * 100
                print(f"{label}: {cpu:.1f}%", flush=True)
                if cpu >= 20:
                    for _ in range(3):
                        print(command("human-monitor-command", {"command-line": "info registers -a"}), flush=True)
                        time.sleep(0.1)
                assert cpu < 20, f"{label}: expected idle, got {cpu:.1f}% of one host core"

            wait_for(b"login: ")
            usage("login")
            serial.sendall(b"anran\r")
            wait_for(b"Password: ")
            serial.sendall(b"sqware\r")
            wait_for(b"lisp> ")
            usage("shell")
            evaluate("(+ 1 2)", b"3")
            usage("after expression")
            serial.sendall(b"(+ 1\r")
            wait_for(b"....> ")
            usage("incomplete expression")
            serial.sendall(b"2)\r")
            wait_for(b"3\r\nlisp> ")
            evaluate("(define busy (spawn (connect (list (command 'spin '())) '())))", b"<job>")
            evaluate("(pause busy)", b"<job>")
            usage("paused job")
            evaluate("(bg busy)", b"<job>")
            serial.sendall(b"(fg busy)\r")
            time.sleep(0.25)
            serial.sendall(b"\x1a")
            wait_for(b"(paused (#f) requested)\r\nlisp> ")
            usage("Ctrl-Z")
            evaluate("(cancel busy)", b"<job>")
            evaluate("(define input (terminal 'read))", b"<port>")
            evaluate("(define output (buffer 'write))", b"<port>")
            evaluate("(define copy (command 'cat '()))", b"<command>")
            evaluate("(define reader (spawn (connect (list copy) (list (list input (list copy 'source)) (list (list copy 'copy) output)))))", b"<job>")
            evaluate("(wait reader)", b"(paused (#f) background-read)")
            usage("background read")
            serial.sendall(b"(fg reader)\r")
            usage("foreground read")
            serial.sendall(b"hello\r")
            time.sleep(0.2)
            serial.sendall(b"\x04")
            wait_for(b"(completed (0))\r\nlisp> ")
            serial.sendall(b"\x04")
            wait_for(b"login: ")
            usage("after shell exit")
            serial.sendall(b"\x04")
            until = time.monotonic() + 20
            while time.monotonic() < until:
                try:
                    if serial.recv(65536) == b"":
                        break
                except socket.timeout:
                    continue
            assert qemu.wait(timeout=5) == 0
        finally:
            serial.close()
            monitor.close()
            if qemu.poll() is None:
                qemu.terminate()
                qemu.wait(timeout=5)


if __name__ == "__main__":
    main()
