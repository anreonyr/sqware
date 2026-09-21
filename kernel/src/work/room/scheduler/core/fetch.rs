// 取活（core::fetch）— 本核没活时怎么拿到活：本核队首 + WFI 休眠
// （跨核偷取已删，见下「照实记」）。
//
// `fetch` 是「Idle → Running」的对外入口，顺序**不可重排**：全退出检查须在取活
// **之前**（停机后不得再取活）；`wait` 内部自带全退出复审 + 睡眠位协议，且它睡下
// 之前与醒来之后都要再取一次活（防「检查完 → 置位 → 睡」窗口内的入队漏唤醒）。
//
// # 照实记：跨核偷取已删（用户裁决）
//
// 旧版这里还有一条 `steal`：空闲核扫别人的队列把活搬走。它的判据（task-3）是
// "`starved` 不再需要它"。icount 环境对齐后实测（release，`QEMU_SMP=4`，icount 关，
// 各 5 轮共 1640 次试验）：steal 开 ⇒ `doom: starved` 合计 23、`rig: lost` 合计 1；
// steal 关 ⇒ 18、2——**两格都在噪声内**，而 `steals` 从约 80/轮降到 0。故整条路径、
// 它依赖的 `starved_len` 镜像、`steal_cursor` 与三个读数一起退休。
//
// 注：早先"关掉 steal 也无差别"的那次对照是在 `-icount auto,sleep=on`（默认）下取的，
// 那时瓶颈是 icount 把 WFI 唤醒节流到毫秒，**不作数**；本次是在与验收门一致的环境里取的。

use alloc::sync::Arc;

use riscv::register::{sie, sip};

use crate::hart;
use crate::runtime::chrono::timer;
use crate::work::room::conductor;
use crate::work::room::messenger;
use crate::work::unit::task::Task;

use super::table::current;

/// WFI 休眠的推远增量：无待唤醒 tock 时 arm 到「永远」。
///
/// # 照实记："睡到永远"曾被怀疑有问题，最后是**台子跑错了环境**
///
/// rig A 曾量到"投给一颗睡在 WFI 的核，它 96% 不醒"（`starved` 312~324/328），据此试过
/// 逐核确认重试、整字广播、以及**把正常期上限从"永远"改成 1 ms 的有界兜底拍**。有界拍
/// 收益是真的（`starved` 308~324 → 91~115、`nudged` 1~9 → 144~168），但代价是空闲核从
/// "睡到永远"变成每核每秒约 500~600 拍空转，**被裁决否决**——这一步否对了。
///
/// 真因不在内核：`scripts/boot.nu` 默认带 `-icount auto,sleep=on`（按宿主时间给 vCPU
/// 记账、让它睡够虚拟额度）⇒ **WFI 里的核被 IPI 叫醒要等额度，实测毫秒级**（延迟直方图
/// 众数 1~10 ms）；而验收门一直是关着 icount 跑的。与门对齐（`QEMU_ICOUNT=`）后，同一颗
/// ELF 上 `starved` 落到 **0~7/328**、`nudged` **324~328**。环境对齐见
/// `scripts/{stress,soak,load}.sh`。
///
/// 故本值保持"永远"：空闲核**不该**为空转付费，唤醒侧也不欠这一笔——欠的是"台子要和门
/// 跑在同一个环境里"（照实记，别再走一遍）。
const WFI_FAR: u64 = 1 << 60;

/// 收尾期的 WFI 拍长（**ticks 与 timebase 同频**：QEMU virt 上 10 MHz ⇒ 25 ms）。
/// 25 ms 足以让"停滞 ≥2 s"那条判据拿到足够采样点，又不至于把空闲核变成忙等。
const BEACON_TICK: u64 = 250_000;

/// 取活：本核队首 → 全退出 → 跨核偷 → 睡下等。返回下一帧 PA（永不返回 None：
/// 全退出时走 `conductor::halt`）。
pub(in super::super) fn fetch() -> usize {
    let s = current();
    loop {
        if let Some(task) = s.pull() {
            return s.seat(task);
        }
        if conductor::done() {
            conductor::halt();
        }
        if let Some(task) = wait() {
            return s.seat(task);
        }
    }
}

