//! 那一份份声明（#[path] 拉进来的）与 PROGRAMS 这张单。

use super::{SCENE_UNIT, UnitFile};

// **它们不在这份文件的自然模块树里**：那些目录（`driver/uart/`、`service/hub/`…）都拖着
// runtime / protocol 代码，`crates/image` 进不去。故只由 PROGRAMS 这一处按 `#[path]` 拉
// 进来一次——**唯一的声明点**。

#[path = "../harness/probe/accept/program.rs"]
pub mod accept;
#[path = "../harness/bench/again/again/program.rs"]
pub mod again;
#[path = "../harness/bench/beat/program.rs"]
pub mod beat;
#[path = "../harness/bench/load/busy/program.rs"]
pub mod busy;
#[path = "../user/cat/program.rs"]
pub mod cat;
#[path = "../harness/bench/again/churn/program.rs"]
pub mod churn;
#[path = "../harness/bench/group/group/program.rs"]
pub mod group;
/// 测具那 29 台（**探针 / 试客 / 压测台**，身子在 `../harness/`）：其中 15 台由编排域起
/// 而"哪几台进哪张镜像"这张表在宿主侧（`crates/image`）也要看得见 ⇒ 声明与身子同住、由本表拉进来
#[path = "../harness/guest/guest/program.rs"]
pub mod guest;
#[path = "../harness/bench/rig/hang/program.rs"]
pub mod hang;
#[path = "../service/hub/program.rs"]
pub mod hub;
#[path = "../harness/bench/load/load/program.rs"]
pub mod load;
#[path = "../harness/guest/lodger/program.rs"]
pub mod lodger;
#[path = "../user/login/program.rs"]
pub mod login;
#[path = "../harness/guest/member/program.rs"]
pub mod member;
#[path = "../harness/bench/load/park/program.rs"]
pub mod park;
#[path = "../harness/guest/passer/program.rs"]
pub mod passer;
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
#[path = "../driver/router/program.rs"]
pub mod router;
#[path = "../driver/rtc/program.rs"]
pub mod rtc;
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

/// **装配表**：镜像里可能有的全部程序。**次序是硬事实**——它同时是**装载次序**与打包时的条目
/// 次序（`crates/image` 按这张表的位次把镜像挨个写进清单），且各景按 UnitFile::wanted_by
/// 过滤 ⇒ 加一台要想清楚放哪
/// **本表一行一台，`rustfmt` 请绕开**：默认那套会把每台摊成十几行，于是"哪几台进哪张镜像"
/// 就没法一眼扫完——而这张表**就是**给人扫的
#[rustfmt::skip]
pub const PROGRAMS: &[&UnitFile] = &[
    // Device service.
    &hub::PROGRAM,
    &terminal::PROGRAM,
    &login::PROGRAM,
    &cat::PROGRAM,
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
    &SCENE_UNIT,
];

/// 清单条数上界与注册表条数必须相容（见 env::ledger::manifest::MAX_PROGRAMS 的头注）
const _: () = assert!(PROGRAMS.len() <= env::ledger::manifest::MAX_PROGRAMS);
