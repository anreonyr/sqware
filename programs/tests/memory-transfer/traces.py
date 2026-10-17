"""结构化导出那一枚：内核真写出的 `room.wait` / `room.wake` / `room.exit` /
`room.fault_killed` / `memory.page_fault`。

它只回答"到这行为止读到过什么"，不替判定说话（门与断言都在 `harness` 里）。
两条纪律落在这里：
  · 固定名导出由 runner 在 QEMU 启动前清理；从头读取本轮事件，包括驱动启动前已写出的事件。
  · **只认完整行**，且每条完整行必须是 JSON：坏行当场失败（残缺的尾巴留到换行再说）；
    读过事件之后又发现文件被换掉 / 截断，那是把新旧拼起来，也当场失败。
"""
import json
import os


class Traces:
    """按行尾读的结构化导出面：`room` / `faults_at` 两个读口。"""

    def __init__(self, path):
        self.path = path
        self.events = []
        self.partial = b""
        self.inode = None
        self.offset = 0

    def poll(self):
        try:
            stat = os.stat(self.path)
        except FileNotFoundError:
            assert not self.events, f"{self.path}: the export disappeared after events were read"
            return
        if stat.st_ino != self.inode or stat.st_size < self.offset:
            assert not self.events, (
                f"{self.path}: the export was replaced or truncated after events were read"
            )
            self.inode = stat.st_ino
            self.offset = 0
            self.partial = b""
        if stat.st_size == self.offset:
            return
        with open(self.path, "rb") as source:
            source.seek(self.offset)
            chunk = source.read()
            self.offset = source.tell()
        self.partial += chunk
        whole = self.partial.split(b"\n")
        self.partial = whole.pop()
        for line in whole:
            try:
                self.events.append(json.loads(line))
            except ValueError as error:
                raise AssertionError(
                    f"{self.path}: a complete line is not JSON: {line!r}"
                ) from error

    def finish(self):
        """生产者退出后，末行也必须完整。"""
        self.poll()
        assert not self.partial, f"{self.path}: incomplete final JSON line: {self.partial!r}"

    def room(self, kind, tid):
        """那一类 room 事件里 tid 等于这一位的载荷（没有就是空表）。"""
        return [
            payload
            for event in self.events
            for payload in [room_payload(event, kind)]
            if payload is not None and payload.get("tid") == tid
        ]

    def faults_at(self, base, pages):
        """本阶段那一段 VA 上的页错误（含未结算与已结算）。"""
        span = pages * 4096
        return [
            payload
            for event in self.events
            for payload in [page_fault_payload(event)]
            if payload is not None and base <= payload["va"] < base + span
        ]


def room_payload(event, kind):
    room = event.get("kind", {}).get("room")
    if not isinstance(room, dict):
        return None
    got = room.get(kind)
    return got if isinstance(got, dict) else None


def page_fault_payload(event):
    memory = event.get("kind", {}).get("memory")
    if not isinstance(memory, dict):
        return None
    got = memory.get("page_fault")
    return got if isinstance(got, dict) else None
