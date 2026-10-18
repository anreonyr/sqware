# Transfer / Traverse 追加实施计划

参考基线：`75ef237326461a0a29dce1191240de82b623057b`。
启动检查点：本地 `b267007b`（保留机制基础，未推送）；远端仍为参考基线。
本计划追加到 Flat / Nest 迁移计划，不把未完成的 IOMMU 产品接线记作已完成。

## 1. 已确认边界

| 原语 | 效果 | 不做的事 |
|---|---|---|
| Transfer | 将同一个 Frame 从一个合法逻辑页槽移交给另一个空槽 | 不复制数据，不改变 Flat/View 身份、VA、Access，不导出 PA |
| Traverse | 在有效授权与驻留保证下输出来源窗口的逐页 PA | 不接裸 PA，不增设备访问权，不移交 Frame，不改变 CPU 映射 |

这是并列的遍历与移交，不是 acquire/release 式的逆操作。
不引入 `SwapIn/SwapOut`、`PieKind::Frame`、`ResolveFault` 或第二套映射对象。
存储位置、换页策略、设备页表和 IOVA 管理不进入 Transfer。

槽号：5–9 保持不变；10 留给 Transfer；11 使用 Traverse。
未提交候选 `Phys` 改为 Traverse，不保留 `Phys` 别名，不复活 0–4。
10 的线上入口必须等迁移安全条件齐备才开放，不能先返回虚假的成功。

## 2. 物理驻留与地址出口

`FlatRange` 表示逻辑来源窗口，不独自保证可迁移体系中的物理地址稳定。
引入内核内部 `Residency`，登记该窗口的迁移排除；记录与页槽受同一 Flat
状态保护，不能先读 pin 数，再在另一把锁下开始迁移。

- `Pages` 持有 Residency，且只在完整物化后构造。
- 独立 retention 持有 Residency，跨原 Grant 撤销继续固定已有承载。
- Fresh 可先被 retention 覆盖，不强迫整段提前分配；正常首次物化仍允许。
- Missing 不得因取得驻留或 Traverse 被清零恢复；Moving 与新驻留互斥。
- 第一版 Transfer 的源、目标被驻留覆盖时都返回 Busy。
- 驻留按持有生命周期解除，没有“超时自动释放物理页”。
- 只有最后一份实际持有消失才解除迁移排除；引用表摘除不能越过在途内核使用。

Traverse 保留现有定长 ReferenceRange 快照方式：

```rust
Traverse {
    retention: PieToken,
    range: VirtAddr,
    out: VirtAddr,
}
```

Transfer 的线上选择保持提案的四个量：

```rust
Transfer {
    from: FlatReference,
    from_page: usize,
    to: FlatReference,
    to_page: usize,
}
```

两端页号都相对各自引用窗口，不是 Flat 绝对页号。已有错误 -1 至 -7 保持不变；
SourceEmpty、DestinationOccupied、NotTransferable 作为新增语义，不能偷用旧编号。
驻留、竞争事务与尚未结算的同步使用可重试失败；编号与域错误映射随 T4 一起落实。

当场验证活的内核保留 cap、VEST、真实保留类与 owner；同时验证调用者自己
引用表中的 retained 引用及其窗口。两者是当前权限与已有持有两个事实，
不额外存储“Acquire 当时是哪枚 cap”的副本。

输出每页一个 8 字节小端 PA。count 唯一决定写入量，raw 调用者负责缓冲容量；
内核验证完整输出地址区间可写，execution 包装核对切片长度。允许未对齐输出，
按字节编码。全部物化、元数据与编码完成后才一次 copy_out。
输出失败不改前缀；已经成功物化的页仍由原 Flat 持有。
PA 描述只在相应驻留责任有效期间使用；它不是设备访问授权。

## 3. 页槽、别名与物理访问

Fresh 表示从未物化；Resident 表示拥有 Frame；Missing 表示原承载已移出；
Moving 表示事务保留。存储可以稀疏，不强制每页常驻元数据。

第一版两端只接受普通 Flat::acquire 来源。from_frames 与 Borrowed 明确拒绝。
仅有构造路径检查仍不够：普通分配 Frame 后来被 from_frames 导入其他 Flat 时，
必须使原 Frame 也失去迁移资格，并与正在准备的 Transfer 同步。
不得以 Arc::strong_count 作为独占证明。

必须审查所有 PA 消费：Pages、发布准备、copy_from/copy_to、内部参数写入、
调试快照和独立 retention。基础 DRAM 直映窗口不是一次具体的物理使用；
使用者不得借它长期保存未受驻留保护的可迁移页地址。

## 4. 完整发布关联与 unpublish

全部核心 Nest::attach 路径都必须可定位，不只依赖 Pend 的 ViewRegistry
或 SourceAuthorization.spaces。来源到 View 的反向关联属于 Nest 层；
Flat 不依赖 Nest、Task、VA 或设备。

内部逐页 unpublish：

1. 提前准备大页拆分、退休资源与待同步元数据。
2. 只撤该页 PTE，并结清 View.published 中对应项。
3. 保留 ViewId、Reservation、active、VA、ceiling/current 与架构属性。
4. 锁外完成所有相关 ASID shootdown；失败保留全部责任。

不得调用 detach 模拟迁移。目标可在提交后按需发布，不要求 Transfer 立即映射。

## 5. 事务与管理授权

Transfer 选择均相对本 Task 的引用窗口，经 checked arithmetic 和 FlatRange::range 校验。
私有来源核合法私有控制者；Pole 来源核该来源记录的根 token、根 Grant 和实际 Pole
身份。FETCH/STORE/VEST/ONLY、View 控制权和 Task 数字都不能代替迁移管理权。
两端分别核验；不开放任意第三方 Pager 管理委托。

事务按准备、阻止新使用、全部 unpublish、同步确认、归属提交、结算执行。
源 Frame 在提交前仍归源逻辑页；目标必须为空且已被同一事务保留。
提交不分配数据 Frame、不复制内容、不做用户回调。
元数据和拆大页所需页表可以在准备阶段分配。

同 Flat 只锁一次；不同 Flat 需要明确的稳定双锁顺序。
现有锁检查禁止同级嵌套，必须表达有序双锁或等价的短提交协议，
不得关闭检查，也不得在 Flat 锁内等待远端 Hart。
迁移保留独立于授权永久撤销，不复用 pause_publish/resume_publish 的布尔闸门。

shootdown 失败不放走 Frame，也不依赖调用 Task 存活来保管事务。
建议失败返回后内核只结算安全撤销并回退迁移保留，不后台暗中完成归属移交；
具体恢复所有者与提交协议须在开放 ABI 前核准。

## 6. 实施顺序与验收门

| 阶段 | 交付 | 验收门 |
|---|---|---|
| T1 驻留基础 | Flat 范围驻留、Pages 与 retention 接入，Traverse 替换候选 Phys | 重叠窗口、独立持有、摘表后在途持有、失败净零；PA 输出授权与原子性 |
| T2 无映射迁移 | 稀疏页槽状态、受限物理资格、事务准备与双向移交 | 同 Frame/PA/内容，无新增数据帧；源 Missing、目标 Resident；别名与 pin 拒绝 |
| T3 Nest 撤销 | 完整关联、逐页 unpublish、大页拆分与同步恢复 | 多 View/多 Nest，身份与权限不变；注入失败不提前提交或回收 |
| T4 ABI 与授权 | Transfer slot 10、两端管理权及最小缺页等待 | 子 Pie/越权/撤销/错 Task 拒绝；合法 Moving/Missing 不被清零或当非法杀死 |
| T5 端到端与竞争 | Pole 管理者、应用 View、多 Hart、Task 退出与 Revoke | 旧 PTE 不重发，失败不重复提交；Frame/页表/事务净零 |
| T6 业务组合 | Page Service、Room/Mail 通知及存储恢复 | 不给 Transfer 增业务完成协议；无丢失唤醒 |

T4 前要有最小状态等待/重查/唤醒；T6 可延后的是业务通知和存储组合，
不是允许正常迁出页缺页时杀 Task。现 resolve_fault 的 bool 结果不足以表达等待。

验收不能只看“调用成功”：

| 轴 | 必测反例与成功证据 |
|---|---|
| 身份 | 双向迁移同一 Arc<Frame>/PA，逐字节内容不变；数据帧分配数不增加 |
| 关系 | 多 View、多 Nest、切片、活动洞、大页中的 4 KiB 页；ViewId/Reservation/VA/Access 不变 |
| 权限 | 两端分别越权、普通子 Pie、只有访问权限、Acquire(View)、撤销后普通引用、错 Task 句柄 |
| 驻留 | Pages、独立 retention、在途内核拷贝与发布；重叠窗口与最后持有解除，不能只看 Arc 计数 |
| 别名 | from_frames 两端拒绝；A 来自 acquire、其 Frame 后来导入 C 时，从 A 迁出也必须拒绝 |
| 竞争 | Transfer 对 Transfer/Attach/Detach/SetAccess/materialize/缺页/Revoke/Task 退出/驻留取得 |
| 同步 | 多 Hart 旧 TLB、逐 Nest 注入失败、调用 Task 退出；失败不提前提交、不重发旧 Frame |
| 回收 | 大量往返、元数据耗尽、失败重试；Frame、页表与事务记录净零，不放宽最终回收断言 |

## 7. 与原 PR7 的衔接

IOMMU 仍在用户态负责设备翻译。当前保留 cap 按收件 Task 绑定，只给 System；
不假设把它转交 IOMMU/DMAC 就能使用。第一版可由 System 取得驻留与物理描述，
通过既有供给路径交付获授权信息；Driver/DMAC 只拿必要 BAR 与 IOVA。

正常停止并同步确认后才能解除驻留；故障时 System 独立隔离。
DDTP Off 与 CQ/FQ/PQ 停用分别确认；IOFENCE 完成页自身也必须被保留，
队列关闭不能冒充该页上的在途 store 已结束。
无法确认就保留并报告，不自动重开或复用旧 IOVA。

精确 BAR 子能力派生仍是单独待确认的权限扩展；本计划不放宽 create_pole，
不允许 borrowed/子 Pie 创建独立根，不转授整 aperture 绕过最小授权。
真实 Platform 与产品接线尚未完成，不能用本计划或机制用例代替硬件验收。

引导设备的既有 `reg` 与新增 PCI `ranges` MMIO 窗口统一接入来源授权：
原 token 持有者可取得自己的来源子窗口，ceiling 为 READ | WRITE，不含 EXECUTE。
这项接线不增加持有主体或 token 权限；它不是把窗口子能力派生给另一主体，
也不解决精确 BAR 跨主体委托。设备区域构造或授权失败向上传播，启动不静默跳过。

## 8. 验证命令与记录纪律

以真实使用路径为准：`nu scripts/qtest.nu --package kernel`，以及
`--scene accept`、`--scene product`、`--scene system-fault`。
缺 initrd 的 scene 哨兵单列，不称全绿；真实设备场景另行增加并实际运行。
宿主测试显式使用 x86_64-unknown-linux-gnu；不全树格式化，不放宽结构或净零断言。
每一阶段记录实际实现、未完成项和独立审查结果，安全门未过不推送为功能完成。

## 9. 执行记录

- 计划落盘时：Transfer 尚未实现。旧 Phys 候选及其基础件修复已作为工作区输入，
  不表示它们已经获最终审查或完成产品接线。
- 本轮从 T1 执行，不提前开放通用 Transfer。
- T1 当前实现：FlatState 在同一状态锁内保存页槽来源与范围驻留登记；
  Pages 和 Hold.retained 改为实际持有 Residency，保持 Fresh retention 不提前物化。
  Traverse 已取代候选 Phys（无兼容别名），slot 10 尚不解码。
- 新增真实内核用例：窗口重叠、独立持有与边界；Pages 最后持有解除；
  引用表 remove/clear 后在途 Arc<Hold> 继续持有；借用页不变成 RAM；
  物化失败/重试释放临时驻留。原逐类净零与帧净零断言保留。
- T1 检查点时 T2–T6 未实现。反向关联、临时发布与内核 PA 使用还不能据此宣布已具备迁移安全；
  Transfer 必须等这些路径纳入同一排除与同步规则后才能开放。

### T1 实跑证据

| 检查 | 结果 |
|---|---|
| `nu scripts/qtest.nu --package kernel` | 62 passed；缺 initrd 的 scene 哨兵 1 failed，退出 101；Traverse 与 Flat 的 15 个子用例通过 |
| 同命令 `--scene accept` / `product` / `system-fault` | 各 1 passed，10.29s / 5.33s / 13.34s |
| `cargo test -p env --target x86_64-unknown-linux-gnu --offline` | 24 passed；包含 Traverse 参数编码；既有 signals unused Wire 告警 1 |
| Pend 宿主套件（显式 x86_64、offline） | 37 passed；创建资格旧断言保留 |
| retention / system-shape 宿主套件 | 6 / 18 passed |
| workspace check dev / release；kernel all-targets semihosting dev / release | 通过；programs 保留面 10 个未接线告警，kernel release 8 组未使用诊断，不称无告警 |
| `cargo fmt --all -- --check` | 退出 1，57 块剩余格式差异；只格式化本次相关文件，没有全树重排 |
| `git diff --check` | 通过 |

本阶段没有重新运行其余 programs 宿主套件、alloc-probe 或真实 DMA 场景，
没有开展独立复审，没有提交或推送。上述证据不覆盖尚未实现的 Transfer。

### 后续授权与补充基线

用户授权代理组自行裁决并继续执行；本轮新建独立 namer / worker / reviewer，
均使用 DeepSeek V4.1 Flash high。worker 是当前阶段唯一代码写入者；
namer 收敛 T2–T4 合同，reviewer 先独立复审 T1，再审后续实际检查点。
父代理负责裁决、集成与最终验证，不把审查启动写成审查通过。

- 全部 `programs/tests/*/Cargo.toml` 宿主套件已补跑：
  `cargo test --manifest-path <manifest> --target x86_64-unknown-linux-gnu --offline`，
  31 套共 265 passed，保留既有未使用诊断。
- alloc-probe 在自身目录使用 `CFLAGS=-std=gnu17`、显式宿主 target 与 offline：
  all-targets check 通过，串行 `--lib` 6 passed，
  `--no-default-features -- --test-threads=1` 为库 6 + 多线程 4 passed。
  默认特性运行仍输出 orphaned-buffer 诊断；这不是无诊断验证，
  也没有重跑默认并行模式来替代已有失败记录。
- 这些补充结果针对本轮输入检查点，不覆盖尚未集成的 T2 或真实 DMA。

### T1 独立复审

