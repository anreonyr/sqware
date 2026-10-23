//! 那一份份声明（#[path] 拉进来的）与 PROGRAMS 这张单。

use super::UnitFile;

// **它们不在这份文件的自然模块树里**：那些目录（`driver/uart/`、`service/hub/`…）都拖着
// runtime / protocol 代码，`crates/image` 进不去。故只由 PROGRAMS 这一处按 `#[path]` 拉
// 进来一次——**唯一的声明点**。

#[path = "../harness/probe/accept/program.rs"]
pub mod accept;
#[path = "../service/account/program.rs"]
pub mod account;
#[path = "../harness/bench/again/again/program.rs"]
pub mod again;
#[path = "../harness/bench/beat/program.rs"]
pub mod beat;
#[path = "../harness/bench/load/busy/program.rs"]
pub mod busy;
#[path = "../harness/bench/again/churn/program.rs"]
pub mod churn;
#[path = "../harness/device/programs.rs"]
pub mod device;
#[path = "../harness/probe/device_ruin/program.rs"]
pub mod device_ruin;
#[path = "../harness/probe/device_guard/program.rs"]
pub mod device_guard;
#[path = "../harness/probe/device_confirmation/program.rs"]
pub mod device_confirmation;
#[path = "../harness/bench/group/group/program.rs"]
pub mod group;
/// 测具那 29 台（**探针 / 试客 / 压测台**，身子在 `../harness/`）：其中 15 台由编排域起
/// 而"哪几台进哪张镜像"这张表在宿主侧（`crates/image`）也要看得见 ⇒ 声明与身子同住、由本表拉进来
#[path = "../harness/guest/guest/program.rs"]
pub mod guest;
#[path = "../harness/bench/rig/hang/program.rs"]
pub mod hang;
#[path = "../service/daemon/program.rs"]
pub mod daemon;
#[path = "../service/hub/program.rs"]
pub mod hub;
#[path = "../driver/iommu/program.rs"]
pub mod iommu;
#[path = "../harness/probe/iommu/dma/program.rs"]
pub mod iommu_dma;
#[path = "../harness/probe/iommu/program.rs"]
pub mod iommu_probe;
#[path = "../harness/bench/load/load/program.rs"]
pub mod load;
#[path = "../harness/guest/lodger/program.rs"]
pub mod lodger;
#[path = "../user/login/program.rs"]
pub mod login;
#[path = "../harness/guest/member/program.rs"]
pub mod member;
#[path = "../harness/probe/memory_transfer/program.rs"]
pub mod memory_transfer;
#[path = "../harness/probe/memory_transfer_fresh/program.rs"]
pub mod memory_transfer_fresh;
#[path = "../harness/probe/memory_transfer_race/program.rs"]
pub mod memory_transfer_race;
#[path = "../harness/probe/memory_transfer_tlb/program.rs"]
pub mod memory_transfer_tlb;
#[path = "../harness/bench/load/park/program.rs"]
pub mod park;
#[path = "../harness/guest/passer/program.rs"]
pub mod passer;
#[path = "../driver/pci/program.rs"]
pub mod pci;
#[path = "../service/pipe/program.rs"]
pub mod pipe;
#[path = "../harness/probe/probe_bound/program.rs"]
pub mod probe_bound;
#[path = "../harness/probe/probe_coalition/program.rs"]
pub mod probe_coalition;
#[path = "../harness/probe/probe_control/program.rs"]
pub mod probe_control;
#[path = "../harness/probe/probe_denied/program.rs"]
pub mod probe_denied;
#[path = "../harness/probe/probe_lease/program.rs"]
pub mod probe_lease;
#[path = "../harness/probe/probe_operator_gate/program.rs"]
pub mod probe_operator_gate;
#[path = "../harness/probe/probe_operator_land/program.rs"]
pub mod probe_operator_land;
#[path = "../harness/probe/probe_owner/program.rs"]
pub mod probe_owner;
#[path = "../harness/probe/probe_rack/program.rs"]
pub mod probe_rack;
#[path = "../harness/probe/probe_rack_guest/program.rs"]
pub mod probe_rack_guest;
#[path = "../harness/probe/probe_rack_mount/program.rs"]
pub mod probe_rack_mount;
#[path = "../harness/probe/probe_rule/program.rs"]
pub mod probe_rule;
#[path = "../harness/probe/probe_rule_other/program.rs"]
pub mod probe_rule_other;
#[path = "../harness/probe/probe_terminal/program.rs"]
pub mod probe_terminal;
#[path = "../harness/probe/probe_watch/program.rs"]
pub mod probe_watch;
#[path = "../harness/probe/probe_watch_after/program.rs"]
pub mod probe_watch_after;
#[path = "../harness/probe/probe_watch_gone/program.rs"]
pub mod probe_watch_gone;
#[path = "../harness/bench/rig/rig/program.rs"]
pub mod rig;
#[path = "../driver/recovery/program.rs"]
pub mod recovery;
#[path = "../driver/router/program.rs"]
pub mod router;
#[path = "../driver/rtc/program.rs"]
pub mod rtc;
#[path = "../user/shell/program.rs"]
pub mod shell;
#[path = "../harness/guest/sleeper/program.rs"]
pub mod sleeper;
#[path = "../harness/guest/subject/program.rs"]
pub mod subject;
#[path = "../system/program.rs"]
pub mod system;
#[path = "../harness/probe/system_fault/program.rs"]
pub mod system_fault;
#[path = "../user/terminal/program.rs"]
pub mod terminal;
#[path = "../driver/uart/program.rs"]
pub mod uart;
#[path = "../harness/bench/group/waiter/program.rs"]
pub mod waiter;

