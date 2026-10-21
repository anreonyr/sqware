"""Synthetic hardware records for negative controls, not native evidence."""

from .ruin import CONSOLE, EVENTS

DTS = """
/ {
  iommu@3010000 {
    compatible = "riscv,iommu";
    reg = <0x0 0x3010000 0x0 0x1000>;
  };
  memory@80000000 {
    device_type = "memory";
    reg = <0x0 0x80000000 0x0 0x10000000>;
  };
};
"""
PENDING = (
    "system: minted hub tid=4\n"
    "system: minted pci tid=6\n"
    "system: minted iommu tid=7\n"
    "system: minted device-ctl-fault tid=9\n"
    + CONSOLE.split("iommu: binding 1 invalidated")[0]
    .replace("ruin: requested recovery\n", "")
    .replace(
        "iommu: binding 1 published",
        "iommu: binding 1 acquired requester=8 pages=6 iova=0x200200000\n"
        "iommu: binding 1 published",
    )
    .replace(
        "work: rang the in-flight request doorbell\n",
        "ctl: served used=1 len=513 written_ok=true sector_ok=true status=0\n",
    )
    + "iommu: binding 1 invalidate fence expired frames held=6\n"
    "hub: binding 1 step failed party=3 op=3 code=4 (holding)\n"
)
REAPED = EVENTS[:2]


def proof():
    base = 0x3010000
    original = 0x23fbec07
    broken = 0x24000007
    cookie = 0x10f00fef
    destination = 0x8fefd004
    fence = (cookie << 32) | 0x3402
    queue = (original >> 10) << 12
    operations = []

    def op(command, at, value=None, reply=None):
        request = f"{command} {at:#x}" + (f" {value:#x}" if value is not None else "")
        operations.append({"request": request, "reply": "OK" + (f" {reply:#x}" if reply is not None else "")})

    op("readq", base + 0x18, reply=original)
    op("writel", base + 0x48, 0)
    op("writeq", base + 0x18, broken)
    op("writel", base + 0x48, 0x101)
    op("readq", base + 0x18, reply=broken)
    op("readq", queue, reply=1)
    op("readq", queue + 16, reply=fence)
    op("readq", queue + 24, reply=destination >> 2)
    op("readl", destination, reply=0)
    op("writel", base + 0x48, 0)
    op("writeq", base + 0x18, original)
    op("writel", base + 0x48, 0x701)
    op("readq", base + 0x18, reply=original)
    op("writel", base + 0x24, 2)
    op("readl", destination, reply=cookie)
    return dict(
        base=base, bad=0x90000000, original=original, broken=broken,
        fault=0x10101, head=0, tail=2, invalidate=1, fence=fence,
        destination=destination, cookie=cookie, before=0, after=cookie,
        operations=operations,
    )