reviewer 未发现驻留、Traverse 授权与输出原子性的阻塞缺陷；独立重跑 kernel
check、健康面（62 passed + 缺 initrd 哨兵 1 failed）、accept/product/system-fault
三个 QEMU 景、env 与 Pend 宿主套件、相关文件格式和 diff 检查。
这些是 QEMU 验证，不是真实设备或 DMA 验证。

后续裁决：

- 保留全部既有设备 `reg` 的同主体 RW/NX 来源接线，边界见第 7 节；
  不将它称为跨主体 BAR 委托。
- Fresh 仍可由 Traverse 首次物化；T2 必须让 Missing 明确失败，Moving 可重试，
  不能把所有非 Resident 状态都清零，也不能把 Missing 一律折成 Busy。
- 不合并 Residency 与内部登记记录到同一文件，不吞掉内部持有不变量错误。
- PCI 父节点定位不能仅靠同名与相同 `ranges` 内容：该项需要消除歧义并加回归用例。
- T2 初案的“仅注释要求无映射”不构成核心安全边界，未获批准。
  双 Flat 提交必须有可辨认的单一归属线性化点，并接受 Fresh 或 Missing 目标。

### T2 实施合同

本阶段只允许无映射迁移，但“无映射”由核心排除规则证明，不交给调用方承诺：
每个 View 从构造至最终释放持有整个来源窗口的 Residency。全部核心 attach 经过
View 构造；发布准备、有效 PTE、待 shootdown 的旧 TLB 与在途内核拷贝必须始终
被这个持有覆盖。View 残余强引用会保守延长排除，不会提前放走承载。
失败同步及 Nest 销毁的持有链必须经独立审查与注入用例证实。

同步移交在同一个提交临界区内完成：同 Flat 取一次状态锁；跨 Flat 使用按地址
排序的显式双锁设施。普通 lock 的同级拒绝规则不变，第二把 Flat 锁只在受控
双锁入口获准；禁止递归、逆序和第三把同级锁。闭包封闭临界区，不让双守卫
逃逸或按错误顺序恢复中断。锁内不做 shootdown、用户回调或等待远端 Hart。

先保留实际需要的稀疏状态：缺席为 Fresh，槽中是 Resident 或 Missing。
准备目标元数据、校验两端来源及驻留、取得 Frame 迁移托管后，才原子移交；
目标 Fresh 或 Missing 均可接收。Missing 不物化、不清零。
异步撤销所需 Moving、事务标识和回退记录留到 T3，不为同步 T2 预造六个
半提交原语或允许源页永久 Busy 的中间状态。

from_frames 明确标记导入来源；帧上的原子托管同时阻止“普通分配帧后来又被导入”
与移交的竞争，不能靠 Arc 计数。构造失败的所有权与保守别名标记必须如实定义。
Flat 不反向依赖 Nest；可执行核心移交的安全边界是上述真实驻留，
不是 pub(crate) 可见性或目前只有测试调用。

本合同不开放 slot 10，也不声称已完成有映射迁移。T3 以后才以完整关联发现、
短期发布驻留和逐页 unpublish 替代 View 全寿命驻留，解除这个保守限制。

### 引导窗口修复（已集成）

PCI 父节点通过同一 DTB 内 `ranges` 属性切片的地址与长度定位，不再把同名或
相同属性内容当成节点身份；device_type 经字符串解析后精确匹配 `pci`。
新增两份可由 DTS 重建的真实 DTB 夹具，覆盖不同父单元宽度下内容相同的
`ranges`，以及 `pci` / `pcie` / `pci-something`。

- worker 回填旧规则运行健康面，`boot_pci_ranges_windows` 失败：
  `/real` 被错按另一父的 6 格布局解析；修后按自身父的 7 格布局得到四个窗口。
- reviewer 独立读取冻结源码与夹具，用自己的 fdt 宿主 harness 复现旧错新对，
  批准该修复 delta；没有在旧隔离树冒称重跑新代码的完整景。
- 集成后父代理运行 `nu scripts/qtest.nu --package kernel`：
  62 passed + 缺 initrd 哨兵 1 failed，退出 101（5.89s）。
- worker 的 `--scene accept` 1 passed；集成后父代理的 `--scene product` /
  `--scene system-fault` 各 1 passed（5.33s / 13.45s）。
- workspace check、kernel all-targets semihosting check、diff check 通过；
  programs 既有 10 组未使用诊断仍在。修复尚未提交或推送。

### T3 / T4 已确认的安全合同（尚未实现）

- T3 移除 View 长驻留时，必须同时删除 T2 的直接 Flat 迁移组合；
  唯一可执行入口位于 memory 组合层，恒经完整关联枚举、逐页撤销与全部同步确认。
  Flat 的占槽、提交和回退只对 memory 内部开放，不提供无证据旁路。
- 源 Moving 槽继续强持 Frame，目标 Reserved 槽记录先前 Fresh/Missing。
  内部 RAII 守卫只持事务身份与两端来源；提前返回或 `?` 自动原子回退，
  成功提交消费守卫。panic=abort 不保证 Drop，异常残留必须靠槽持帧而安全阻断，
  不能依靠日志或 unwind 回收。
- 失败不移交承载：同次调用恢复源 Resident 与目标原态，各 Nest 继续保管自身
  未完成同步。不会在后台替调用者提交归属。View 仍存但原始 Nest 不可升级时，
  永久保守拒绝并记录原因，不能把失联当成无旧 TLB。
- 关联表只做完备发现，全部核心 View 在首次发布前登记；迁移先占槽再枚举。
  发布在抓 Frame 快照前取得短期 Residency，覆盖 PTE 和 published 记录提交。
  不以 Arc 计数、弱引用过期或空 published 列表单独充当同步证明。
- 条件等待由 Room 适配，core memory 不依赖 work。最外层 work 提交锁覆盖
  无 Site 锁时的条件短读与完整入队，随后释放，再执行 finish_block/切换。
  不能持 Site 锁读取更低级的 Nest/Flat，也不能持提交锁等待远端 shootdown。
- 通知在页状态更新后整链放行；knock 自己取得同一 work 提交锁即可，
  不要求整个 memory 迁移持该锁。填充若早于谓词则不睡，若晚于谓词，
  通知会等待入队完成。单比特 beacon 不是多等待者条件正确性的依据。
- 醒后重查 View 身份、当前权限与真实页状态；释放关系和关闭空间也必须结束等待。
  不添加只为诊断的 Flat 世代计数。具体缺页结果、等待 key 与通知名单仍待实现前审定，
  尤其要避免“归属已提交后分配唤醒名单失败”和遗漏新关系的等待者。

### T2 初次代码检查点

已集成稀疏 Resident/Missing、来源 Imported 标记、Frame 原子 Custody、
View 全窗 Residency 与显式双锁同步移交。当前执行对象接收两个已校验的范围及
相对页号，执行时在同一锁边界重核页槽、两端驻留、来源与 Frame 资格；
缺席目标先备容量，取得独占之后不再经过可失败步骤。slot 10 仍不解码。

初次集成时，worker 自验为健康面 63 passed + 缺 initrd 哨兵 1 failed，accept/product/system-fault
各 1 passed；kernel check dev/all-targets、release、workspace check 及
Pend/frame-runs/trace/console-line 宿主套件通过。这些数字是 worker 检查点证据，
后续独立复验与集中返工结果见下。

父代理首轮检查要求集中返工：

- 第二把锁的取用口只能留在 sync 内部，不能公开返回守卫绕开闭包析构边界。
- 归还 Frame 托管必须验证 InTransit 不变量，不吞 CAS 失败。
- 不为测试扩大未持驻留的生产裸 PA 接口；测试应经过真实持有或模块内白盒。
- 补足双锁拒绝路径、导入撞 InTransit 及失败前缀的保守别名行为等合同负例，
  不把“既有 qtest 没有触发锁序报错”当作这些反例已经通过。
- 源码仅描述当前约束，不以未来 T4 接线或当前唯一测试调用者解释生产结构。

初次检查点未获最终复审。T3/T4 合同不等于实现，有映射迁移仍不存在。

### T2 收口（通过，继续 T3）

上述返工已完成。第二把锁的获取口与未持有的 PA 读取口收回内部可见性；
托管归还无条件执行 CAS 并验证不变量；锁序检查与自检复用同一纯判据；
物化恢复单调游标。双锁两种参数序各自独立核对交出顺序、持有集及最终 SIE。
搬运成功用例逐字节核对整页花纹；中段 Missing 与重叠驻留均有负例。
worker 曾故意放宽 strict/peer 规则，自检分别失败，还原后通过。

Custody 白盒移到独立宿主台架，test→core include 同一生产文件，不保留
core→tests 依赖。批量登记只有一个真实实现；用全新 Sole 前缀验证中途 Busy
后的永久别名标记，并验证在途拒绝、归还与幂等。worker 曾将登记改为空成功，
宿主 2/3 用例失败，还原后通过。native 中普通分配帧事后被导入的拒绝断言保留。

独立 reviewer 实跑初次 T2 的完整健康面、三个整机景及宿主回归；
集中返工与最后分层修正由其只读复审批准，未将只读复审写成再次实跑。
父代理对最后集成树实际运行：

| 检查 | 结果 |
|---|---|
| `nu scripts/qtest.nu --package kernel` | 63 passed + 缺 initrd 哨兵 1 failed，退出 101（5.33s） |
| 同命令 `--scene accept` / `product` / `system-fault` | 各 1 passed（10.34s / 5.28s / 13.40s） |
| `cargo test --manifest-path kernel/tests/custody/Cargo.toml --target x86_64-unknown-linux-gnu --offline` | 3 passed |
| kernel all-targets、semihosting、offline，dev / release | 通过；dev 无诊断，release 23 组未使用诊断 |
| `git diff --check` | 通过 |

删除整个 transfer 模块的 dead_code 豁免，接受 release 的未接线诊断并如实落数；
23 组不是“无告警”，也不表示已有生产 Transfer 调用者。此前父代理实跑的
env 24、programs 31 套 265、alloc-probe 串行及 worker/reviewer 的 Pend 37、
frame-runs 9、trace 2、console-line 5 证据保留，但未冒称每次小修都重新跑全套。
全树格式检查仍因既有漂移退出 1，没有全树重排。

本阶段可封存为本地基础检查点，不作为有映射迁移或硬件产品完成。
slot 10 仍拒绝；T3 已交同一 worker 继续，尚未集成。未推送共享分支。

### 检查点与 T3 进度

T2 已封存为本地提交 `e53980793a86ddc7dba7ee9bec367fb15bcc35bc`。
未推送；远端参考基线不变。提交包含此前未提交的驻留、Traverse、引导窗口修复
与无映射迁移，不把原 PR7 的硬件产品接线记成完成。

T3 的来源关联第一增量已集成但尚未批准：全部核心 commit 在首条 PTE 前登记，
View 最终释放时清理；长期 View 驻留与 T2 迁移门禁仍在，没有提前放开映射中移交。
worker 自验新增用例后为健康面 64 passed + 缺 initrd 哨兵 1 failed，三景各 1 passed。
这些读数不代替下列反例的审查：

- 索引锁内为身份比较而升级 Weak，临时强引用可能因并发释放变成最后持有，
  析构 View 时重入同一索引锁。去重必须直接比较弱引用身份，不能锁内丢临时强引用。
- 仅在全局 Vec 为空时归还容量，仍会在一条长期存活 View 存在时留下瞬时扩容的
  高水位。要求补“长期关系 + 大量瞬时关系”的实际回收用例并改成按记录释放。
- 活关系快照必须先准备完整容量，再升级引用；不能在已经持有部分强引用时
  分配失败并于索引锁内析构这些引用。

这组问题已交同一 worker 与下一原子增量一起修复。下一批必须同时替换长期驻留、
直接迁移旁路与发布门禁；逐页撤销和同步确认尚未实现前，已映射页仍然拒绝迁移。

### T3 准备检查点与交接

来源关联已改为逐条 Box 链，去重直接比较弱引用身份，查询在完整准备容量后才
升级引用并移入返回快照。键改从调用者强持的 FlatRange 内部取得，不接受跨来源
生命期缓存的裸身份；快照需在不持 core 锁时释放。
原 Vec 高水位与锁内临时强引用的问题在实现上已修，但对应压力测试仍需增强：
“8 轮逐个瞬时关系 + len”不证明容量回收，要求同时大量在场并用真实分配器净额验收。

槽态已增加 Moving（源槽强持 Frame）与 Reserved（保留 Fresh/Missing 原态）。
驻留取得、物化与 PA 读取拒绝在途窗口。当前执行仍在一次短临界区内完成，
尚无跨锁外阶段的 RAII 编排、短期发布驻留或逐页 unpublish；长期 View 驻留仍保留。
父代理在集成检查点实际运行 kernel all-targets semihosting dev check 通过，
健康面 64 passed + 缺 initrd 哨兵 1 failed，退出 101（5.23s）。

旧 worker 因上下文耗尽已停止写入。已验证交接输入为
`63e8dfa7-a2a4-4ca8-b170-f78e1122f42b.8538379`；
其后另报告的 `5216c64e-f30f-44ec-8d4f-7dd42e285cba.8542492` 尾部改动
只过类型检查、未跑测试，未集成（包含删除目标回退函数）。
确认旧 worker 停止后才启动新上下文 worker，不让两者并行写本阶段。
新 worker 只接已确认合同、真实代码检查点与剩余问题，先完成事务/发布边界的原子替换。

### T4 管理授权边界（已确认，尚未实现）

管理权验证与归属提交不能分开：调用者 pend 串行锁从取得两端 Hold 一直保持到
迁移提交或回退完成。Revoke、release、根 Grant 收窄、adopt、Task 退出的资源清理
均须经过同一调用者的串行边界；不能用“Frame 仍保活”代替授权线性化，
也不能靠提交前再读一次 live 标志修补 TOCTOU。

- 私有来源核 phase、owner、调用者 live 与来源未撤销。
- 根来源核当前调用者自己表里的实际根 Pie，与来源记录的 Pole、根 token、
  根 Grant、来源授权均为同一身份，且 Grant/Pole/来源仍活。
  根权限合法收窄为非 FETCH 的非空集合，不会仅因此丢失管理身份；
  不额外发明 FETCH/VEST/ONLY 管理位。
- 两端分别核验与校验相对窗口；同根可复用 PoleOperation，但不能略过另一端验证。
  PoleOperation 只作额外 try 型串行化，closing 会绕过它，不能独自承担授权边界。
- 复用唯一根身份来源，不新增平行 root_identity 或空 Manager 包装。
  页偏移只换算一次，不能先选一页再对这一页重复应用原偏移。
