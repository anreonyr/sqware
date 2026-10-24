# VirtIO Block Driver 职责边界

本检查点基于 PR22，收口完整 Driver 的名称、恢复执行器的边界及原生验收发现的回收问题。
最终短程源码验收已完成，范围与限制见下文。

## 状态权威

| 功能 | 唯一权威 |
| --- | --- |
| 实例构造、启动、停止、整个 Team 回收与恢复准入 | Control |
| 声明目标的观察与有条件恢复请求 | 普通常驻 Daemon |
| 通用装配、授权交付和退休依赖 | Assembly / System |
| 设备目录、来源身份、认领与逻辑绑定 | Hub |
| PCI 配置、BAR 范围与 Bus Master | PCI Driver |
| Domain、IOVA 分配、翻译、失效、队列和数据引用 | IOMMU Driver |
| VirtIO Block 准备、激活、关闭及重试进度 | `virtio-blk` 的每设备 Coordinator |
| 精确 BAR 保留映射、执行者子授权、reset 与真实静止回读 | 内部恢复 Executor |

常驻程序为 `virtio-blk`，镜像为 `prog-virtio-blk`，代码位于
`programs/src/driver/virtio/blk/`。它收集所有匹配的物理设备，并在同一受保护进程中
逐设备推进 Coordinator。每个设备独立保留 Operation、未决请求和首错。

恢复执行器位于 `blk/recovery/`。Executor 持有实际 Dock、PCI 来源与资源水位；
Attachment 保存当前授权与 reset 回读元数据，实际映射仍由 Executor 持有。
恢复子模块不依赖 Coordinator、Operation 或 Stage。Coordinator 的逻辑完成水位
与 Executor 的资源水位继续分别保护各自边界。

## 授权与失败责任

`device/coord`、协调入口 mark、执行者 client 协议、每绑定授权 mark、PCI BAR mark、
恢复效果 party=4 及操作码 1/2/3 保持原义。名称迁移不新增进程、协议兼容层或生命周期账。
源远端与历史 PR 的旧名称属于各自代码检查点，旧现场不改名冒充新验收。

BAR 映射和恢复授权必须在 Publish 前建立，关键进程保护必须先于硬件访问发布。
Control 确认整个 Work Team 停止后，才能推进 Invalidate → Disable → 设备 reset/回读
→ Quiesce → Reference Release → Detach → Unplace。失败保留实际资源、进度与首次错误。
β 的关键进程死亡采用引用清理前的保护性终止，死亡本身不构成 DMA 排空证明。

## 回收与原生夹具修复

Loader IPC 夹具应复用生产 `control::poll`，接收并应答内部身份查询。
原 12 秒期限、owner 拒绝、实例登记、peer/worker join、heirs、镜像缓存和最终回收
断言继续生效。共享命名空间的删除观察使用同一父节点的单次名称查询与成功的子项
快照，避免使用等待路径出现的接口证明路径已删除。

原生诊断确认：调用方退出后，已构造的主任务与 Team 已被内核回收，Control 仍可能
保留旧的 Owner 回执并停在 Stopping。构造来源与内核可见性证明子树已回收后，
仍先执行全部退役依赖，最后核销该回执。已认领与未认领实例使用同一判据；Loader
和 Account 夹具都必须等待实例真正 settled，原退役顺序与期限断言继续保留。

发布前中止夹具应等待执行者实际收到每绑定 BAR 授权，再发起 Close；它始终不交付
DMA 窗口，因此 Publish 仍不可能发生。等待使用已有认证 client 来源与实际授权元数据，
保留原期限和唯一性检查。真正的 detach 与最终回收断言保持严格，生产路径仍允许
在 Attach 前合法关闭。

## 最终验收

最终源码指纹为
`d21f1ef4d9832d84338e1901e7698475fb9d3688bdc589d8db26d1b52328699f`。
本次实际验收结果如下：

- 37 项短程验收通过，包含原始九个四核场景及四/八核设备、DMA、故障、真实回读
  拒绝、内存转移竞态、Shell 和 Daemon 场景。Loader 与 Account 的完整流程、发布前
  中止均通过；`system-fault` 和发布前中止的物理帧均为 85→85。
- 其中 31 项原始退出码为 0；PCI、IOMMU、VirtIO Block 三目标各四/八核的六项
  保护场景原始退出码为预期的 101，独立判定器均为 0。这是引用清理前的保护性终止，
  不构成 DMA 排空或回收证明。
- 回收修复后的实例宿主测试 39 项、结构测试 22 项通过，workspace check 通过，
  仍有五项既有警告。先前命名检查点的 65 组／774 项 Rust、126 项设备判定器及
  27 项竞态脚本测试结果分别保留，不冒充本次重跑全部宿主测试。
- 同一独立 Flash 审查组完成迭代审查；最终退役顺序与回收证明未发现具体阻断问题。

结果保存在 `target/driver-composition/device-domains/native-boundary-final3-current.json`
与 `boundary-final-acceptance.json`，每项包含实际命令、源码指纹和独立现场。
这些是当前工作区的本地验收产物，位于 Git 忽略目录，不随源码提交。
此前失败现场继续保留；默认 health 的历史缺 initrd 哨兵失败未计为通过。

按用户 2026-10-25 的要求，不再重复长测。PR22 已通过的 1024 轮及保存的失败保留为
历史检查点；最终命名源码没有再次进行 1024 轮测试，短程结果单独记录。
