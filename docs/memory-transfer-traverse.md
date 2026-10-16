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

### 后续 T4 等待与结果合同（已确认，尚未实现）

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

### T4C 入口补充合同（尚未实现）

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