- 不持 work 提交锁跨迁移或 shootdown，不在持 caller.pend 时取得别的 Task 的
  串行锁或资源表内锁。完成归属修改/回退后先放 caller.pend，再整链通知等待者。

以上为后续 T4 的实现合同，不表示 slot 10 已开放或用户缺页等待已接通。

### T3 事务与发布边界原子替换（已实现，通过复审）

新上下文 worker 的模型请求失败，但留下了可编译的实现检查点
`29214ec4-fc66-4512-bf91-1c7e23a01ae7.8577051`。
该检查点真实 qtest 为 62 passed / 3 failed（其中一项是缺 initrd 哨兵），
没有作为通过成果接收。父代理接手修复，不再与失败 worker 并行写入：

- 事务守卫只在两端占槽成功后构造。此前在可失败校验之前构造带 Drop 的守卫，
  前置条件拒绝时会错误回退根本没占的槽。
- 提交以正常消费守卫、关闭回退责任的方式结束，字段照常释放。
  不使用 ManuallyDrop 跳过整个对象析构，避免永久泄漏两端 Flat 引用。
- 压力测试把建空间自带的关系与待测关系分别取基线；160 条瞬时关系同时在场，
  在空间构造之后精确核 160 条新增登记，退尽后帧池净占用及逐 Kind 严格相等。
- 回退目标墓碑用例先实际执行 0→1 的移交，再验证第 0 页 Missing；
  不再把已接收承载的目标错当墓碑。原失败与净零断言保留。

当前唯一执行入口在 memory 组合层，旧 flat::Transfer::execute 已移除。
内部事务跨锁外阶段保留 Moving/Reserved，并以 RAII 在错误返回时还原两端。
两个发布路径在抓承载快照前取得短期 Residency，覆盖 PTE 与 published 记录提交；
View 长期驻留已同时移除。来源关联门禁仍对活动关系及未确认失效保守拒绝，
失联 Nest 也拒绝；尚未逐页 unpublish，不声称能成功迁移映射中的页。

父代理在修复后实际运行：

| 检查 | 结果 |
|---|---|
| `nu scripts/qtest.nu --package kernel` | 64 passed + 缺 initrd 哨兵 1 failed，退出 101（5.39s） |
| 同命令 `--scene accept` / `product` / `system-fault` | 各 1 passed（10.39s / 5.28s / 13.44s） |
| kernel all-targets semihosting offline，dev / release | 通过；release 33 组未接线等未使用诊断，不隐藏 |
| workspace dev check | 通过；programs 既有 10 组未使用诊断 |
| custody / Pend / frame-runs / trace / console-line 宿主套件 | 3 / 37 / 9 / 2 / 5 passed，均显式 x86_64、offline |
| 本次相关文件 `rustfmt --check` / `git diff --check` | 通过；没有全树重排 |

独立 reviewer 确认事务构造/提交生命周期与短租约的成功、错误析构锁序，
同时发现来源门禁还存在以下两项，现已修复并复审关闭：

- 原先分两次读“同步干净”和“活动范围已清”，并发撤页可能让两次读来自不同
  时间点。现在以同一 Nest 状态守卫读取失效状态和 View 覆盖，锁序与所有覆盖写点
  一致（Nest → View），不能拼出未确认 TLB 已消失的假结论。
- 可重试 Blocked 不再抢先返回；任何位置的失联属主都优先得到永久拒绝。
  新用例让同一来源同时具有活动映射和失联关系，连续两次枚举覆盖相反候选顺序。
  父代理临时恢复错误早返，新增断言真实失败（63 passed / 2 failed，含哨兵），
  还原后健康面为 64 passed + 哨兵 1 failed，退出 101（5.48s）。

撤页测试还明确构造“起始 Clean、撤下后活动页成洞但失效仍未确认”的真实状态，
移交必须 Busy；该证据是单 Hart 状态交错，不冒称 T5 的多 Hart 撕裂读复现。
Moving 阶段可能仍存在旧 PTE，但 Frame 始终强持在源槽并仍归源，
不得用“有 PTE ⇒ Resident”代替以上同步证明。

门禁修复后父代理再跑 accept/product/system-fault 各 1 passed
（10.41s / 5.29s / 13.45s），kernel all-targets semihosting release check
通过（33 组诊断），相关格式检查通过。该原子替换可以本地封存，未推送；
逐页撤销、大页拆分、跨 Nest 同步失败回退及 T4 等待/授权仍未完成，slot 10 仍关闭。

### 后续 T4 等待与结果合同（T3 时确认，实施记录见 T4A–T4C）

- 等待 key 按 View 聚合，包含空间 ASID 与 ViewId。接受同一关系上不同页的
  等待者一起醒后重查，不做逐页整窗扫描。空间清理须匹配该内部 key。
- 元数据准备必须在双 Flat 锁内、任何 Moving/Reserved 写入之前完成；
  Flat 事务仅接纯 core 准备回调，候选快照同时覆盖两端，只收 Weak。
  锁阶 Flat → 来源关联允许，Flat → View 禁止；只能先收完整上界，
  占槽后在无 core 锁时按不可变来源窗口过滤、去重，沿用已备容量。
- 准备失败没有槽变化，无通知责任。占槽后的失败会回退，也必须交还同一份
  受影响关系清单；不能因 unpublish 返回错误而丢掉回退通知。
- 核心执行结果使用单独结果载荷：迁移结果与 `Vec<Weak<View>>` 两条通道，
  不新增集合包装或泛化 ClaimFailure。载荷只表达 core 关系事实，不依赖 Room。
  `must_use` 不强制通知顺序；entry 的单一消费路径及测试负责先释放 caller.pend，
  再整链通知，最后取迁移结果答 syscall。不得在中间 `.result?` 丢弃清单。
- 通知逐项升级引用、取 key，先释放 View/Nest 强引用再进入 Room 提交边界，
  防止最后一个 Nest 的析构把 shootdown 带入 COMMIT。已登记条件等待可用现有
  `signal` 整链放行；谓词重查保证不依赖无人等待时预存的单比特 beacon。
- 条件等待的核心只读口必须区分 Fresh 与 Missing：Fresh/Resident 可回到常规缺页，
  Moving/Reserved/Missing 才继续等待，越界单独失败。不使用
  `physical_at().is_ok()` 作为谓词，否则 Reserved 回退到 Fresh 会永久睡死。
  COMMIT 内只短读身份、权限及可物化性，不调完整 resolve_fault/materialize/shootdown。
- TaskGone 复用既有 `doom::pop_waiter`：按任务身份遍历 site 分片脱链并回收等待票，
  不依赖共享空间销毁或未来页面事件，不新增按 Task 的第二套 wipe/generation。

必须覆盖回退通知、Fresh 回退后晚到等待者、同 View 多等待者一次通知、
共享 Team 中单 Task 退出脱链，以及通知在管理串行锁外的真实顺序。
这些合同不计作已实现的 T4 能力。

### T3 逐页撤销初次检查点

已集成 worker 检查点 `e3bc2e57-d43b-4b5d-995c-f3e8d89f431a.8662087`：
占槽后枚举来源页的全部候选关系，先排除失联/内核属主，再逐 Nest 结清旧失效、
准备单页几何与退休元数据、撤 PTE/published 并确认本次失效，最后才提交归属。
失败由 RAII 回退两端；已经确认撤掉的 PTE 不回填，失败 Nest 继续持退休责任。
不退占用、不改活动形状、身份、VA、权限或架构属性。

新增/更新的原生矩阵包含两 Nest 三关系、切片与活动洞、整页内容与同 Frame/PA、
前一 Nest 已确认后的第二 Nest 同步失败、准备失败、目标 Missing 回退和按需重发。
worker 自验健康面 64 passed + 缺 initrd 哨兵 1 failed，三个景各 1 passed。
父代理在初次集成树实跑 all-targets semihosting dev check 通过，
健康面同为 64 passed + 哨兵 1 failed（退出 101，5.48s），env 24 passed、
workspace release check 通过（kernel 35 组、programs 10 组未使用诊断）。
这些读数不代表以下安全/覆盖问题已批准：

- Frame 身份应在撤 PTE 前、同一 Nest/View 临界区内核对；
  撤页原语只对 memory 内部开放，并自己拒绝永久内核/全局映射。
- 稳定索引次序不能让“连续查两次”冒充两种候选顺序，永久拒绝测试须明确覆盖
  两种注册次序，并核对没有先撤普通映射。
- 现有大页原语用例先做了逐页改权，目标已被拆成小叶，不能据此声称新撤页序列
  从大叶起步已验。要求新增新建 2 MiB 叶后直接拆单 4 KiB/撤页/邻居不变的原语用例。
  普通 acquire 页形成大叶的 Transfer 端到端仍未验；“没有连续/对齐保证”
  不是“物理上绝不可能”。

父代理发现并要求修复的全局历史反例：非内核 ASID 也可由核心 attach_shaped
建立 G 映射；当前 ASID 定向 sfence 不覆盖 G，且该关系退场后弱关联会消失。
因此只检查当前活 G 关系不足。首版保守规则为：来源一旦进入内核 ASID 或 G
绑定的发布提交，整份 Flat 永久失去迁移资格；资格归同一 Flat 状态锁保护，
替代原只描述 imported 的来源资格字段，不另造历史表或可恢复开关。
两条发布路径均须标记，初始普通 acquire 可迁移、导入/借用不可迁移；
活关系预检及撤页原语还须各自拒绝 Kernel/G。
该限制不禁止正常 nonglobal 来源，不把完整 DRAM 直映当成一次具体物理使用。
一般全局映射的 detach/回收失效范围是现有机制的独立后续风险，本次不冒称已修。

新 worker 仍是本阶段唯一写入者，独立 reviewer 同时审查。
初次检查点未获批准；slot 10 仍关闭，真实多 Hart 旧 TLB 与 T4/T6 仍未完成。

### T3C 集中返工检查点（通过 delta 审批）

已集成 `82b959d5-b239-4399-a7d1-521566554d67.8715131`，原生迁移矩阵共 18 个子例。
来源资格现由 FlatState.transferable 唯一维护，替代 Flat.imported：
acquire 初始为真，导入/借用为假；Kernel/G 关系进入不可失败发布提交之前、
短租约仍在时永久置假。资格读取与占槽受同一状态锁保护。
不同页已先占槽、另一页才进入全局发布时，前者没有因此被全局映射，
可以完成；之后新移交按永久标记拒绝。它不是管理授权的撤权标志。

活关系全量预检、memory 内部的撤页原语还各自拒绝 Kernel/G；
Frame 身份改在同一 Nest/View 临界区内、任何 PTE 变更前核对，
原语不再返回事后校验用的 Frame 克隆。新增显式两种候选注册顺序、
拒绝时普通 PTE 不变、G 关系已消失后整份来源仍失格，以及第二 Nest 定向
准备 OOM（前者已撤、后者未动、两槽恢复、目标 Missing 保持）的用例。

大叶原语用例现从全新 2 MiB 叶（level 1）开始，直接准备内部 4 KiB、预留退休表、
在 NoAllocation 段撤页/回收，核对邻居 PA/全部属性及销毁后的表净零。
worker 跳过 prepare_geometry 时该用例真实失败；永久资格两处发布标记关闭时，
G 关系退场后的拒绝用例真实失败。另一次“删除 G 预检”mutation 发生在
尚无永久标记的中间版本，不作为最终树两条冗余防线各自独立承重的证据。
最终树不通过测试 hook 绕过资格来制造覆盖。

父代理对最终集成树实际运行：

| 检查 | 结果 |
|---|---|
| kernel all-targets semihosting offline dev check | 通过，无诊断 |
| `nu scripts/qtest.nu --package kernel` | 64 passed + 缺 initrd 哨兵 1 failed，退出 101（6.04s） |
| 同命令 `--scene accept` / `product` / `system-fault` | 各 1 passed（10.35s / 5.33s / 13.49s） |
| kernel all-targets semihosting offline release check | 通过，34 组未使用诊断，未添加豁免 |
| custody / frame-runs / Pend / trace / console-line 宿主（显式 x86_64、offline） | 3 / 9 / 37 / 2 / 5 passed |
| 本次相关文件 `rustfmt --check --config skip_children=true` | 通过，没有全树重排 |

独立 reviewer 已实跑初版并确认退休、同步、回退主链路，并只读批准集中返工：
永久资格的默认值/同锁读写、全部发布入口、短租约与多页竞态、Frame 前置身份核对、
锁序和析构，以及定向 OOM、两种候选次序和新大叶原语用例均已核对。
测试中把资格拒绝误写成后置预检承重的旁白已改正；没有为覆盖冗余防线而绕过状态。
没有把大叶原语测试算作普通 acquire 大叶迁移端到端，也没有宣称多 Hart 旧 TLB、
T4 管理授权/缺页等待、T6 业务或真实 DMA/IOMMU 产品已完成。slot 10 仍关闭。

### T4A 两端候选与执行结果检查点

已集成 `3a29afab-ce41-4971-ab61-bc9c8ac3f56b.8785530`：
事务在同一双 Flat 临界区中检查两端、准备目标容量与两端候选清单，
然后才取得承载独占、写 Moving/Reserved。准备回调只收来源键与 Weak；
不在 Flat 锁内升级 View、不逆向取 View/Nest 锁。候选是完整上界，
占槽后在无 core 锁时按来源窗口原地过滤、按 Weak 身份去重。

`TransferOutcome` 分开携带迁移结果与受影响关系。占槽前失败清单为空；
占槽后成功或失败交还同一份清单，失败先显式析构事务完成回退。
这一载荷尚未接入 Room 通知或 syscall，不代表等待者已经能够被唤醒。
只读 `can_materialize` 区分 Fresh/Resident/有效 Borrowed 与
Missing/Moving/Reserved，避免目标回退成 Fresh 后晚到等待者永久睡眠。

原生迁移矩阵增至 22 个子例，新增候选准备 OOM 不改槽、失败结果覆盖两端且
排除窗口外关系、同关系覆盖两端去重，以及可物化性（含在途与回退）检查。
worker 的去重删除 mutation 使重复计数断言失败，已恢复；父代理没有重复该 mutation。

父代理在该集成树实际运行：

