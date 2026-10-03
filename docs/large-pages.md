# 大页支持

连续映射根据虚拟地址、物理地址的对齐和剩余长度，选用 1 GiB、2 MiB 或 4 KiB 叶子页。已有下级页表时继续向下映射，不覆盖它。RAM 的身份映射和高地址直映、连续 Backing 映射自动使用这条路径；逐帧分配的堆与栈仍按 4 KiB 映射。

地址查询返回对应 4 KiB 页的物理基址，Space 再补页内偏移，原始页表查询也处理大页偏移和物理对齐。大页的虚拟与物理地址必须按页尺寸对齐，规则见 [RISC-V Supervisor ISA](https://docs.riscv.org/reference/isa/priv/supervisor.html)。

完整覆盖一个大页的 protect/unmap 直接操作叶子。局部覆盖先拆分两端所在的大页，保留物理地址和权限，再修改目标区间。Space 在修改 Map 元数据之前完成所需页表分配；分配失败可能留下等价的小页表示，地址和权限保持不变，可重试。回收空页表不再分配临时 Vec。内核刷新包含全局 TLB 项，并通知全部其他 hart。

## 验证

默认 Sv57 和强制 Sv39 各 26 个健康用例通过。没有 initrd 的 scene 用例按现有约定失败；另行带 product 镜像的 release scene 通过。

新增用例覆盖 1 GiB/2 MiB 映射、软件与原始页表查询、完整改权限不分配、局部改权限与挖洞、相邻页保持原样、跨页尺寸边界、已有下级页表、最高虚拟地址回收，以及拆分第二张页表时模拟内存不足后重试。分配器压力测试显式预热每个块尺寸档，再比较净占用，避免把首次保留的缓存页计为泄漏。

## 页表数量

运行 `python3 scripts/bench-map-index.py --tables`。脚本在独立临时快照中比较 `2225340` 与工作区，输出 `/tmp/sqware-page-tables/{before,after}.log`。板子参数来自 qemu-args.nu；默认 256 MiB、单 hart，debug 测试入口只初始化内核并读取 kernel Space 的页表数。

在 QEMU 默认 Sv57 下，页表由 **269 张降至 16 张**。每张 4 KiB，节省 **253 张 / 1012 KiB** 页表页。这里只计算页表页，不包括 TableNode 子节点 Vec 和 Map 元数据；未测实机 TLB 性能。当前不自动把拆开的小页重新合并成大页。
