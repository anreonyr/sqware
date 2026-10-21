"""The runner must own the VM PID, and capture failure must remain observable."""

import json
import os
from pathlib import Path
import re
import signal
import subprocess
import tempfile
import time
import unittest


class QemuWrapper(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        source = Path(__file__).resolve().parents[3] / "scripts/qtest.nu"
        script = re.search(r"const WRAPPER = '(.*?)'\n\n", source.read_text(), re.DOTALL)[1]
        self.wrapper = self.root / "wrapper"
        self.wrapper.write_text(script)
        self.wrapper.chmod(0o755)
        self.binary = self.root / "qemu-system-riscv64"
        self.binary.write_text("""#!/usr/bin/env python3
import json, os, pathlib, sys, time
pathlib.Path(os.environ["VM_PID"]).write_text(str(os.getpid()))
pathlib.Path(os.environ["VM_ARGS"]).write_text(json.dumps(sys.argv[1:]))
print("VM stdout", flush=True)
print("VM stderr", file=sys.stderr, flush=True)
if os.environ.get("VM_HANG") == "1":
    time.sleep(60)
sys.exit(int(os.environ.get("VM_EXIT", "0")))
""")
        self.binary.chmod(0o755)
        self.status = self.root / "capture.status"
        self.status.write_text("P")
        self.capture = self.root / "console"
        self.pid = self.root / "vm.pid"
        self.args = self.root / "args.json"
        self.env = {
            **os.environ,
            "PATH": str(self.root) + os.pathsep + os.environ["PATH"],
            "QEMU_TEE_LOG": str(self.capture),
            "QEMU_TEE_STATUS": str(self.status),
            "VM_PID": str(self.pid),
            "VM_ARGS": str(self.args),
        }

    def start(self, **environment):
        process = subprocess.Popen(
            [str(self.wrapper), "-nographic", "-machine", "virt", "-m", "256"],
            stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            env={**self.env, **environment}, start_new_session=True,
        )

        def cleanup():
            owned = process.poll() is None
            if self.pid.exists():
                try:
                    owned |= os.getpgid(int(self.pid.read_text())) == process.pid
                except ProcessLookupError:
                    pass
            if owned:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            process.communicate(timeout=5)

        self.addCleanup(cleanup)
        return process

    def test_vm_keeps_the_runner_pid_and_captures_both_streams(self):
        process = self.start()
        output, _ = process.communicate(timeout=5)
        self.assertEqual(process.returncode, 0)
        self.assertEqual(int(self.pid.read_text()), process.pid)
        self.assertEqual(json.loads(self.args.read_text()), ["-machine", "virt", "-m", "256"])
        self.assertEqual(self.capture.read_bytes(), output)
        self.assertIn(b"VM stdout", output)
        self.assertIn(b"VM stderr", output)
        self.assertEqual(self.status.read_text(), "0")

    def test_killing_the_runner_pid_kills_the_vm_and_finishes_capture(self):
        process = self.start(VM_HANG="1")
        until = time.monotonic() + 5
        while not self.pid.exists():
            self.assertLess(time.monotonic(), until)
            time.sleep(.01)
        self.assertEqual(int(self.pid.read_text()), process.pid)
        process.kill()
        process.communicate(timeout=5)
        self.assertEqual(process.returncode, -signal.SIGKILL)
        self.assertEqual(self.status.read_text(), "0")

    def test_capture_failure_is_not_a_success_marker(self):
        process = self.start(QEMU_TEE_LOG=str(self.root))
        process.communicate(timeout=5)
        self.assertEqual(self.status.read_text(), "1")

    def test_without_capture_the_vm_exit_status_is_preserved(self):
        process = self.start(QEMU_TEE_LOG="", VM_EXIT="7")
        process.communicate(timeout=5)
        self.assertEqual(process.returncode, 7)
        self.assertEqual(int(self.pid.read_text()), process.pid)
        self.assertEqual(self.status.read_text(), "P")


if __name__ == "__main__":
    unittest.main()