| 检查 | 结果 |
|---|---|
| kernel all-targets semihosting offline dev / release check | 通过；dev 无诊断，release 38 组未使用诊断 |
| `nu scripts/qtest.nu --package kernel` | 64 passed + 缺 initrd 哨兵 1 failed，退出 101（5.53s） |
| 同命令 `--scene accept` / `product` / `system-fault` | 各 1 passed（10.39s / 5.28s / 13.44s） |
| workspace offline dev / release check | 通过；programs 10 组既有未使用诊断 |
| custody / frame-runs / Pend / trace / console-line 宿主（显式 x86_64、offline） | 3 / 9 / 37 / 2 / 5 passed |
| 七个相关 Rust 文件 `rustfmt --check --config skip_children=true` | 通过，没有全树重排 |

独立 reviewer 已核对准备顺序、锁阶、登记与短驻留的互斥、回退和清单完整性，
并实跑健康面与 dev/release 检查。新增四组 release 未使用诊断为候选收集、
过滤、去重与 TransferOutcome；生产消费入口尚未接线，没有添加告警豁免。
reviewer 在一次性副本中分别破坏准备顺序、目标候选、去重、窗口过滤、失败清单
与可物化性，各次对应断言失败；这些是 reviewer 的实验，父代理未重复。

审查后修正注释，并在既有子例中补跨 Flat 成功、同 Flat 失败的完整清单断言；
两种准备失败增加目标 `can_materialize == true`，直接排除 Reserved 泄漏，
不再只靠 Resident 快照与后续重试间接判断。父代理对该收口树再跑
kernel all-targets semihosting offline dev check、相关文件格式检查通过，
健康面仍为 64 passed + 哨兵 1 failed，退出 101（5.54s）。
独立 reviewer 已只读核对并批准收口 delta；其他景与宿主结果为收口前读数，
没有把仅注释/断言变化后未重跑的检查写成重跑。

本检查点不包含管理授权、条件等待、生命周期通知，也不证明真实 Task 等待链、
共享 Team 单任务退出、多 Hart 旧 TLB 或真实设备迁移。slot 10 仍关闭。

### T4C 入口补充合同（实施前确认，实施记录见后）

- 保持已批准的四字 Transfer 参数与相对 Hold 窗口页号。新增错误码
  `-8 SourceEmpty`、`-9 DestinationOccupied`、`-10 NotTransferable`，
  旧 `-1..-7` 不动；核心 OutOfRange 对应 NoRegion，OOM 对应 OoM，
  Busy 对应 Busy，管理授权失败对应 Denied。
- 两端管理判据都要求 caller live 与来源未 revoked；私有阶段还要求真实 owner。
  根阶段要求调用者表中真实根 token、PoleMeta、root Grant 同一 Arc 且仍活，
  并要求 `heir().is_none()`（既有“交出后不可用”语义，不靠 ONLY 权限位）。
  `root()` 扩展现有结果携带 Grant，不新增管理者包装或第二身份来源。
- caller.pend 覆盖两端取 Hold、解析、管理核验到 execute 结算/回退；
  不取另一个 Task 的 pend，不把 work COMMIT 带入迁移或 shootdown。
  PoleOperation 不是授权；若使用其 try 串行化，同根只取一次，取不到返回 Busy。
- `doom` 的安全程序序是同 pend 下先清空 references，再 close_controls/
  seal_owned；不是“owner taskId 相同所以不能并发”。入口 Seal 须在同 pend 下
  核验并使 mail 失活，随后释放 pend 才 close_maps，避免通知或 shootdown 带锁。
  来源 revoked 新口只短读，不常持闸门跨 execute，不扩永久撤销 ABI。
- execute 返回后先释放 caller.pend，再逐个消费 affected。从 Weak 升级取得
  raw Nest ASID 与 ViewId，先释放 View/Nest 强引用，再 signal 整链；
  最后才使用迁移 result，不用提前 `?` 丢弃失败回退通知。

### T4B1 条件等待集成检查点（本地基础已审）

已集成原 worker 的集中返工版本
`845b2fcc-8f0f-42fd-a9f1-812beae125e8.8983332`，保留 T4A 已封存增量。
核心探针在 Nest → View → Flat 锁序下只读判定四态；待结算失效先返回
Materialize，由 COMMIT 外的完整解析结清。完整解析以
`Result<FaultOutcome, NestError>` 分开返回资源失败与缺页/身份事实，
Busy/Missing/关系已消失重探，不因移交竞态直接杀任务。

Room 新增 View 聚合 key 与条件等待；COMMIT 仅覆盖谓词与登记，之后先释放
COMMIT、再释放所持 Nest 上下文，最后才 finish_block（可能调度或进入 idle）。
谓词还核对本次 View 身份：同 VA 已被替换为另一个 Missing 关系时不能挂到旧键。
空间清理匹配 View key；TaskGone 仍复用既有按 Task 身份摘链的 doom::pop_waiter。

Work Space 的 withdraw/dispose/access/narrow 保存 Nest 操作结果后，
不论 Ok/Err 都 signal 整条 View 等待链；调用层统一经过这些入口，
不把 Room 反向引入核心。信号只使用已取定的 ASID/ViewId，
不会在 COMMIT 内升级 Weak 或析构最后一份 Nest 强引用。
trap 的等待、重探都记 `PageFault.resolved=false`，真终止另记 FaultKilled；
登记失败保留真实 tid。所有进入 kill/quit/run 的缺页分支先释放额外 Nest 强引用。

初版虽通过健康面，父代理读码发现旧 View 键漏醒、待结算失效绕过、资源失败分类
及等待 trace 语义问题，要求返工后再发现 kill 前强引用与登记失败 tid 问题；
以上均已集中修改，不能把初版测试通过视为安全批准。worker 的三个 mutation
（忽略身份、忽略待结算失效、把同步失败折 Invalid）各触发对应断言失败，已恢复。
父代理没有重复 mutation，独立 reviewer 已核对六处集中修正。

独立 reviewer 提出的 Borrowed 单核空转疑问已经双方逐项核对排除：
普通 withdraw 在同一 Nest/View 临界区内撤 PTE 并更新 active/index，
两者都先于可失败 shootdown；close 同样在锁内撤页并清 active/index。
未发布安装失败不暴露关系，已发布失败交既有清理链。
只有逐页 unpublish 故意保留 active，但借用来源在事务资格检查处就被拒绝；
它服务的普通来源在撤页时为 Moving，提交后为 Missing，均应等待。
因此 Borrowed 不存在“活动权限仍在、PTE 已撤、publish 空操作”的合法单核状态。
不新增重试计数或 Borrowed 缺 PTE 兜底来掩盖该不变量；并发下残余 Materialize
重试原指令，不承诺持续竞争下的公平性或固定次数上界。

资源失败责任仍按整个 Nest 保守处理：即使待结算失效来自别页，
完整解析无法结清时也记录真实失败后终止任务，不把它解释成本页缺少承载。

父代理对六处返工后的树（最终 R1 收口前）实际运行：

| 检查 | 结果 |
|---|---|
| kernel all-targets semihosting offline dev / release check | 通过；dev 无诊断，release 38 组未使用诊断 |
| `nu scripts/qtest.nu --package kernel` | 65 passed + 缺 initrd 哨兵 1 failed，退出 101（5.73s） |
| 同命令 `--scene accept` / `product` / `system-fault` | 各 1 passed（10.35s / 5.28s / 13.50s） |
| workspace offline dev / release check | 通过；programs 10 组既有未使用诊断 |
| custody / frame-runs / Pend / trace / console-line 宿主（显式 x86_64、offline） | 3 / 9 / 37 / 2 / 5 passed |
| 14 个相关 Rust 文件 `rustfmt --check --config skip_children=true` | 通过；未检查两个有既有排序漂移的 messenger 模块，未全树重排 |

最终 R1 收口将来源可物化性检查移至硬件条目之前，Moving 即使旧 PTE 仍在也
保守返回 Wait；该旧 PTE 原本由 Moving 槽强持 Frame，并非无归属。
新增已发布在途用例，先确认 Resolved，探针窗口再确认旧 PTE 尚在而纯判据为 Wait；
Borrowed 部分撤页失败用例核对待结算两页 Materialize、再次失败返回 Shootdown、
结清后被撤页 Invalid、邻页 Resolved，并通过 audit。健康例共九个子例，
另有 View key 清理预热。worker 逆转检查顺序后真实命中在途已发布断言。

父代理对最终收口树再跑 kernel all-targets semihosting offline dev / release
检查（dev 无诊断，release 38 组）、三处改动文件格式检查与 diff check 通过；
健康面 65 passed + 哨兵 1 failed，退出 101（5.50s），accept / product /
system-fault 各 1 passed（10.39s / 5.28s / 13.44s）。
workspace、宿主与其余文件格式结果仍为上表收口前读数。
独立 reviewer 在父代理实际树定向复核了检查顺序、已发布在途、Borrowed 部分撤页、
无掩盖性计数/兜底，以及借用来源无法进入迁移撤页五点，批准作为 B1 本地基础。
该批准不包括真实 Task 等待、生产迁移入口或多 Hart 旧 TLB。

新增健康例仅证明纯探针、替换身份谓词、失败同步分类、View key 空间清理与
Space 入口等价性；不证明真实 Task 等待链。block → signal → resume、
同 View 两等待者、Fresh 回退晚到者、共享 Team 单任务退出、管理锁外通知、
probe 与 materialize 间的精确 Busy/Missing 交错及真实多 Hart 旧 TLB 均尚未端到端验证。
不新建大量假 Task/Team/Space/调度器的宿主替身来替代该证据；真实场景随 T4C
受控入口接通后验证，未满足前不发布。B1 封存时 TransferOutcome 尚无生产消费者，
slot 10 仍关闭；后续接通状态见 T4C 实施记录。

### 真实等待场景的观测准备（场景尚未实现）

父代理实跑 `nu scripts/qtest.nu --package kernel --scene product -- --features semihosting`：
1 passed（5.33s），现有 runner 的 rest 参数已可透传 feature，无需另建运行入口。
本轮根目录产生的 `sqware-diagnose.jsonl` 可完整逐行解析，共 45,884 行，
含 898 个 room.wait 与 891 个 room.wake；产物已归入忽略的
`trace/scene-product-feature-diagnose.jsonl`。这些是 product 的诊断通道证据，
不是 Transfer 或 View 等待的端到端证据。

后续场景应先移出承载再启动同 Team、共享同一 View 的故障任务，
以已知 tid、故障 VA 和测试阶段关联事件；不能把折叠后的 key 当成唯一 View 身份。
“即将访存”标志和固定延时都不证明入队；完成也不证明曾睡眠，
因为回填可能早于真正访存。复用结构化导出与现有 feed-script 接口，
由测试侧看到两名任务真实等待后才反馈放行回填；缺事件应超时失败，
不降低断言、不新增生产观察 ABI 或 trace schema。该握手尚未实现。

### T4C 管理授权与生产消费检查点（代码已审）

已集成 `9743b0cc-74da-43bd-897e-1863d4dad4ff.9113018`。受控本地树现已开放
slot 10 的真实四字 Transfer，11 Traverse 与退役 0–4 不变，错误码按前述
`-8/-9/-10` 落位。execution 包装直接调用该 ABI，不预加来源偏移。

入口在同一 caller.pend 下取得、解析两端 Hold，核验当前管理授权，
执行到结算/回退；释放 pend 后才逐项消费 affected，先放 View/Nest 强引用再
signal，最后答执行结果。两端均核授权，页号仅在 Transfer::new 换算一次。
私有阶段核真实 owner；根阶段核表内真实根 token、无 sire/heir、
Pole 弱身份与 Grant 同 Arc，以及 pie.alive（Meta 与 Grant 都活）。
不看 FETCH/STORE/VEST/ONLY 位、不取额外 PoleOperation 租约。
来源只在既有 Phase 中维护身份；既有 Phase 短锁克隆代替初版重复的局部 Manager。
root() 在现有结果中携带 Grant，未新增身份 API 或管理者包装。

Seal 的核验与 mail.seal 同 pend，随后在 pend 外 close_maps。
doom 先在 pend 内清 references，才到 pend 外收控制/封来源；
不把“taskId 相同”当成并发排除。早错管理判据不升级相位里的弱 Pole：
临时强引用来自 pend 所保护的真实表项，不会在该窗口成为最后一份。
另核实核心 Nest/Pressing 析构只做 core 回收及 shootdown，不进入 Room、
work COMMIT 或别的 Task pend；本合同本就允许 pend 覆盖核心逐 Nest shootdown。
因此不为“最后一份 Nest 可能在 pend 内析构”另加延迟容器，
真正的 Room signal 与其 COMMIT 仍严格在 pend 外。

新增三个真实 dispatch 健康例覆盖移交、拒绝与通知消费顺序，
含私有/根、非零窗口偏移、同 Frame/PA/整页内容、Fresh/Missing 目标、
两端管理者分别拒绝、heir、撤销/doom、retained Busy 与 Borrowed 永久拒绝。
公开 FETCH-only 根仍可管理；非 owner 私有记录用例明确是夹具直造，
不宣称公开 API 可达。sealed retained 用例隔离 Meta 存活判据：
保留引用绕过普通 resolve 的 Meta 检查，Grant 仍活时依然必须 Denied。
worker 将判据改成只看 Grant 后真实落到 Busy，已恢复。
其余 mutation 分别命中授权、执行持 pend、通知释放 pend、
偏移只换一次、失败回退清单与不看 VEST 位；父代理未重复这些 mutation。

父代理实际验证：

| 检查 | 结果 |
|---|---|
| 最终集成 kernel all-targets semihosting offline dev / release | 通过；dev 无诊断，release 6 组既有未使用诊断 |
| 最终集成 `nu scripts/qtest.nu --package kernel` | 68 passed + 缺 initrd 哨兵 1 failed，退出 101（6.15s） |
| 最终集成同命令 `--scene accept` / `product` / `system-fault` | 各 1 passed（10.39s / 5.33s / 13.50s） |
| 最终集成 workspace offline dev / release | 通过；programs 10 组既有诊断，release 另有 kernel 6 组 |
| 最终集成 Pend 宿主（显式 x86_64、offline） | 37 passed |
| 集中返工前 env 宿主（显式 x86_64、offline） | 26 passed；signals 的既有 unused Wire 警告仍在 |
| 集中返工前 execution 专用宿主台架（`crates/execution/tests/Cargo.toml`，显式 x86_64、offline） | 6 passed |

