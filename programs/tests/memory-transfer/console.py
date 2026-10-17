"""串行那一枚：本景自己打的口径 ＋ 宿主那一枚门。

它只管一条字节流：一行一行地认（阶段名 / tid / VA / View 身份），对端走了也不丢掉已经
到的字节；门就是往同一条流上回一枚字节。判定不看这一面的话都由 `harness` 拿。
"""
import socket
import time

PREFIX = "memory-transfer: "


class Console:
    """一条串行连接：收到的字节原样追进捕获文件，并按行交出去。"""

    def __init__(self, conn, capture):
        self.conn = conn
        self.capture = capture
        self.pending = bytearray()
        self.seen = []
        self.closed = False

    def _tee(self, data):
        with open(self.capture, "ab") as output:
            output.write(data)

    def _pull(self, seconds):
        """读一批（最多等 seconds）；对端已经关了就置 `closed`。"""
        if self.closed:
            return
        self.conn.settimeout(max(0.0, min(0.25, seconds)))
        try:
            data = self.conn.recv(65536)
        except (socket.timeout, BlockingIOError):
            return
        if not data:
            self.closed = True
            return
        self.pending.extend(data)
        self._tee(data)

    def _next(self):
        """缓冲区里下一个非空行；没有就答 None。"""
        while b"\n" in self.pending:
            at = self.pending.index(b"\n")
            text = bytes(self.pending[:at])
            del self.pending[: at + 1]
            if text:
                self.seen.append(text)
                return text
        return None

    def line(self, seconds):
        """下一条完整的行；seconds 内没来、对端又已经走了，就当场失败。"""
        end = time.monotonic() + seconds
        while True:
            got = self._next()
            if got is not None:
                return got
            if self.closed:
                raise AssertionError(f"the guest went away early; lines={self.tail()!r}")
            left = end - time.monotonic()
            if left <= 0:
                return None
            self._pull(left)

    def drain(self):
        """把已经在线上的字节收进来（不卡住判定环）。"""
        self._pull(0.0)

    def tail(self):
        return [line.decode("utf-8", "replace") for line in self.seen[-8:]]

    def send(self, byte):
        self.conn.sendall(byte)