/// Unit declarations in source registration order.
#[rustfmt::skip]
pub const PROGRAMS: &[&UnitFile] = &[
    // Device service.
    &hub::PROGRAM,
    &daemon::PROGRAM,
    &pipe::PROGRAM,
    &account::PROGRAM,
    &terminal::PROGRAM,
    &login::PROGRAM,
    &shell::PROGRAM,
    &probe_terminal::PROGRAM,
    // 客人 / 过客 / 房客：量服务用的（去掉机器照转）。
    &guest::GUEST,
    &passer::PASSER,
    &lodger::LODGER,
    // 三台驱动。
    &router::PROGRAM,
    &uart::PROGRAM,
    &rtc::PROGRAM,
    &sleeper::SLEEPER,
    &subject::SUBJECT,
    &member::MEMBER,
    &system::PROGRAM,
    &accept::PROGRAM,
    &probe_denied::PROBE_DENIED,
    &system_fault::ENTRY,
    &system_fault::FAULT_UNIT,
    &system_fault::DEPENDENT,
    &system_fault::CHILD,
    // 产品那一侧那三台：**常驻驱动**（PCI／IOMMU／设备恢复），各自独立 Task/Space，
    // 由各景的启动单静态起；它们不在普通设备库存里拿平台窗，两扇/三扇窗都由声明供给定向交到。
    &iommu::PROGRAM,
    &pci::PROGRAM,
    &recovery::PROGRAM,
    // 场景工作负载与控制器。
    &device::WORK,
    &device::WORK_BEFORE,
    &device::WORK_AFTER,
    &device::CONTROL,
    &device::CONTROL_FAULT,
    &device::CONTROL_BEFORE,
    &device::CONTROL_AFTER,
    &device::CONTROL_RUIN,
    &device::CONTROL_FORCED,
    &device::CONTROL_GUARD,
    &device::CONTROL_SOAK,
    &device::CONTROL_SOAK_LONG,
    &device::CONTROL_CONFIRMATION,
    &device_ruin::PROGRAM,
    &device_guard::PROGRAM,
    &device_confirmation::PROGRAM,
    // 真实 Task 等待那一景：**装载者 ＋ 控制单元**两台——控制单元要真的被装载（有 sire），
    // 故它那一 Team 的成员带着收据；真实等待链本身在控制单元里（见 `probe/memory_transfer`）。
    &memory_transfer::ENTRY,
    &memory_transfer::UNIT,
    // 热多 Hart 旧 TLB 那一景：**一台**（supervisor 引导镜像＝控制者），
    // 读者是它同 Team 里 raw 起的一台真任务，故不为它另铸一台镜像。
    &memory_transfer_tlb::PROGRAM,
    // fresh 失败回退那一景：同样**一台**（supervisor 引导镜像＝控制者），
    // 两阶段的读者都是它同 Team 里真起的任务。
    &memory_transfer_fresh::PROGRAM,
    // 真实多 Hart 交错那一景：**装载者 ＋ 对端**两台（与 `memory-transfer` 同形）。
    // 对端必须真的被装载进**另一个 Team**（另一个 Space）——它在自己空间里真登记
    // 一次冷缺页，而管理端驻留在内核的在途窗口里。
    &memory_transfer_race::ENTRY,
    &memory_transfer_race::UNIT,
    // 真实 RISC-V IOMMU 那两景：各**一台**（supervisor 引导镜像＝控制者）。
    // `iommu-probe` 从引导账取设备区、建 DDT/DC/Sv39 表与队列，编程后由硬件翻一次并回读物理页号；
    // `iommu-dma` 则多一台**真实 PCI 端点**（`virtio-blk-pci`）：ECAM 发现、精确 BAR 交付、
    // 按 IOVA 真读一轮，再隔离之后重试同一段。
    &iommu_dma::PROGRAM,
    &iommu_probe::PROGRAM,
    &probe_owner::PROBE_OWNER,
    &probe_rule::PROBE_RULE,
    &probe_rule_other::PROBE_RULE_OTHER,
    // 事件那条路的正证客人（订阅 ＋ 过滤 ＋ 事件内容）：自持 `watch` 与 `land` 两位，
    // 不借别人的试验场（见它那份声明）。
    &probe_watch::PROBE_WATCH,
    // "退场即撤订"那一对：`gone` 订一条路、停一下、退场；`after` 在同一条路上连改 6 趟。
    // 判据是持树者那一侧的两行读数（`watch dropped who=…` 与 `watchers=` 下降），不是整机那一格
    // ——故两台都**轻**（趟数少、间隔长），别把 `layout.rs` 在案的那族残余放大成红。
    &probe_watch_after::PROBE_WATCH_AFTER,
    &probe_watch_gone::PROBE_WATCH_GONE,
    &probe_lease::PROBE_LEASE,
    &probe_bound::PROBE_BOUND,
    // 共享内存那一具架（`ipc::rack`）：单域那一台量**队列语义与唤醒协议**；
    // 另两台一对，量**跨域共映射**（页与铃当门牌过树，对端用产品同一条客人面取回）。
    &probe_rack::PROBE_RACK,
    &probe_rack_mount::PROBE_RACK_MOUNT,
    &probe_rack_guest::PROBE_RACK_GUEST,
    // 那两族"没有会话"的服务（名册 / 盟册）：各该有 `Grant::ALL.len()` 枚（四族格数各归各家——
    // operator 归 `gate`、control 归 `control`）。次序：那两族起的头 ＋ 树那条路（`operator`）。
    &probe_coalition::PROBE_COALITION,
    &probe_control::PROBE_CONTROL,
    // 操作面那一族（`/svc/sys/operator/{part,land,…}`）：**两位一对**——`gate` 拿控制面会话把七格
    // 验一遍并取回那一枚入口、铺好试验场；`land` 只持 `land` 一位（时序见各自那份声明）。
    &probe_operator_gate::PROBE_OPERATOR_GATE,
    &probe_operator_land::PROBE_OPERATOR_LAND,
    // 压测台与它们的受害者（整台替换引导镜像）。
    &churn::CHURN,
    &rig::RIG,
    &busy::BUSY,
    &park::PARK,
    &hang::HANG,
    &load::LOAD,
    &beat::BEAT,
    &again::AGAIN,
    &waiter::WAITER,
    &group::GROUP,
];

/// 清单条数上界与注册表条数必须相容（见 env::ledger::manifest::MAX_PROGRAMS 的头注）
const _: () = assert!(PROGRAMS.len() <= env::ledger::manifest::MAX_PROGRAMS);