父代理另试 `cargo test -p execution --target x86_64-unknown-linux-gnu --offline`，
因 production TLS 的 RISC-V `mv ..., tp` 无法在 x86 汇编而失败；
它不是上述专用宿主台架，不修改目标架构代码来制造通过。
其余 custody/frame-runs/trace/console-line 的本阶段最终重跑证据来自 worker，
父代理未重复；15 个相关 Rust 文件格式检查与 diff check 已通过。
独立 reviewer 已审核心合同，并在父树复核 Phase 克隆及 sealed retained 新例，
在一次性副本中独立删除 pie.alive 后真实命中 Busy 而期望 Denied。
最后注释补丁明确了内核铸造保留 cap 与引导设备借用来源的夹具性质，
以及 sealed retained 只直调失活步骤，不覆盖入口 Seal 的 pend/close_maps 编排。
父代理在该注释收口后重跑 dev check、三处相关格式检查和上表健康面；
其余景、release、宿主与 workspace 读数来自相同代码逻辑的注释收口前树。
T4A/B1 的 38 组是当时未接线树的历史读数，不改成当前 6 组。
父代理另跑 `cargo rustc -p kernel --lib --release --offline -- --print cfg`，
实测不含 debug_assertions：release 场景不包含 debug-only 探针或 lockdep，
不能用健康面自检代称 release 的同一组护栏。根 Cargo.toml 的相反注释已更正，
没有修改 profile、打开或关闭任何检查。

迁移路径的 COMMIT 约束不扩张成全仓禁止 pend → COMMIT：
现有记录提交和 Seal 的 Mail 唤醒允许这个方向；迁移核心不持 work COMMIT
等待 shootdown，Transfer 的 View signal 仍在 pend 外。execution transfer
包装本阶段只有目标构建与线格式证据，尚未经过真实用户态 ecall。

slot 10 在本地受控树可执行，不等于允许发布或完成 T4/T5：
真实 Task block → signal → resume、同 View 多等待者、Fresh 回退晚到、
共享 Team 单任务退出、多 Hart 旧 TLB、pend 跨 shootdown 的多 Hart 活性，
以及入口 Seal 的并发编排仍未端到端验证。下一阶段用真实场景取证，
不把当前无等待者时的消费顺序探针算作 Room 等待链证据。

### T4D1 真实 Task 等待场景（已取证并通过定向审查）

新增 `memory-transfer` 景：装载者通过真实 Loader 创建带 sire/收据的控制单元，
故障任务由控制单元在同一 Team（Spawn team=0）内创建，共用同一 View。
装载者核对控制单元 Reap + EXIT_OK，控制单元逐名核对预期退出原因，
不放宽 ledger/verdict 来忽略意外故障。串口经 DTB 设备清单、引导区授权、
生产 accord 与 Dock 映射取得，未新增裸 PA 接口、生产 ABI 或 trace schema。

宿主读本轮结构化事件后才经 UART 开门，不以固定延时或“即将访存”作入队证据。
Wait trace 位于 COMMIT 内、入链之前；填回 signal 必须取得同一 COMMIT，
因此通知不会越过该次完整登记。身份来自同 Team/View 的实际构造、
已知 tid/VA/阶段，折叠 key 只作辅助，不作为全局唯一身份。

| 阶段 | 最终事件判据 |
|---|---|
| A 一次填回两名 | 两名各 Wait 1、Wake 1、正常 Exit；填回仅一次 Transfer |
| B 单任务 Slay | 被杀者 Wait 1、Wake 0、无自愿 Exit，控制单元核 Slay 回执；存活者 Wait 1、Wake 1、正常 Exit |
| C 部分撤页 | 被撤页任务 Wait 1、Wake 1、FaultKilled.stval 为公告 VA、Fault Exit；邻页任务 Wait 2、Wake 2、正常 Exit |

六名故障任务各 Reap **恰一次**，全部来源/View 显式关闭、引用显式释放。
预期被 Slay/Fault 的两名改用 raw Spawn 标量参数，不创建不能展开回收的 Rust
闭包/结果槽；正常返回的任务仍用既有 Join 包装并逐字节校验整页内容。

集中集成时发现 qtest 清理块冲突、初始化跳过本轮早到事件、未等 EOF 即归档，
父代理接管单写修复。最终驱动从 offset=0 读取本轮完整行；完整坏 JSON、
消费后删除/替换/截断、EOF 末行不完整均失败。先等远端 EOF 再最终读取，
重核各阶段精确 Wait/Wake/Exit 与六名 Reap 后才写 summary、归档。
固定名诊断与追写捕获仅在本景、QEMU 启动前清理。
Python 通道各一文件，FEED 采用标准包加载；新增 13 个宿主回归保护读证据规则，
只 mock 宿主 EOF，不模拟 KernelTask。独立 reviewer 实跑这些回归，
分别破坏坏 JSON、EOF、最终 Wake 校验后对应断言真实失败。

父代理最终实际验证：

```sh
nu scripts/qtest.nu --package kernel --scene memory-transfer \
  --feed-script programs/tests/memory-transfer/driver.py -- --features semihosting
CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true nu scripts/qtest.nu --package kernel \
  --scene memory-transfer --feed-script programs/tests/memory-transfer/driver.py \
  -- --features semihosting -- scene
python3 -B -m unittest discover -s programs/tests/memory-transfer -p 'test_*.py' -v
```

- 常规连续 3 次各 1 passed（0.37s / 0.42s / 0.37s）；带护栏 1 passed、
  68 filtered（0.87s）。最终 summary 逐名符合上表，六名 Reap 各 1；
  父代理两份 summary 归于忽略的 `trace/scene-memory-transfer-parent-*-phases.json`。
- 13 个宿主回归通过（0.043s）。相同 FEED 加载路径真实回归 terminal session：
  accept 1 passed（10.39s）、product 1 passed（3.78s）。
- workspace offline dev/release 与 kernel all-targets semihosting offline
  dev/release 通过；programs 10 组、kernel release 6 组既有诊断，dev kernel 无诊断。
  健康面仍为 68 passed + 缺 initrd 哨兵 1 failed，退出 101（5.65s）。
- system-fault 1 passed（13.44s）；loader-api 宿主（显式 x86_64、offline）
  6 passed；14 个相关 Rust 文件格式检查通过，未全树重排。
  Pend 宿主本阶段读数来自 worker，父代理未重跑未改的核心宿主套件。

DA 命令的第二枚 `--` 后 `scene` 是 libtest 过滤器，不能省略或当普通位置参数。
worker 实跑不加过滤器时 12 passed / 57 failed（前次 10 / 59），均为单连接
场景 helper 退出后的 Connection refused；不是全绿，也不是本景端到端证据。
普通 release 只有 scene 一例。独立 reviewer 另跑常规 5/5、DA 1 次通过，
不带 semihosting 的负对照真实失败。最终审查批准协议、权限、收据与主树 observer 收口，
不把子树早期通过读数替代主树验证。

磁盘清理恢复后，父代理补跑无 semihosting 的精确负对照：
driver 报缺等待事件，固件退出 101，qtest 脚本最终退出 1（30.27s）；
当场断言本轮没有 summary 或归档 trace；这是无导出的负对照，不代表失败轮不能归档。
为防旧成功产物残留，仅在本景 QEMU 启动前清理固定名 summary/归档，其他景不变。
随后恢复正常精确命令 1 passed（0.42s）、带护栏过滤命令 1 passed /
68 filtered（0.87s）、13 个宿主回归再次通过（0.042s），两处注释格式与
diff check 通过；其余基线与场景读数仍为上述主树 observer 收口后的读数。
worker 的不发门负对照仍只引用其报告，父代理没有重复。

本景现在证明真实 ecall、Task 等待/整链唤醒、共享空间单任务退出与部分撤页通知。
仍不证明 Fresh 失败回退/晚到者、精确 probe↔materialize Busy 交错、
热多 Hart 旧 TLB、pend 跨 shootdown 活性、入口 Seal 并发编排或设备/DMA 迁移，
不作为 T4/T5 全部完成或发布许可。

### T5A 热远端 TLB 场景（已取证并通过集中复审）

新增 `memory-transfer-tlb` 景。控制者与 raw 读者在同 Team/非内核 ASID，
普通来源仅有一个源 View；预先物化并填入花纹后才放行读者。
读者在测试侧汇编窗口内清 SIE、安装私有向量、真实 load 热身并发布 warmed；
窗口内没有 ecall、分配、调度、satp 写入或本地 sfence。
控制者看到 warmed 后只执行一次真实 Transfer，成功返回后才打开采样门。
读者不让出 hart，控制者却能完成这段动作，构成远端执行的因果证据。
`spawn.h`/`exit.h` 只存入 summary 的 boundary，不作采样 hart 判据：
恢复 SIE 后任务可能迁移，同核的两个边界事件不能据此否决这次测试。

采样后的私有向量记录原始 cause/stval/sepc，不重执行 fault load，
而是 sret 到统一出口；出口恢复原 stvec/sstatus，之后真实 ecall 收工。
宿主最终核对 reader 的正常 Exit 与 Reap 恰一次、同 VA 没有进入生产内核的
Load 缺页，并等待 EOF 后才写 summary/归档。读者收工只印固定标记，不分配。
窗口期限由 DTB 的 timebase-frequency 预先换算，窗口内只读 rdtime，
不用会进入生产 trap、顺带刷新 TLB 的计时接口。

通过判据是 Trapped、cause=13、stval=公告 VA、sepc=sample_at、
warm=预置花纹，且 VA/warm_at/sample_at/sepc 均非零、两条 load 地址互异、
入口 SIE 原本开启。首次复审发现 status 在数据栅栏前发布：
返工改为三条出口只向 caller-saved t4 写结论，统一出口唯一一处
`fence rw,rw → sd status`，控制者以 Acquire 读取后再读载荷。
宿主回归明确拒绝 cause/stval 正确但 sample=sepc=0 的伪通过；
程序侧同规则是防御支，未有端到端零地址复现器。

生产修改仅在 lease 发布后、shootdown 本地 sfence/lease 扫描前各加完整数据
fence；没有改 lease 选择、SBI ABI、PTE 退休或迁移提交规则。
显式数据排序不能只用本地 sfence 代替，lease 的 Release/Acquire 也不能把
“扫描未观察到租约”变成同步关系。两侧均保留 fence，不凭 QEMU 的表现删减。
occupy 与 satp 的相对次序仍由调用点负责，不假设内核入口先 occupy 后 satp。

父代理在返工主树实际执行：

```sh
nu scripts/qtest.nu --package kernel --scene memory-transfer-tlb \
  --feed-script programs/tests/memory-transfer/tlb.py -- --features semihosting
CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true nu scripts/qtest.nu --package kernel \
  --scene memory-transfer-tlb --feed-script programs/tests/memory-transfer/tlb.py \
  -- --features semihosting -- scene
python3 -B -m unittest discover -s programs/tests/memory-transfer -p 'test_*.py' -v
```

- 常规 1 passed（0.17s）；带护栏 1 passed / 68 filtered（10.49s）。
  后者实测 warm=0x7、cause=13、status=1、stval=VA=0x67000、
  sample=sepc=0x14916、warm_at=0x148ca；无生产 Load 缺页。
- 25 个宿主回归通过（13 个既有、12 个本景）；使用 socketpair 检查真实字节流
  EOF，不模拟 KernelTask 或 QEMU。workspace offline dev/release、
  kernel semihosting all-targets dev/release 检查通过，programs 10 组、
  kernel release 6 组既有告警；diff check 通过，未全树重排。
- 同一 T5A 集成树的 D1 1 passed（0.42s）、accept 1 passed（10.39s）、
  product 1 passed（5.33s）、system-fault 1 passed（13.49s）；
  默认健康面 68 passed + 缺 initrd 哨兵 1 failed，退出 101（5.90s）。
  这组回归在 status 返工前执行，返工只动本景及 fence 注释。
- 独立 reviewer 在返工后的父树只读副本重跑常规 1 passed（0.17s）、
  带护栏 1 passed / 68 filtered（10.49s）、宿主 25 passed（0.044s）；
  D1/accept/product/system-fault 各 1 passed（0.42s/10.34s/5.33s/13.45s），
  健康面仍为 68 passed + 哨兵 1 failed，退出 101（5.84s）。
  私有向量的寄存器保存、t4 caller-saved 结论、CSR 恢复次序均通过定向审查。
- 独立 reviewer 在初审副本中分别删除全部远端 RFENCE、仅删除 lease 定向选择
  而保留内核全局刷新，两次都得到 Stale byte=0x7，排除了旁路全局刷新掩盖。
  worker 在返工树保留本地 flush、跳过远端 RFENCE 后也真实失败，恢复生产文件，
  并核对本轮无成功 summary；这些负对照不是父代理重跑的读数。
  驱动失败时仍会归档已存在的诊断文件，它是失败证据，不代表本轮通过。
- reviewer 在返工副本再次验证 lease-only 变体为 Stale，失败轮有诊断归档、
  无 summary；无 semihosting 的另一轮则两者皆无。`QEMU_SMP=1` 实跑退出 1、
  status=Timeout、采样门始终未打开，补强了远端执行判据。
  删除统一出口的发布 fence 后仍 1 passed（0.17s），明确本景不能检测这个排序缺陷。
  变体只在独立副本执行，生产树保留两侧 fence 与唯一 status 发布 fence。

范围只覆盖当前 QEMU 与仓库 SBI.bin 的热远端撤页及 caller.pend 横跨同步时的进展。
实际汇编中 fence 位于发布前是排序证据，QEMU 通过不等于真实硬件弱内存已验证；
不从 SBI.elf 反推 SBI.bin 的指令（两者转出的 binary 不同），也不宣称所有 SEE
均已验证远端同步完成契约。Fresh 失败回退/晚到者、Seal 并发、设备/DMA、
真实 Platform 产品接线及 T4/T5 全部验收仍未完成。本景无等待者，不覆盖等待链
与 shootdown 的交互。独立审查批准本地封存，不构成共享分支合并或推送许可。

### T4D2 Fresh 失败回退与晚到者（执行记录）

在 `81018931` 后新增独立 `memory-transfer-fresh` 景及非默认 `fresh-scene`
测试开关。目标是两份不同普通 Flat 的真实 Transfer 在占槽后撤页确认失败：
A 中 Reserved 目标上的真实等待者经生产 consumer 通知后物化新清零页；
B 中失败时无等待者，失败返回后才创建晚到 Task，直接物化 Fresh，不进入等待。
握手页为第三份普通来源，线格式由程序与内核测试夹具 include 同一文件，
不新增生产 ABI、不放宽 Traverse 授权，也不跨 Transfer 持 Pages/Residency。
源花纹保留是内容证据，不能据此声称端到端取得同 Frame 身份或零分配证据。

worker 初次检查点报告常规 DA 场景连续六次通过、三个负对照失败；
父代理合入该检查点后执行相同精确命令：

```sh
CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true nu scripts/qtest.nu --package kernel \
  --scene memory-transfer-fresh --feed-script programs/tests/memory-transfer/fresh.py \
  -- --features semihosting,fresh-scene -- scene
```

