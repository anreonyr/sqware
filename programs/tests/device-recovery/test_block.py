"""Exercise the fixture's actual NBD socket protocol without QEMU."""

import json
import os
import pathlib
import select
import socket
import struct
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import block


class BlockFixture(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="sqware-nbd-")
        self.addCleanup(self.directory.cleanup)
        self.path = pathlib.Path(self.directory.name) / "disk.nbd"
        self.disk = self.path.with_suffix(".img")
        self.disk.write_bytes(bytes(range(256)) * 4)
        self.console = self.path.with_suffix(".console")
        self.console.write_text("")
        self.errors = []
        self.environment = patch.dict(os.environ, {"QEMU_TEE_LOG": str(self.console)})
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.server = threading.Thread(target=self.serve, daemon=True)
        self.server.start()
        until = time.monotonic() + 2
        while not pathlib.Path(str(self.path) + ".json").exists():
            if self.errors:
                raise self.errors[0]
            self.assertLess(time.monotonic(), until, "fixture did not listen")
            time.sleep(.005)
        self.client = socket.socket(socket.AF_UNIX)
        self.client.settimeout(2)
        self.client.connect(str(self.path))
        self.addCleanup(self.finish)
        header = block.exact(self.client, 152)
        self.assertEqual(header[:8], b"NBDMAGIC")
        self.assertEqual(struct.unpack(">QQI", header[8:28]), (0x00420281861253, 1024, 3))

    def serve(self):
        try:
            block.serve(self.path, self.disk)
        except Exception as error:
            self.errors.append(error)

    def finish(self):
        pathlib.Path(str(self.path) + ".resume").touch()
        self.client.close()
        self.server.join(timeout=2)
        self.assertFalse(self.server.is_alive(), "fixture leaked its server thread")

    def request(self, op=0, offset=0, length=512):
        self.client.sendall(struct.pack(">IHHQQI", 0x25609513, 0, op, 7, offset, length))

    def reply(self):
        self.assertEqual(struct.unpack(">IIQ", block.exact(self.client, 16)), (0x67446698, 0, 7))
        self.assertEqual(block.exact(self.client, 512), self.disk.read_bytes()[:512])

    def test_boot_read_then_held_dma(self):
        self.request()
        self.reply()
        self.console.write_text("recovery: binding 1 active iova=0x100000 execution=10\n")
        self.request()
        state = pathlib.Path(str(self.path) + ".json")
        until = time.monotonic() + 2
        while json.loads(state.read_text())["phase"] != "held":
            self.assertLess(time.monotonic(), until)
            time.sleep(.005)
        self.assertEqual(select.select([self.client], [], [], .05)[0], [])
        pathlib.Path(str(self.path) + ".resume").write_text("release\n")
        self.reply()
        self.client.close()
        self.server.join(timeout=2)
        self.assertEqual(self.errors, [])
        events = [json.loads(line) for line in pathlib.Path(str(self.path) + ".events").read_text().splitlines()]
        self.assertEqual([event["phase"] for event in events], [
            "ready", "boot-read", "replied", "held", "resuming", "replied", "disconnected",
        ])

    def test_write_is_rejected(self):
        self.request(op=1)
        self.assertEqual(self.client.recv(16), b"")
        self.server.join(timeout=2)
        self.assertIsInstance(self.errors[0], AssertionError)

    def test_out_of_bounds_read_is_rejected(self):
        self.request(offset=1024)
        self.assertEqual(self.client.recv(16), b"")
        self.server.join(timeout=2)
        self.assertIsInstance(self.errors[0], AssertionError)

    def test_truncated_header_is_not_normal_disconnect(self):
        self.client.sendall(b"short")
        self.client.shutdown(socket.SHUT_WR)
        self.assertEqual(self.client.recv(16), b"")
        self.server.join(timeout=2)
        self.assertIsInstance(self.errors[0], ValueError)


if __name__ == "__main__":
    unittest.main()