/// 本 hart 进入 WFI 休眠（Idle 自环的「阻塞点」）。`Some` = 睡醒后有活；
/// `None` = 到期兑现（`redeem`）放行了别人的任务，本核没拿到——交回 `fetch` 复审。
///
/// 协议：置睡眠位 → 复查（防 push 漏唤醒）→ 全退出检查 → 睡到最近 tock → WFI。
/// 唤醒后：有任务 → 正常出口；到期假醒但无活 → 哑睡壳回睡（保持睡眠位、不打点不清位）。
fn wait() -> Option<Arc<Task>> {
    let me = hart::hart_id();
    conductor::sleep(me);
    // 置位后复查：防「检查完 → 置位 → 睡」窗口内的 push 漏唤醒
    let found = current().pull();
    if let Some(task) = found {
        conductor::wake(me);
        return Some(task);
    }
    if conductor::done() {
        conductor::halt();
    }
    loop {
        // 每次决定重新睡下前，先复审全退出：halt 的 yell 会把本核从 WFI 拉起。
        // 若这里不归队 halt，而 redeem 又无可唤醒任务，
        // 就会清 SSIP 后回睡，停机屏障将永远等不到本核的 HALT_ARRIVED。
        if conductor::done() {
            // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位，无并发别名。
            unsafe { sip::clear_ssoft() };
            conductor::wake(me);
            conductor::halt();
        }
        // **停机信标**：必须在**循环内**（每拍一次），不能只在进入 `wait` 时看一次 ——
        // 收尾期的核是"进一次 `wait` 然后一直 WFI"，进去那一刻根任务往往还没析构，
        // 一次性检查会永远错过窗口（实测：把它放在循环外，40 次空闲里 `root_gone()`
        // 恒为 false）。挂住时四个核都睡在 WFI、没有栈帧可读，这一行是唯一能说出
        // "还差谁"的证据；判据是**时间**（收尾毫无进展 ≥2 s），故正常收尾不会误报。
        crate::work::room::scheduler::core::beacon::idle(me);

        // WFI 的拍长：有到点登记就睡到最近那一拍；**否则**——收尾期（根任务已
        // `Reaped`/`Doomed`/已消失）最多睡 `BEACON_TICK`，正常期睡到"永远"。
        //
        // 为什么收尾期非要有界：信标只在**本循环顶部**发声，而 `WFI_FAR` 会让核
        // 一睡不醒 ⇒ 信标永远没机会说话。这正是"挂住时一行证据都没有"的原因
        // （本轮实测：`churn 16 4 4` @32 M 稳定卡死，四个核全睡在 WFI，`[stop]`
        // 一个字都没出）。收尾期本来就没人在跑，多醒几拍不值一提；正常期保持
        // 长睡——**"永远"这条依赖已被证伪，但有界拍的补救被裁决否掉**（见 [`WFI_FAR`]
        // 的照实记：门铃只有约四成送达，补救要在唤醒侧做，不在空闲侧兜）。
        // 空闲核的上限：收尾期 `BEACON_TICK`（信标要在陷阱之外也能说话），正常期"永远"。
        // `beat_until` 再与最近**活**到点取 min ⇒ 与原先"有到点就睡到那一拍、否则睡
        // fallback"逐字等价；只有收尾期且到点比 `BEACON_TICK` 更远时会比原先更早醒一拍
        // （只多几次信标采样——对"挂住时一行证据都没有"是正向，照实记）。
        let fallback = if crate::work::room::scheduler::core::beacon::shutting_down() {
            BEACON_TICK
        } else {
            WFI_FAR
        };
        timer::beat_until(fallback);
        // 外部中断的闸门也在这里重开：「下一拍无条件重开」那条只管**以陷阱形式取到**
        // 的 timer tick，而空闲核的拍子是在这里处理的——SIE=0 的 WFI 只被"挂起"唤醒，
        // 不进陷阱。少了这一句，关过闸门的空闲核就再也不会重开（"自愈"在空闲核上不成立）。
        //
        // **恒开，不看 SEIP**：闸门是**按 hart** 记的（上一枚 `ring` 答 `Busy` 时
        // `trap/mod.rs` 关的是**当时那个 hart**），而应铃的消费者可能在别的 hart 上、
        // 它重开的是**自己**那一格。若本核因 SEIP 挂着就跳过这一句，它的闸门就再没人开。
        // 开了之后 SEIP 一升起 WFI 就立刻返回（SIE=0 ⇒ 只醒、不进陷阱），这正是自愈点。
        // SAFETY: 只置 sie.SEIE 一位，不改任何内存与栈。
        unsafe {
            sie::set_sext();
        }
        // 空闲核不进外部 trap（SIE=0，见上），**这条长驻态上的振铃点只剩这里**：
        // 控制器还挂着就把那枚铃摇响——`Err(Busy)` = 消费者还没应，下一轮再看。
        // **机器活着时**能持 `SEIP` 的核只有这两条长驻态（跑任务 / 空闲，见上），
        // 各有振铃点之后 `devices::raise_irq` 没有"没人可调"的窗口。停机与报警之后
        // 还有别的 `wfi` 长循环（`conductor` / `diagnose::halt`）**不在此列**——那时铃
        // 已经没有消费者，也没人再领 PLIC。
        //
        // **照实记的代价**：SEIP 挂着而消费者尚未 claim 的那一段，本核不再睡
        // （`SEIE=1` ⇒ WFI 立刻返回）。这一段长度由"消费者 claim"决定，不是固定拍；
        // **但它是自旋**（每轮 WFI 立刻返回、循环接着跑），只是有界——消费者的认领会
        // 把 SEIP 落下。旧写法（"挂着就别开"）怕的正是这个自旋，代价的另一半是
        // 关过闸门的空闲核再也开不回来（实测：铃再不响、谁也没醒）。
        if sip::read().sext() {
            let _ = crate::platform::devices::raise_irq();
        }
        // WFI：SSIP（IPI）/ STIP（定时器到期）挂起即唤醒——只唤醒不取中断（SIE=0）。
        // 注意：不再有清退应答点——RFENCE 由固件强制打断空闲核（含 WFI 态），
        // 目标核进 trap 执行 sfence，无需空闲核主动 sweep。
        // IPI 自检钩子（framework 档，见 `runtime::diagnose::ipi`）：全是只读计数，
        // 生产档一行不编。
        #[cfg(feature = "framework")]
        crate::runtime::diagnose::ipi::wfi_entry(me);
        unsafe {
            core::arch::asm!("wfi");
        }
        #[cfg(feature = "framework")]
        crate::runtime::diagnose::ipi::wfi_exit(me, sip::read().ssoft());
        // **待杀记录的兜底也要在这一条路上跑**（修的正是"他杀偶发不生效"那一格）。
        //
        // 定时器到期在空闲核上是**在这里**处理的：WFI 时 SIE=0（见上），到期只把核
        // 从 WFI 里放出来，**不进陷阱** ⇒ `SupervisorTimer` 那一支（trap/mod.rs）里挂的
        // `sweep_doomed` 在本轮**一次也不会跑**。而"已离核、未进容器"那一瞬被点名的
        // 任务恰恰没有 IPI 可投（`running_hart` 答 `None`，见 `doom::doomed_nudge`），
        // 它唯一的兑现点就是扫单位 ⇒ **整机闲着的时候那笔杀令无人兑现**（实测：目标
        // 1.3 s 没收，`until` 两次都答 `Unsettled`；机器一忙起来（下一次陷阱）当场收掉）。
        //
        // 代价：一次 `PENDING` 原子读（`sweep_doomed` 第一行就是它），空表时到此为止。
        crate::work::room::messenger::sweep_doomed();
        // timer 到期分派由 messenger 处理（票根 → 键 → 站点，一路）
        if messenger::redeem() {
            break;
        }
        // 假醒：也可能被 yell 的 IPI 唤来（有活入队）——先复查取活，
        // 有任务即正常出口（睡眠位就在本分支清掉，见下）；真无活才保持睡眠位回睡。
        if let Some(task) = current().pull() {
            conductor::wake(me);
            return Some(task);
        }
        // 哑睡壳（假醒无活）：保持睡眠位、不打点不清位，清残留 SSIP 后回睡。
        // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位，无并发别名。
        unsafe { sip::clear_ssoft() };
    }
    // 正常出口：清 SSIP（防残留位导致下次 WFI 立即重醒）与睡眠位
    // SAFETY: 写本 hart 自己的 sip CSR，仅清 SSIP 位，无并发别名。
    unsafe { sip::clear_ssoft() };
    conductor::wake(me);
    None
}