父树实测 **0 passed / 1 failed / 68 filtered**（10.23s），脚本退出 1。
A 的 Wait/Wake 各 1、目标未结算及已结算 Load、源撤页后已结算 Load 均出现，
内容读数为目标 0、源 0x7f800。B 报 Busy/armed 后控制者超时：
`the reader never reported`。完整导出有第二组 Spawn/Embark 环境调用，
但只记录 tid 1–11 的 Spawn、没有晚到 Task 的 Spawn，也没有 B 目标页缺页。
这一现象尚未定位，不能归因于特定 Team 生命周期或 ABI 缺陷；
也不允许预先创建晚到 Task 来绕过“失败后才创建”的合同。
现场存于忽略的 `trace/fresh-parent-initial-failure-{diagnose.jsonl,qtest.log,feed.log}`，
无成功 summary，保留失败诊断归档。源码回退及生产通知均已核对无负对照残留。

父代理另跑宿主回归 38 passed（0.048s），不把它当成端到端通过。
初读另发现握手 volatile 不等于同步原子、源重新缺页判据会被初次 Store 满足、
目标缺页未核精确数量/类型、feature 实体门与注释不一致等问题，已集中交原 writer
返工。worker 曾报告 Join 的 `invalid TaskExit id` 偶发 panic，未定位；
改用 raw Task 不能视为修好该问题。本阶段独立 reviewer 正在核实实现与失败现场，
尚未批准封存、提交或推送。

独立复审初版结论为 **BLOCKER**。reviewer 最终补正为首轮六次通过，
后续 18 轮 14 passed、4 轮超时，合计 24 轮 20 passed / 4 failed；
另一次启动后被中止的运行不计。不能用连续绿色替代稳定性验收。四轮均有第二组
Spawn/Embark 调用却没有相应 Spawn 事件，也没有 B 目标页缺页；
其中 Chrono::Clock 调用持续至外部 60s 超时，控制者自己的 10s 时限未生效。
失败轮与引导 early IPI 探针实际发送 16/16 的记录相关，绿色轮该探针跳过；
这只是观察到的相关性，不是根因证明，更不能通过关闭 IPI 检查让场景变绿。
寄存器/回答路径异常及原 Join 异常仍待原 writer 定位。

reviewer 另以观察器输入变体证实多处误通过：删除源撤页后的 Load、
把缺页改为 Store 或增加目标已结算缺页、晚到者出现额外 Wake 等仍能通过。
已要求源 Store→Load、目标 A 未结算 Load→已结算 Load、目标 B 恰一条
已结算 Load 的精确判据，以及独立 tid 和晚到者零 Wait/Wake/Park。
“失败后才创建晚到者”的完整边界目前来自程序序，不宣称宿主 trace
包含了尚不存在的 Transfer 返回事件。

复审还发现既有结构化导出在 HOST 锁超时、序列化失败和写失败时可能静默丢事件。
因此 EOF、JSON 行完整性和已观察到的事件只能证明收到的部分，不能单独证明
“从未发生某个事件”；这也限制既有 D1/T5A 中仅靠导出缺席作出的结论。
此项是继续验收前的证据完整性前置，而非已解决的旁支问题。父代理已读码确认，
交同一 writer 评估最小 fail-closed 修复：复用既有损坏标记记录所有导出失败，
在所有 producer 停止后的成功测试出口拒绝损坏证据，不无限阻塞核心锁路径；
最终命令失败时清除成功 summary、保留失败诊断。该修复尚未实现或验证，
当前不能把失败/缺失事件的 trace 当完整账本。

原 writer 随后因 `OpenCode Go error: {"model":"deepseek-flash"}` 中断，
父代理按其精确 frontier `506468cb-3a27-4a2e-938a-1f8c474585e1.9768789`
保留了六文件未完成返工，不把模型服务失败当编译或测试失败，也未换用其他模型。
该返工尚未完成 ABI 定位、导出完整性修复与全部判据回归；
内核 copy_out 后才放 Release fence 的写法仍待纠正，不能称已实现正确发布。
父代理检查当前差异格式无空白错误，独立跑 `test_fresh.py` 为 11 passed /
2 failed（观察器错误文案已变、测试的预期正则未更新），不是最新宿主全绿。
早先 38 passed 属于初版读数，不适用于这份中断检查点。尚无新的端到端通过证据。

父代理随后用同一 Flash high 模型的新上下文 worker 接手 ABI 定位，
只交已确认合同、真实检查点与失败现场，不迁入整段历史；旧 writer 不再续写。
父代理独立处理证据与同步边界，文件所有权分开。当前新增修复尚在定向复审：

- 握手两侧统一用 AtomicU64 Release/Acquire。测试夹具通过既有 Nest::translate
  取得第三 Flat 的普通 RAM 握手页，在回调期间访问原子字；该页不参与 Transfer，
  控制者在调用返回前不关闭其 View。不新增生产 PA 出口，不取 Moving 源帧，
  不跨迁移持有源 Pages/Residency，也不假定一次请求始终在同一 hart 上。
- 导出沿用唯一 BROKEN 标记，序列化、host 锁超时、创建和写入失败均不可逆标记。
  仍保持有界导出，不让持核心锁的生产者无限等待；停机屏障后、测试成功出口前
  检查该标记，证据损坏则退出 101。qtest 最终失败时删除本景成功 summary，
  保留诊断和失败日志；helper 晚完成时的成功产物发布竞态仍在复审核对。
- 宿主要求精确的源 Store→已结算 Load、A 目标未结算 Load→已结算 Load、
  B 目标恰一次已结算 Load、晚到者零 Wait/Wake/Park、独立 tid 和恰一次回收。
  新增 12 项误通过反例，修正中断时两项预期文案，宿主总计 **50 passed**（0.050s）。
  late Spawn 晚于 A 源重读是可观察顺序，完整“B 失败返回后才创建”仍由程序序保证。

父代理本轮实际验证：kernel all-targets offline dev 与带 DA 的 release，
均带 `semihosting,fresh-scene`，检查通过；`nu-check scripts/qtest.nu` 为 true、
diff check 通过。D1 精确景 1 passed（0.47s）、TLB 1 passed（0.22s）；
Fresh 精确 DA 1 passed / 68 filtered（10.64s）。这一轮绿色不取消既有偶发失败阻断，
新 worker 尚未交付 ABI 根因或修复。

父代理另作受控负对照：仅临时丢弃最后一条 Halt 导出事件并置 BROKEN。
TLB 的真实采样/退出事实仍正确、宿主 `.result` 为 `ok`，但 firmware 与 qtest
均退出 **101**（0 passed / 1 failed，0.27s），报告
`structured export incomplete; scene evidence rejected`；最终无成功 summary，
保留 diagnose 归档。移除注入后与注入前源文件哈希一致，TLB 再次 1 passed（0.22s）。
现场在忽略的 `trace/export-drop-halt-negative-{qtest.log,diagnose.jsonl}`。
这证明测试出口拒绝已知丢失，不证明导出从不失败；写错误和锁超时分支仍待独立复核。
随后仅对 export.rs 作局部格式调整，未全树重排。未提交、未推送。

定向复审批准了原子握手、导出 sticky 失败和 50 项宿主判据，
但确认“qtest 失败后删除 summary”仍有 helper 晚写回的竞态。
父代理改为单一发布权：qtest 在造镜像前删除旧正式 marker，
给本轮生成带 UUID 的私有 pending 路径，经 `SCENE_SUMMARY_PENDING` 传给三个
memory 驱动；驱动只写 pending，qtest 在固件退出、driver `.result` 与
景收尾检查均成功后才原子 rename 为正式名。晚完成 driver 最多留下私有 pending，
不能重建正式成功文件。非这三景不变；同一 TRACE_OUT 并发跑景仍不受支持。
runner 预检失败发生在本轮初始化前，保留的是上一轮证据，不宣称本轮跑过。

新协议定向复审通过，独立宿主回归 50 passed（0.052s）。
父代理实跑 D1 1 passed（0.42s）、TLB 1 passed（0.22s），正式产物存在且可解析；
两个 unit run 用例明确断言只写给定 pending、不写正式名。
新协议下重复丢 Halt 负对照：TLB 事实正确、driver `.result=ok`、pending
含 status=1/cause=13，但 firmware/qtest 均退出 101（0.22s），正式 summary 不存在，
保留诊断。恢复前后 export.rs 哈希均为
`be350ba5848cfa2405d160c2443f41a75ac58adb3fd3549cb0108ffb4fa05efd`，
恢复后 TLB 再次 1 passed（0.22s）。新现场在
`trace/export-pending-negative-{qtest.log,diagnose.jsonl}`，pending 保留作失败证据。

随后 Fresh 精确 DA 命令再次失败：0 passed / 1 failed / 68 filtered，
外部超时 60.05s，无正式 summary。B 的程序诊断打印
`spawn ok tid=390752`、`embark ok tid=390752`（0x5f660），
导出仅观察到 tid 1–11 的 Spawn。这是异常返回的正向读数，不是 tid=0；
“必然读回自己的输入零值”不能作为已确认根因。console/feed/qtest/diagnose
保留为 `trace/fresh-parent-staged-failure-*` 并交 ABI worker；超时轮的 console
仅到 boot IPI 行，不能称取得完整运行期内核控制台，后续正向读数来自 UART feed。
Fresh 偶发异常仍为阻断，以上证据与同步修复通过不代表本阶段通过或允许提交推送。

reviewer 只读复核了新协议负对照的日志、缺 Halt 的 diagnose、遗留 UUID pending
及恢复后正式 summary 的不同 deadline，确认未误发布，恢复哈希也一致。
该注入显式跳过 Halt 并置 BROKEN，实测证明的是消费者拒绝损坏证据的链路；
序列化、锁超时、创建和写错误四条生产端置标记路径已作静态复核，不冒称逐条注入覆盖。
本轮定向审查结束，ABI 修复到位后再独立复审。

ABI worker 随后交出根因检查点：失败的撤页确认留下 Nest::Failed，
下一次 Spawn 的参数 copy_in 被 ready_state 拒绝，真实返回 Denied、没有发布 Task。
同时入口 instr_len 无法读取客体陷入指令，默认按 4 字节推进 sepc；
客体实际指令为两字节 `c.ebreak`，多推进两字节落入下一条四字节存储指令的中段，
不再执行原本向返回缓冲写 a0 的存储，后续调用可能读到旧缓冲。worker 临时内核边界诊断记录
Spawn 的内核答案为 -1、sepc=0x1a636（正确应为 0x1a634），同轮 UART
却报 tid=390752；其反汇编将该值 0x5f660 对到栈上返回缓冲地址。
此项是 worker 的 instrument/反汇编读数，父代理未独立复现该 instrument，
已交原 reviewer 核实链路。early IPI 仍仅作为时序相关，不认定其为根因。

父代理精确合入 worker 的一行 `retry_invalidation` 前置检查点，但仍拒绝封存：
该版以 `let _ = ...` 忽略重试失败，连续同步失败时仍可落入错误长度回退，
不是修复了整个缺陷类。worker 报修复后多轮通过，但移除该行后的三轮也绿，
因此随机循环不能代替确定性回归。已要求同组返工：重试失败不执行环境操作、
不猜长度推进 PC、不暴露旧缓冲作成功；补“重试成功后二字节正确返回”与
“重试再次失败无副作用/无错误 PC 推进”的确定性用例。
不得让 ready_state 自行重试、也不得在 Transfer 失败返回前暗中结清责任；
这两条既有失败语义继续保留。该检查点未获最终审查，也未提交或推送。

父代理补做了独立客体反汇编（不是沿用 SBI.elf 或 worker 的输出）：
`target/image/release/riscv64gc-unknown-none-elf/release/prog-memory-transfer-fresh`
的 SHA-256 为 `991c6ff4a8510ebd1936f12de78b9a2cc835a31164e323512246ef9f5b2d9d0c`。
`llvm-objdump --disassemble-symbols='env::ecall::trap' --demangle <ELF>` 实测
0x1a632 为 `9002 ebreak`、0x1a634 为 `00a83023 sd a0,0(a6)`；
再从 `--start-address=0x1a636 --stop-address=0x1a640` 解码，
错误 PC 处的上半字 `00a8` 实际是一条合法压缩 `addi a0,sp,0x48`，
然后继续执行 0x1a638 的 `sd a1,8(a6)`。
因此不仅 a0 的缓冲写入缺失，a0 寄存器还会被显式改为 sp+72；
不能把错误执行简化为“干净跳过整条指令、总是只读旧值”。
命令输出与该 ELF 哈希保存于忽略的 `trace/fresh-guest-trap-{disassembly.txt,elf.sha256}`，
已交 ABI worker 与 reviewer。重试成功后无需为了形式额外再陷一次；
待实现的约束是确认长度后才执行/推进、重试仍失败时明确归故障责任。

reviewer 也已亲验上述指令。但不能据 `390752 - 0x48` 的算术关系
就认定该 TaskId 直接来自被改坏的 a0：trap 返回三元组使用隐藏返回缓冲，
最终值还取决于 caller 如何读取该缓冲。RISC-V call/jal 不自动压栈，
不同调用点的栈深也未必相同；“每个 API 都返回同一 sp 派生常量”未经测量。
已要求最终报告分清寄存器执行、缓冲写入和调用者解码，不再扩大数值推断。

ABI 返工检查点将边界归位：`entry::call::dispatch` 只服务调用、不再处理 PC；
真实 trap 的唯一环境入口负责重试失效、以 `Option` 读取长度后推进 PC，
重试失败或文本不可读时先显式将目标 Task doom 为 Fault，再交回换出。
既有 reap 会优先保留 `state.dying()` 的原因，不将该 `None` 当正常 Reap。
新增 `syscall_sync_failure` 用真实 Caller/Space/Frame 和两次顺序的一次性注入，
确定性检查成功重试后的二字节推进与答案，以及重试再失败的不执行、不推进、
哨兵未改、责任仍在、Task 已判死；没有扩建永久失败注入器。

父代理在整合主树（含握手、导出和 pending 发布修复）实跑：
默认健康面 **69 passed + 缺 initrd 哨兵 1 failed**，退出 101（7.94s），
新 `syscall_sync_failure` 通过；Fresh 精确 DA **1 passed / 69 filtered**
（10.38s）；宿主 **50 passed**（0.052s）。这是合并后的读数，不沿用子树早期结果。
此版仍未封存：PC 加法须恢复 checked、测试须明确断 Fault 原因并补
“重试成功但入口字节不可读”分支；已交同一 writer 集中收口，reviewer 同步核实。

