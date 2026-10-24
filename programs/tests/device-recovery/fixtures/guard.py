"""Synthetic native evidence for the deployed guard-death verifier."""

CONSOLE = """guard: root task=41
system: minted pci tid=7
system: activate pci tid=7
pci: space task=7 ecam=0x29000 functions=1
pci: ready functions=1
system: minted iommu tid=8
system: minted recovery tid=9
recovery: ready hub=4 pci=7 requester=8 key=41
guard: armed target=pci task=7 team=3
iommu: binding 1 published requester=8 held=6 ddtp=0x1000
pci: endpoint bus-master on=0x4
recovery: binding 1 active iova=0x100000 execution=10
work: rang the in-flight request doorbell
guard: submitted controller=10
guard: critical domains protected=3
guard: ready target=pci task=7 team=3
guard: trigger target=pci task=7 team=3
[critical] trip task=7 task_team=3 team=3
[critical] guarded exit reason=0xfffffffe; no cleanup follows
[critical] peer harts quiesced=3
"""

HELD_CONSOLE = CONSOLE.split("guard: trigger")[0]
TRIGGER = "guard: trigger target=pci task=7 team=3\n"
READY = "guard: ready target=pci task=7 team=3\n"
QT = """test embedded::tests::scene ... FAILED
timed out after 15s
"""
HELD_EVENTS = [
    {"phase": "ready"},
    {"phase": "boot-read", "cookie": 1, "offset": 0, "length": 512},
    {"phase": "replied", "cookie": 1, "offset": 0, "length": 512, "error": 0},
    {"phase": "held", "cookie": 1, "offset": 0, "length": 512},
]
BLOCK = HELD_EVENTS + [{"phase": "disconnected"}]
FEED = {
    "sent_g": True,
    "held_before_trigger": True,
    "resume": False,
    "trigger": True,
    "serial_eof": True,
}
EVENTS = [
    {"h": 0, "when": 1, "kind": {"room": {"wait": {"tid": 7}}}},
]