#### 最终检查点（已通过独立实测复审）

ABI writer 的 `9945338` 已合入并停止写入。入口推进用 checked_add，
溢出不改原 PC、不执行调用、明确 Fault；`syscall_sync_failure` 同一用例现在覆盖：

1. Failed 后成功重试，真实二字节 `c.ebreak` 推进 2，SelfId 答案正确。
2. 第二次同步也失败：不推进、不写答案、pending 保留，
   且 `dying() == (Fault, EXIT_FAULT)`，不是仅断 dead。
3. Nest Clean 但入口页无 X：不猜长度、不推进、不写答案，明确 Fault。
4. 真实四字节 `ebreak` 正确推进 4。

不新增持续注入。`retry_invalidation` 尾部清理失败只作静态复核、未注入覆盖；
checked 算术保留，不为入口不可达的溢出状态新建测试抽象。
“未修版本三轮也绿时谁先结清失效”仍未定位，不用它替代以上确定性证据。
ARMED 在 claim 后写入且每轮请求先重置 PREPARED；claim 后的返回 Busy
在当前 Transfer 实现只来自 unpublish 的 Shootdown 类，不是任意早错 Busy。
但 B 不据此声称具体哪次 SBI 调用失败或源 PTE 已撤；A 另有源 Store→Load 的证据。

父代理最终整合实跑：

| 检查 | 结果 |
|---|---|
| 默认健康面 | 69 passed + 缺 initrd 哨兵 1 failed，退出 101（7.35s） |
| Fresh 精确 DA | 1 passed / 69 filtered（10.38s）；小收口后 10.44s；负对照恢复后 10.38s |
| D1 / TLB | 各 1 passed（0.42s / 0.17s） |
| accept / product / system-fault | 各 1 passed（10.68s / 5.33s / 15.20s） |
| 宿主 observer | 50 passed（0.051s） |
| workspace offline dev/release | 通过；programs 10 组、kernel release 6 组既有告警 |
| kernel all-targets semihosting offline dev、加 fresh-scene dev/release | 通过；dev 无告警，普通 release 6 组既有告警；另跑 fresh-scene DA release 无告警 |
| 相关格式 / 脚本 | 20 个 Rust 文件按 edition 2024、skip_children 检查通过；nu-check 为 true，diff check 通过 |

未全树格式化；messenger 两文件的既有 import 漂移未重排。
新 helper queued 的定义和导出共用 fresh-scene 特征门，避免默认 debug 构建新告警；
Fresh reader 去掉临时 unused alias、复用既有 note 输出，未改 raw 入口或阶段次序。

父代理在最终树另跑不带 fresh-scene 的精确 DA 负对照：
Transfer 真成功，程序以 `the transfer committed` 明确失败（10.48s），
firmware 101、脚本 1，无正式 summary；恢复特征门后的精确命令重新通过。
现场为忽略的 `trace/fresh-final-unarmed-{qtest.log,feed.log}`。
它证明本景不会在夹具未武装时伪通过，不把这次负对照内的级联退出当成功场景。

最终独立 reviewer 在父树的隔离副本逐条修改、构建、实跑、还原，
以下六项 mutation 均令 `syscall_sync_failure` 成为唯一新增失败：

| 变体 | 被哪条确定性判据拒绝 |
|---|---|
| 文本不可读时造 4 字节长度 | Clean 不可读分支 |
| None 被当成 2 字节长度 | Clean 不可读分支 |
| 长度恒为 2 | 真实四字节 ebreak 分支 |
| 先取长度、后 retry | Failed 后成功重试分支 |
| Fault 改为 Reap/0 | dying 的 cause/reason 断言 |
| 删去显式 doom | dying 的 cause/reason 断言 |

各次其余健康用例不变，缺 initrd 哨兵仍失败；每个变体结束均核对文件还原，
不是删断言或放宽回收检查换绿。reviewer 最终与父树字节一致的副本读数：
健康面 69 passed + 哨兵 1 failed（8.40s）、Fresh DA 1 passed / 69 filtered
（10.43s）、宿主 50 passed（0.051s）；其先前 Fresh 连续三次通过，
D1/TLB 各 1 passed（0.42s/0.22s）。
queued 特征门和 reader 的小收口也经独立静态与 UART 对照核实。
reviewer 未重跑 accept/product/system-fault，仍引用上述父代理实际读数。

该阶段已通过独立实测复审，批准本地封存；未获得共享分支合并或推送许可。
Frame 身份/零分配、真实硬件弱内存、所有 SEE 的远端完成契约、Seal 并发、
设备/DMA 和真实 Platform 产品接线仍不在本景完成范围，不称 T4/T5 总验收完成。

### T4D3A Seal 补关与入口覆盖（已通过独立复审）

基线为 `88dad547`。本阶段补 PieCall::Seal/Revoke 的真实解码派发，
不把同 Task 的先后次序当成真实并发。初次测试确认 Seal 的部分失败缺陷：
失活已落地、close_maps 同步失败留下责任后，再次 Seal 会被存活门控的
PieSnapshot::owner 拒绝。用户明确批准原属主重复 Seal 补完关闭，
对象保持失活，不恢复访问或迁移管理权；该批准不是推送或共享分支落地许可。

生产修复仅在 Seal 入口增加补关资格：
资源 Meta 已失活且稳定的 Mail.owner 等于当前 Task，才可越过失活导致的
owner 缺席；token 仍须在本任务表中，且原来的“已死、无 maps”早错仍返回 Dead。
活 Meta 继续走原 owner/alive/Grant 判据，不能把 Grant 失效当成补关理由。
未全局修改 PieSnapshot::owner、alive、manages，也未改锁、租约时机或 ABI。

| 状态 | 返回/效果 |
|---|---|
| Meta 活、Grant 已失效 | Denied，不失活、不撤图 |
| Meta 死、仍有 maps、非属主 | Denied，不碰尚欠的关闭责任 |
| Meta 死、仍有 maps、原属主且 token 仍在 | 继续 close_maps，成功 0；同步失败仍保留责任 |
| Meta 死、无 maps | 既定 Dead，不恢复任何权限 |

纯补关按资源归属与关闭责任判定：Meta 和 Grant 都已死时，原属主若仍持表中 token
也可以补关；不能把实现误写成“所有 Grant 死的情况都拒绝”。

新增五项内核用例：

- Seal 的非属主/无效 token、staged reserved 与 PoleOperation Busy 早错，
  以及正常失活和关图；增加活 Meta/死 Grant 的拒绝反例。
- Seal 后同配置 Transfer 从 Busy 变 Denied。此配置的保留窗口本身 pin 页，
  证明的是管理拒绝先于物理资格，不宣称一次可成功迁移变成拒绝；
  被拒副作用的比较基线在 Seal 完成后，保留窗口仍能解析。
- 真正成功的 Transfer 后 Seal：目标承载/内容保留，源 Pole 自己关系关闭，
  无关映射不变；只属顺序属性。
- 注入 Seal 关闭同步失败：Busy、失活不回滚、maps/失效/禁复用责任保留，
  非属主补关 Denied；原属主经 trap::envcall 重试 0、关闭完成，
  再次 Seal 精确 Dead，管理判据始终为假。不再用 Shut 绕过此重试合同。
- Revoke 的目标/根 token/联合授出资格拒绝、真正撤子及不影响其他分支。
  联合条件未被单独隔离，撤销后的 Transfer Denied 也不独证 manages 门的必要性；
  不用造假的 token 表追求不可达单门反例。

运行槽夹具放在 conductor 现有的测试子模块，因 Pie 入口读取
current().running_task，不能只传 dispatch 的 ident 冒充在座调用者。
Held 转 Ready 先同步 release_held，不 launch 进调度队列；
结束腾槽/摘徽记，已 doom 的任务不复活。seat 对 kernel_sp、内核 TP 与时限的写入
不还原，已在夹具中明确限定用途。它不是完整 guest 调度或 ecall 端到端：
只有补关重试驱动 trap::envcall 并读取真 c.ebreak 长度，其余走服务 dispatch。
reserved 预置使用生产 record 的 staged 形状，不宣称覆盖整个跨 Team spawn 事务。

父代理在最终主树实跑：

```sh
nu scripts/qtest.nu --package kernel -- -- pie_
nu scripts/qtest.nu --package kernel
```

- 精确五例 5 passed / 70 filtered（0.52s）；
  健康面 74 passed + 缺 initrd 哨兵 1 failed，退出 101（6.05s），不称全绿。
- kernel all-targets semihosting offline dev/release 通过，
  dev 无告警、release 6 组既有告警。
- D1/TLB 各 1 passed（0.42s/0.17s）；
  Fresh 精确 DA 1 passed / 74 filtered（10.29s）；
  accept/product/system-fault 各 1 passed（10.39s/5.33s/13.54s）。
- 四个相关 Rust 文件按 edition 2024、skip_children 检查通过，diff check 通过。
  worker 的末次检查点另有两处排版漂移，由父逐处修正，未全树重排。

初审期间 reviewer 的 scratch Git 指针曾误指向其 Delta 树，已恢复并比对；
父树实核初次三文件差异指纹仍为原检查点，未发现源码丢失。
默认 index 不宣称未变，也未 reset；所有父侧验证/提交继续用独立临时 index。
后续 mutant 副本排除 .git/任何 gitdir 指针，禁止用 checkout/reset/restore 还原，
不将副本代码回拷任何被审工作树。

最终独立 reviewer 对父树四个代码文件复算 diffSHA，
为 `807cd8f07c3bf440a7359c9dc8e8f54a169ac8cea248197db538374ff2faeb4e`，
与局部格式修正后版本逐字节一致；文档另行记录，不改变这四个文件。
副本排除 .git，确认不是 Git 工作树后，逐项局部 patch、构建、实跑并在副本还原：

| 变体 | 被拒绝的用例/原因 |
|---|---|
| 删除 cleanup 例外，恢复旧门 | partial_failure：原属主重试 Denied，不是 0 |
| cleanup 放宽到活 Meta | authority：活 Meta/死 Grant 被错误 Seal |
| cleanup 不核稳定属主 | partial_failure：非属主错误补完关闭 |
| 删除 dead/no-maps 早错 | partial_failure：完成后的调用答 0，而非既定 Dead |

四条 mutation 均被抓。reviewer 独立复跑精确五例 5 passed / 70 filtered、
默认健康面 74 passed + 同一缺 initrd 哨兵 1 failed，退出码分别 0/101；
未重跑 release 场景，release 证据仍是上述父代理实际矩阵。
with_running 的 release_held 和“不复活 doom”守卫经静态复核；
删除任一守卫的探针仍五例通过，故不冒称这两个防御分支已有动态覆盖。
Revoke 联合条件、staged 预置与服务派发的覆盖边界仍按上文限定。

本阶段已获独立审查接受，批准本地封存，未获得推送或共享分支落地许可。
真正跨 Task 的迁移/撤销/退出竞争、多 Hart 确定性交错、
设备/DMA 与真实 Platform 接线仍未完成，不以入口级覆盖宣称 T4/T5 总验收通过。

### 普通 Acquire 窗口与跨 Task/Space 受控交错（本轮收尾，待独立复审）

本轮只补 Transfer 剩下的两项验收缺口，不动既有实现：
**普通 `Acquire` 形成大叶之后的那一次完整移交**、以及**跨 Task（独立 Space）的
竞争与退出交错**。两项都是**内核健康用例**层的取证，没有新建场景、
没有改 trace schema、没有改任何生产路径或注入钩子。

#### 一、大页前提：当前没有合法构造（用例不把叶形当合同）

新增 `memory_transfer_pages` 里的一例（`plain acquire window`）：起点是普通
`Flat::acquire` 的整段来源（512 页、页对齐）＋一个 2 MiB 对齐的 VA 窗口
（`0x6800_0000`、`BASE`）＋统一 `RW`，按页物化、从映射逐页写回（每一页头一个字节
就是它的页号，且都翻到自己的承载上），然后把中段那一格 4 KiB 交出去。判据：只那
一格让出翻译；邻页的承载 PA、内容与翻译不动；目标槽拿到同一个 `Arc<Frame>` 与同一
个 PA；**撕下来的那一格没有 PTE，所以没有哪一片叶能跨过它**，窗两端与邻页因此
只能是 4 KiB 叶（与分配是否连续无关，不复制 `Nest` 的分解算法当第二 oracle）；
`audit()` 干净。

它**不声称**大叶存在，也**不把 4 KiB 钉成合同**：叶形是分配布局的事，连续对齐的
分配会把这一窗变成 2 MiB 叶，两种都由本例照过。大叶那一条仍**未验证**：本轮在同一
台健康面上实测，这 512 页按序物化后 PA 首址不是 2 MiB 对齐、相邻处有断点，
`Nest::leaf_span` 两端都只报 4 KiB——那是**这一次布局的读数**，不是“普通 `Acquire`
永远不能成大叶”的结论。

- 实测命令：`nu scripts/qtest.nu --package kernel -- -- memory_transfer_slots`
  （1 passed）。这一例嵌在 `embedded::tests::memory_transfer_slots` 里——那是
  `tests/health/transfer.rs` 的 `accept()` 用例集（本轮从 22 例变 23 例），**不是**
  `memory_transfer_pages`（后者跑的是未改的 `syscall::transfer_pages`）。
  这一层是**原生 Flat/Transfer**（内核内的真实来源、事务与页槽），
  不是 guest ecall 那一层；它跑的是同一段 2 MiB 对齐窗口的完整移交。
- 实测 PA 布局的读数来自本轮临时探针（已拆除，不留在树里）：512 页的 PA 首址
  不是 2 MiB 对齐、相邻处有 16/26/32/38 处断点、`leaf_span` 报 4096。
  这些断点数**不是**分配器的合同，只是这一次布局的事实。
- 要跑通“大叶形成后的 Transfer E2E”，必须先有连续性/对齐保证（或经用户裁定扩
  分配 API）。本阶段不扩分配能力、不手工写 PTE、不借 `BorrowedRegion` 充数，
  也不盲试到偶然绿：**这一条仍未验证/当前可复现构造受阻**，如实落这里。

#### 二、跨 Task／跨 Space：受控交错矩阵（不是并发）

新增 `transfer_across_tasks_and_spaces`（`tests/health/syscall.rs`）。真实可达形状
只有一种：这段来源的管理者（`lord`）是那把根 Grant 的持有者，另一位任务（`peer`）
在**另一个 Team／另一个 Space** 里持 `accord` 交出的子分支，并在自己空间里 `open`
出压着同一批帧的关系。四个用例：

| 用例 | 交错的点 | 判据 |
|---|---|---|
| 外来空间那一页 | 管理者的移交撤的是**别人空间**里的 PTE | 两个空间那一页都让出翻译；目标槽拿到同一承载与 PA；持窗者那一枚仍欠关图（`meta.opened`），它自己一手 `Shut` 关得掉（关图责任归它，本用例不宣称第二次 `Shut` 的返回）；清单里两条关系（含外空间那条）都在 pend 释放之后到消费者手里 |
| 持窗者先退出 | `doom` 的 cull 与清理先收掉它名下的图 | 退出后它的 pies/heirs 清空、关系与 PTE 都不在；承载与内容不动；管理者之后照旧搬得动那一页 |
| 撤掉那条分支 | lord 侧 `Revoke` 收回那一枚 | 持窗者空间里那段关系随撤销消失、lord 自己那条不动；承载与内容不动；被撤的分支不再是可用的一端（`Denied`） |
| 撤页确认失败 | 一个 Nest 上确认失败 | 结果 `Busy`；未结失效留在那个 Nest（`has_pending_invalidation`）；已让出的页**不回填**；两端页槽按原态还回（源 Resident、目标 Fresh）；`retry_invalidation` 后重试真搬走 |

边界与线性化依据（不造状态、不冒充并发）：

- 双 manager 同来源**不可达**：`manages` 只认私有属主或本表里 `sire`／`heir` 都空、
  与相位同 Arc 的那一枚根 token；交给别 Task 的是子 Pie，它管不了这段来源。
- 别的 Task 替调用者持 pend 也不可达：入口以调用者自己的 pend 为准；
  `Revoke`/`doom` 侧的 `with_tasks_checked` 会把子树里每个 Task（含 lord）的 pend
  一起取上，故“真并发”必须由**别的 hart**提供，本夹具只做受控交错。
- 本用例里每一手都是真入口、真任务表、真 pend，但先后发生：**不是并发**。
  真多 Hart 的同刻交错不在本夹具的证明范围。
- 净额：帧净额（数据帧 ＋ 整页池）逐例与整体都为零；逐类账里只有 `Prime` 与
  `Plain` 两格不归零，其余（页表 `Table`、任务/团队/空间、栈、各类帧）一律归零。
  那两格是本轮实测到、**未能归属**的残留，不当成结论、也不拿宽松条件吞掉：
  `Prime` 每用例 ＋1（整页被块池建下，它的账同时进 `block_occupied`，故帧净额
  看不见它），`Plain` 每用例约 ＋4.5（小块仍处于已取状态）。
  试把每用例末尾的 `Caller` 返回（`doom`）也算进窗口时也是同样的残留，且临时探针
  证实那些 `Task`/`Team`/`Space` 的强引用在单独一轮里确实会归零，故**不能**把这两格
  说成“夹具保活”或“不可释放的全局缓存”：归属与逐轮上界仍是待核项（已写进代码注释）。
  现在把夹具自己的两台 Task/Team 建在基线之前、终结在断言之后，于是窗口里量到的
  只是本景自己的动作（来源、Pole、关系、页槽、承载），这也是 `Task` 能归零的原因。

#### 三、本轮实际读数

| 检查 | 结果 |
|---|---|
| `nu scripts/qtest.nu --package kernel` | **77 passed** ＋ 缺 initrd 哨兵 1 failed，退出 101（6.05s） |
| 同命令 `-- transfer` | 8 passed（含两条新用例）0.82s |
| D1 常规景（`--scene memory-transfer --feed-script … -- --features semihosting`） | 1 passed（0.37s） |
| D1 带护栏（`CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true` … `-- scene`） | 1 passed / 77 filtered（0.82s） |
| TLB 常规景 | 1 passed（0.17s） |
| accept / product / system-fault | 各 1 passed（10.41s / 5.33s / 13.46s） |
| 三个改动文件 edition 2024 格式检查、`git diff --check` | 通过（未全树重排） |

健康面的用例数是 78（HEAD 的 77 个加本轮那一例整机用例）；本轮改的两条里
`plain acquire window` 是 `memory_transfer_slots`（`transfer.rs` 的 `accept()`
用例集，22 → 23 例）里的新一例（不新增测试名），
`transfer_across_tasks_and_spaces` 是新的一例。未跑 Fresh DA 与宿主套件：
本轮没有动它们的输入（只有两个内核测试文件与 `embedded.rs`，生产路径、注入钩子与
trace schema 一字未改）。

本阶段**未**获得独立复审、也未获推送或共享分支落地许可；
真多 Hart 的确定性交错（需要新场景与可能的 test-only 注入钩子）、
Fresh DA 与宿主套件重跑、设备/DMA 与真实 Platform 接线仍不在本轮范围内。

### b2 归因修正与真多 Hart 交错场景（本轮，待独立复审）

基线为 `b2b75a05`。本节两件事：按复审的真归因修 b2 的两处记录/夹具，
并补上“真多 Hart 确定性交错”那一景（b2 记的最后一个未覆盖项）。

#### 一、b2 归因修正（只动夹具与记录，不动断言口径）

- **夹具漏收尸**：`cross_task_interleavings` 的 `retire` 原先只 `doom` 不 `reap`，
  而名册里那一格的死 `Task` 弱引用会钉住一份 `Arc` 的内脏（实测每台 512 B），
  于是逐类账里的 `Task` 从 0 涨到 7（1 台热身 ＋ 6 台 Caller）。现按生产 `reap`
  同一条收尾：`retire` 末尾 `team.prune_tasks()` ＋ `roster::prune_dead()`。
  修后 `Task` 归零，不再拿“夹具保活”解释。
- **`Prime`/`Plain` 不是线性增长**：复审用同一对 Caller、同一形状连跑 6 轮，
  第 0 轮 ＋2/＋7、第 1..5 轮全为 0——那是一次性的池子惰性，不是每用例的泄漏。
  故基线改到**每对 Caller 先用一个不终结任何任务的形状热身一轮**之后
  （`holder leaves first` 的目的就是真叫走持窗者，同一对 Caller 跑不了第二轮，
  故它也只能靠这一轮热身）。两格仍只钉“不增长”（`<=` 基线）；本夹具自己跑出来
  两格都是 0。帧净额与 `Task`/`Team`/`Space`/`Table`/栈等一律仍然相等。
- **记录修正**：`plain acquire window` 嵌在 `embedded::tests::memory_transfer_slots`
  （`transfer.rs` 的 `accept()`，22 → 23 例），准确命令是
  `nu scripts/qtest.nu --package kernel -- -- memory_transfer_slots`；它证的是
  **原生 Flat/Transfer** 那一层，不是 guest ecall。
- **措辞修正**：跨空间那一例里“持窗者自己 `Shut` 关得掉”不写成“且只关一次”——
  本用例不宣称第二次 `Shut` 的返回，不为这一句话扩矩阵。

实测：`-- transfer_across_tasks_and_spaces` 1 passed / 77 filtered（0.67s）；
健康面 77 passed ＋ 缺 initrd 哨兵 1 failed（6.00s）。

#### 二、真多 Hart 交错：`memory-transfer-race`

新增一景（`programs/src/harness/probe/memory_transfer_race*` ＋
`kernel/tests/health/race_scene.rs` ＋ 内核 `race-scene` feature ＋ qtest 那一格）。
它要证的是一件前面几景都没有的事：一次真移交在**在途窗口**里驻留不让 hart 的时候，
**另一个 Team／另一个 Space** 里的对端能在**另一颗 hart** 上进内核、真登记一次冷缺页。

形状（每一步都只有一种走法）：

- **管理端**（引导镜像那一台，Team 0／Space L）备一份两页私有来源（第 0 页要搬、
  第 1 页是目标空槽）与一页**共享控制页**，两份各自铸成共享根（`shared: true`），
  再各授一枚子分支给对端。控制页与迁移范围**分开**，各有各的寿命。
- **对端**由 `Loader` 装进**另一个 Team**（Space P），在自己空间里把两条子分支各
  `open` 成关系（来源那条盖两页），先读一次第 0 页（真物化、真发布一条 PTE），
  把 tid／两个 VA／那一页的读数报回控制页，然后等在控制页的 `START` 上。
- 管理端等到那份报告，摆好请求，发起真移交（第 0 页 → 第 1 页）。
- **内核夹具**（复用现有 `arm_in_flight`：占槽之后、撤 PTE 之前，`caller.pend` 仍持着、
  `SIE` 关着、不持任何 core 锁）按调用者空间读出控制页，核出压着源页的**两条**关系
  （自己那条 ＋ 别人空间那条，按 ASID 分），当场读出**那条外空间关系真握着一条翻译**
  （`FOREIGN`），放开对端，再用 `messenger::queued` 有界自旋（`rdtime`）
  等**那条外空间关系上真的排上了一个等待者**；等到了才写 `ARMED` 返回，到点写
  `TIMEOUT` 返回——不冒充登记过，也不丢资源。
- 对端看见 `START` 去读第 1 页：那一页此刻是移交留的空位（`Reserved`、**没有 PTE**），
  于是那一次读是一次真冷缺页（`Load` 未结算）→ 真登记 → 被那一次移交自己的
  `wake(affected)` 放行 → 重试同一条指令，读到的是**搬过来的那份承载**（花纹和与
  第 0 页一致；新清零页只会读到 0），写回读数后正常收工。

为什么它是真的：

- 管理端驻留的那一段就在**真入口、真事务、真在途窗口**里（不是 `with_running` 换
  调用者、不是 sleep、不是边界上的 spawn-exit）；对端那一手是真页错误、真等待位、真
  `Reap`。
- 登记是**内核侧**看出来的（`messenger::queued` 按那条外空间关系查），不是程序自己
  举的牌子；放行来自移交自己的 `wake(affected)`；内容判据是**同一份承载**在**另一个
  空间**里被读出来的。
- 没有常开 ABI、没有常开 trace schema：夹具与 `queued` 那一口都只在
  `cfg(debug_assertions)` ＋ 各自的选景 feature 里编得出来。

实测读数（release ＋ `CARGO_PROFILE_RELEASE_DEBUG_ASSERTIONS=true`，
`--features semihosting,race-scene`）：

```
memory-transfer-race: phase=fault peer=11 va0=0x1d000 va1=0x1e000 read0=0x7f800
memory-transfer-race: phase=fault transfer=Ok(()) kernel=armed foreign=1
memory-transfer-race: phase=fault read1=0x7f800 read0=0x7f800
memory-transfer-race: phase=fault joined cause=Reap reason=0x0
memory-transfer-race: close done
```

结构化导出里对端那一串（同一轮）：`memory.page_fault va=0x1e000 Load resolved=false`
→ `room.wait tid=11` → `room.wake tid=11` → `memory.page_fault va=0x1e000 Load resolved=true`
→ `room.exit tid=11 reason=0x0` → `room.reap tid=11`；控制台侧另核两台都正常退场、
`task: all tasks exited, system halted`、`doom: held=0 starved=0 blocked=0 nudged=0`。

判定分两面：**程序侧**当场核夹具那一步是 `armed`、那条外空间关系真握着翻译
（`foreign=1`）、两页读数一致且非零；**宿主驱动**
（`programs/tests/memory-transfer/race.py`）另按已知 tid 与已知 VA 重核同一串真事件
（恰一次登记、恰一次放行、目标页先未结算后已结算的两次取数缺页、登记早于放行早于
那次重试、第 0 页上不许有未结算的缺页、恰一次 `Reap`），并另核两台都正常退场。
宿主侧回归（`programs/tests/memory-transfer/test_race.py`）18 例：真读数照过，
`kernel=timeout`、`foreign=0`、读数不等/为零、目标页没等过、第二次登记、第二次
`reap`、非零出口码、末行截断、整行不是 JSON 各当场失败——超时不被当成证明。

#### 三、SMP 负对照：只有一颗 hart 时那一手不可能进展

- `QEMU_SMP=1` 这一档**跑不到本景**：带护栏那一档会把内核自己的自检跑起来，
  而 `tests/health/hart.rs` 钉的就是“机器报的 hart 数 > 1”（它是给参数表 `-smp 4`
  立的那条哨）。实测 `QEMU_SMP=1 … -- scene` 在**引导自检**上就红（
  `机器只报了 1 颗 hart`），与握手无关。
- 故本景的“握手不能前进”负对照由**夹具变体**给出，不由 SMP=1 冒充：
  把夹具里那一手放开（`START=RELEASE`）拆掉，对端永远不登记 → 夹具到点写 `TIMEOUT`
  → 管理端当场红（`kernel=timeout`），导出里也没有 `room.wait`/`room.wake`。
  那正是“该握手无法前进”的样子，不拿任意超时当证明。

#### 四、本轮实际读数与未覆盖

| 检查 | 结果 |
|---|---|
| `nu scripts/qtest.nu --package kernel` | **77 passed** ＋ 缺 initrd 哨兵 1 failed（6.00s） |
| 同命令 `-- transfer` | 8 passed（0.82s） |
| 同命令 `-- transfer_across_tasks_and_spaces` | 1 passed / 77 filtered（0.67s） |
| 本景（SMP=4，护栏档） | 1 passed，五条读数如上（退出码 0，`scene-memory-transfer-race-summary.json` 落盘） |
| 本景宿主驱动与回归 | `race.py` 逐格判定通过；`test_race.py` 18 例 OK；同目录 `discover` 68 例 OK |
| D1 / TLB | 各 1 passed（0.42s / 0.17s） |
| Fresh 护栏景 | 1 passed / 77 filtered（10.40s） |
| accept / product / system-fault | 各 1 passed（10.40s / 5.34s / 13.60s） |

未覆盖与边界（不冒 Done）：

- 本景只走“**对端在一次真移交在途时冷缺页**”这一种合法竞争。
  **退出/撤销**那两种没有做成同刻竞争：任何“给出一枚本景来源的子分支”的 cull
  （`Shut`/`Revoke`/`doom`）都会经 `with_tasks_checked` 取到**管理端自己的 pend**，
  而那一把 pend 正被在途窗口持着 ⇒ 那种动作只能**排在管理端放行之后**完成，
  在窗口里“等它完成”就是自锁（复审那一封交接里点名的那一条）。故本景不造这种组合，
  也不把“请求已发”写成“已完成”。
- 普通 512 页窗口那一例的**大叶前提**仍未构成（同 b2 那节：当前 PA 布局首址不对齐、
  相邻处有断点），本景不碰它。
- 设备/DMA 与真实 Platform 接线仍不在本轮范围内。

本阶段**未**获得独立复审、也未获推送或共享分支落地许可。
