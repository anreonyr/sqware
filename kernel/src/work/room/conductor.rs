// 系统级生命周期：跨核共享的原子状态与编排。
//
// 两件事：
//   全退出停机 — PUSHED/REAPED 任务计数 + ROOTED 守门；ROOTED=true 且
//              （PUSHED==0 或 REAPED==PUSHED）= 全部退出 → 发 SBI srst 复位；
//              HALTING 做一次性互斥，防多核同时发复位。**测试模式不复位**：改为
//              按结局账退到 semihosting（判据见 `halt`）。
//   休眠唤醒   — WAITING 位图（bit h = hart h 正 WFI 等待）；入队者先用 `pick`
//              挑一颗核（优先"在等"的），再把活**踢给它**（`scheduler::core::kick`
//              ——唯一的入队路径），落点核若在等就发一记定向 IPI（消雷鸣群）；
//              halt 屏障由 yell 广播喊全员归队。
//
// 命名：动词（push/exit/done/halt/sleep/wake/**pick**/**kick**/**yell**/**nudge**，其中
// `kick` = `scheduler::core::kick`，本文件只记它的读数）+ 计数名词
// （PUSHED/REAPED/WAITING/HALTING/HALT_ARRIVED/KICKS/FALLBACK）。
//
// **照实记**：旧表写的是 `spawn`/`wfi`/`boot_done` 与计数名词 `BOOT_DONE`——这四个名字
// 全树都没有对应符号（`spawn` 在 `boot::spawn_root`，WFI 是 `fetch::wait` 里的一条
// `asm!`），故照实换成本文件与 `scheduler::core` 里真有的那些。

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::hart::{self, HartId};
use crate::lock::OnceLock;
use crate::putln;
use crate::runtime::chrono::{clock, timer};
use crate::runtime::diagnose::ledger;
use sbi::ecall::SArgs;
use sbi::{self, fid};

/// 已入队（创建）任务计数（全退出检测：REAPED == PUSHED → 停机）。
static PUSHED: AtomicUsize = AtomicUsize::new(0);
static REAPED: AtomicUsize = AtomicUsize::new(0);
/// 根服务已产生标记（root 域已 spawn）。一旦置位，`done()` 才允许 true——
/// 防 PUSHED 永久为 0 时被误判"全部结束"。一次性：`boot::init` 装出 root 之后
/// 立即置位（此后所有任务都挂在 root 的 heir 树下，root 退出即 doom 级联）。
static ROOTED: AtomicBool = AtomicBool::new(false);
/// 屏障信标的自旋阈值：取够大以避开正常收尾的抖动（正常一轮屏障只等几个核、
/// 微秒级），又够小以在门的单步超时（15 s）之前把话说出来。
const BARRIER_REPORT_AT: usize = 20_000_000;

/// 停机互斥：第一个触发 srst 的核胜出，其余 wfi（避免双 srst）。
static HALTING: AtomicBool = AtomicBool::new(false);
/// 已到达 halt 的核数 — 关机屏障：胜出核须等**全部**核到达后再断言帧基线。
static HALT_ARRIVED: AtomicUsize = AtomicUsize::new(0);
/// WFI 等待唤醒的 hart 位图（字 w 的 bit b = hart `w·64 + b` 正阻塞）。
/// 多字：字宽 = SBI 单次 `sbi_send_ipi` 的寻址窗口（XLEN = 64 位），按
/// MAX_HART_SLOTS 分字。位位置 = hartid。
static WAITING: [AtomicUsize; WAITING_WORDS] = [const { AtomicUsize::new(0) }; WAITING_WORDS];
/// 全局旋转游标：`pick` 挑核用（自增）。
///
/// 命名（用户裁决）：**pick = 挑一颗核**（选核，只读、不发 IPI）；**kick = 把那枚活
/// 踢给它**（`scheduler::core::kick`，唯一入队路径 + 必要时定向 IPI）。旧名
/// `YELL_CURSOR` 随旧的无参 `kick()` 一起退役——那个函数只叫醒等待核、不搬活，
/// 已经在甲案里被取代（见 `scheduler::core::hart::Scheduler::push` 的头注）。
///
/// 为什么挑核优先"在等"的：被挑中的核是**唯一**能拿到这枚活的核（无参 kick 时代的
/// "源核下次 yield 自取"兜底已经不存在——S 态域任务空转不吃陷阱 ⇒ 源核永不 yield，
/// rig A 实测 `starved=318/328`）。挑一颗正 WFI 的核 = 它一醒来就在自己的队列里看见活。
/// 一个都没有（全忙）时才退到 `游标 % n`，那是**提示性**落点：核下次进 `fetch` 自取。
///
/// Relaxed fetch_add 即可（不必严格跨核同步：每次 pick 拿不同起点即可）。
static PICK_CURSOR: AtomicUsize = AtomicUsize::new(0);

/// WAITING 位图字数：每字 64 位（= 协议单次 IPI 掩码窗口）。
const WAITING_WORDS: usize = crate::layout::MAX_HART_SLOTS / usize::BITS as usize;

/// 任务**产生**计数 +1（PUSHED）。Relaxed 够用：计数只用于相等比较。
///
/// 挂在产生处（`TaskBuilder::hold`）而非入队处——`Held` 线程被父域 kill 时
/// `REAPED` 与 `PUSHED` 仍配平（否则 `done()` 恒假，系统永不停机）。
pub(crate) fn push() {
    PUSHED.fetch_add(1, Ordering::Relaxed);
}

/// 任务回收计数 +1（REAPED）。
///
/// `pub(crate)`：正规路径是 `messenger::bury`（每个躯壳一笔），但**"任务消失"不止
/// 那一条路** —— 外壳被放掉而没走过 `reap` 的形态（未放行的引导线程、被判死后先掉的
/// 壳……）由 `Task::drop` 补账，否则 `done()` 恒假、全机再也停不下来。
pub(crate) fn exit() {
    REAPED.fetch_add(1, Ordering::Relaxed);
}

/// 全部任务是否已退出。
///
/// 守门 `ROOTED == true`（防 boot 早期 PUSHED==0 误判"全部结束"——
/// 当时 PUSHED 尚未增长就被 read，会永久返 false → 无任务场景无法停机）。
///
/// 守门后：`PUSHED == 0`（boot 没装出 root，过期 PUSHED==0 仍可停机）或
/// `REAPED == PUSHED`（全部回收）。
pub(super) fn done() -> bool {
    if !ROOTED.load(Ordering::Acquire) {
        return false;
    }
    let pushed = PUSHED.load(Ordering::Relaxed);
    pushed == 0 || REAPED.load(Ordering::Relaxed) == pushed
}

/// **只读计数**：`(已产生, 已回收)`。信标点名"谁还没归队"时用它 —— 两者不等即为
/// 「还有任务没走完收尾」，相等而屏障不齐即「有核没到 halt」。
pub(crate) fn counts() -> (usize, usize) {
    (
        PUSHED.load(Ordering::Relaxed),
        REAPED.load(Ordering::Relaxed),
    )
}

/// 屏障已到达的核数与应有核数（信标用）。
pub(crate) fn barrier() -> (usize, usize) {
    (
        HALT_ARRIVED.load(Ordering::Acquire),
        crate::hart::hart_count(),
    )
}

/// 标记根服务已产生（一次性）。由 `boot::init` 在装出 root 之后立即置位；
/// 守门 `done()` 必须见位才认 true。
pub(crate) fn rooted() {
    ROOTED.store(true, Ordering::Release);
}

/// 全部任务已退出：显式停机（srst；AtomicBool 防双核同时触发——后到者 wfi）。
///
/// 关机屏障：胜出核等全部核到齐后跑注册关机钩子、最后复位。钩子按注册顺序：
/// 调度器槽清空（`scheduler::rip`）→ block 池冲洗（`block::flush`）。
/// 钩子由 `boot::init` 一次性注册，conductor 不直接命名任何子系统。
pub(super) fn halt() -> ! {
    // 退驻：本核即将卧倒，永不再应答清退——必须先从名册消失，否则关机钩子
    // （`rip` 拆任务 → 拆空间 → shootdown）会死等本核。
    crate::memory::manager::asid::vacate();
    HALT_ARRIVED.fetch_add(1, Ordering::AcqRel);
    if !HALTING.swap(true, Ordering::AcqRel) {
        // 喊醒所有 WFI 睡核：它们醒来后同样会走 done → halt → 登记到达。
        // **必须用广播 `yell`**——屏障要求全员到齐，单点 `kick` 会让部分
        // hart 留 WFI 不归队、屏障永远释放不了。
        yell();
        // **信标**：屏障等不齐时，最后一行日志必须说出"还差几个核"——否则现场只剩
        // "没有 `system halted`"，无从判断是屏障没齐（有核没到 halt）还是根本没走到
        // `done()`（有任务没退完，那种情况下本函数压根不会被调用）。
        // 一次性：只报第一行，之后照旧自旋（诊断不许把停机变成刷屏源）。
        let mut spins = 0usize;
        let mut reported = false;
        while HALT_ARRIVED.load(Ordering::Acquire) < hart::hart_count() {
            spins += 1;
            if !reported && spins == BARRIER_REPORT_AT {
                reported = true;
                let (arrived, total) = barrier();
                let (pushed, reaped) = counts();
                crate::putln!(
                    "[stop] halt 屏障等待：已达 {arrived}/{total} 核；任务 PUSHED={pushed}                      REAPED={reaped}（差 {}）—— 屏障等的是**核**，不是任务",
                    pushed.saturating_sub(reaped)
                );
            }
            core::hint::spin_loop();
        }
        putln!("task: all tasks exited, system halted");
        // 到点兑现的迟到读数（只读、Relaxed）：此处全部核已过 halt 屏障 ⇒ 计数器不再变。
        // 毫秒按 `hertz()` 现算（`clock::ticks_to_duration`），不写死频率。`late_n` 只数
        // **真的等到超时**的到点（被 `mute` 取消的不算），故它随 workload 差三个数量级：
        // debug soak ≈ 2/轮，release soak ≈ 800/轮（`echo`/`guest` 各以 1 ms 轮询睡眠）。
        // 注意 `late` 是**兑现时刻**减登记到点，不是"哪个核武装得晚"——树内 workload 里
        // 总有核在空闲，而空闲核按 `due()` 武装、`redeem` 又是全局的，故这条债在树内
        // 量不出来（见 `wait::block` 里那次武装的注释）。
        {
            let (late_n, late_max, late_sum) = timer::late_stats();
            let max_ms = clock::ticks_to_duration(late_max).as_millis();
            let avg_ms = if late_n == 0 {
                0
            } else {
                clock::ticks_to_duration(late_sum / late_n).as_millis()
            };
            let (tocks, mutes) = timer::tock_stats();
            putln!(
                "timer: late_n={late_n} late_max_ms={max_ms} late_avg_ms={avg_ms} late_max_tick={late_max} traps={} tocks={tocks} mutes={mutes}",
                timer::ticks()
            );
        }
        // 收令落在哪一档（只读）：`held` = 还没放行；`starved` = **就绪却没上台**（"唤醒 ⇒ 上台"
        // 这一段的问题落在这一格）；`blocked` = 还挂在等待点上（唤醒没送到）；`nudged` = 判它在
        // 台上 ⇒ 记 doomed + 定向 IPI。**rig A 要问的"点名落在离核那一瞬"就靠 `nudged`**
        // ——台子的 `now`/`waited` 分不出位置（那是投递与复探的赛跑）。
        {
            let (held, starved, blocked, nudged) = crate::work::room::messenger::branch_stats();
            putln!("doom: held={held} starved={starved} blocked={blocked} nudged={nudged}");
            // 「唤醒 ⇒ 上台」那一段：kick 挑核踢活几次（`kicks` = 落点核在等、发了 IPI；
            // `fallback` = 落点核不在等、活靠它下次自取）。
            let (kicks, fallback) = kick_stats();
            putln!("sched: kicks={kicks} fallback={fallback}");
            // 外部中断那枚铃：**摇了几次 / 其中几次"还响着"**，以及其中的**空闲核补摇**那一支
            // （`scheduler::core::fetch` 的空闲循环）。那一支是"没人可调"窗口的补丁，故这一行
            // 是它的读数：`idle_ring` 非零 ⇒ 这一手真的在走；为零 ⇒ 那一段没发生（也是读数）。
            let (ring, busy, idle_ring, idle_busy) = crate::platform::devices::irq_stats();
            putln!("irq: ring={ring} busy={busy} idle_ring={idle_ring} idle_busy={idle_busy}");
        }
        crate::runtime::diagnose::trace::note(crate::runtime::diagnose::trace::EventKind::Halt(
            crate::runtime::diagnose::trace::HaltEvent::Halt,
        ));
        hooked();
        // 测试模式：**不复位**——世界已结清，结局由账说话（见 `diagnose::ledger`）。
        // 判据**只此一处**，三条：
        //   · **一台域都没起** ⇒ 世界压根没跑起来（镜像里没有可起的域？）——**静默成绿**
        //     是最难查的一类假象，故它算红。看的是**计数**不是账：账只记非零，
        //     正常跑完它是空的；
        //   · 账里有一笔 `EXIT_PANIC` ⇒ 那一台的断言塌了（域内 panic）⇒ 红，并把账上的
        //     笔全打出来（账只装非零，环只有 8 条，多台同时塌时一次看全）；
        //   · 其余 ⇒ 绿。
        if crate::testing() {
            if counts().0 == 0 {
                putln!("[verdict] 一台域都没起：世界没跑起来，这一景什么都没验");
                semihosting::process::exit(101)
            }
            let mut blame: Option<ledger::Entry> = None;
            ledger::each(|e| {
                if blame.is_none() && e.reason == env::EXIT_PANIC {
                    blame = Some(*e);
                }
            });
            let Some(e) = blame else {
                semihosting::process::exit(0)
            };
            ledger::each(|x| {
                putln!(
                    "[verdict] tid={} reason={:#x} note: {}",
                    x.task.get(),
                    x.reason,
                    x.note()
                );
            });
            putln!("[verdict] 塌在 tid={}: {}", e.task.get(), e.note());
            semihosting::process::exit(101)
        }
        let _ = sbi::SystemResetCall::new(fid::SystemReset::SystemReset).call();
    }
    loop {
        unsafe { core::arch::asm!("wfi") };
    }
}

// ── 关机钩子注册面 ──
//
// 子系统在 `boot::init` 把自己的关机函数挂到这里——conductor 不硬编码子系统名。每条
// 钩子调一次，顺序 = 注册顺序（`scheduler::rip` → `block::flush`），由 `boot::init` 装
// 配时定。**mail 不在其列**：它的资源随 `Task` 的 drop 链透传（`PoleMeta::drop` 还物理
// 帧），没有自己的关机钩子（见 `boot::init` 里那一段照实记）。
type Hook = fn();

static HOOKS: OnceLock<&'static [Hook]> = OnceLock::new();

/// 挂上关机钩子（一次性；由 `boot::init` 调用）。
pub(crate) fn hook(hooks: &'static [Hook]) {
    let _ = HOOKS.set(hooks);
}

/// 跑全部挂上的（halt 屏障之后调）。
fn hooked() {
    if let Some(hooks) = HOOKS.get() {
        for h in hooks.iter() {
            h();
        }
    }
}

/// 标记 hart 进入 WFI 等待。调用方须在置位后**复查队列**再睡。
pub(super) fn sleep(hart: HartId) {
    debug_assert!(
        hart.get() < crate::layout::MAX_HART_SLOTS,
        "sleep hart {hart} beyond MAX_HART_SLOTS"
    );
    let (word, bit) = hart.bit();
    WAITING[word].fetch_or(bit, Ordering::AcqRel);
}

/// 清除 hart 的等待标记（WFI 唤醒后 / 复查发现任务时调用）。
pub(super) fn wake(hart: HartId) {
    debug_assert!(
        hart.get() < crate::layout::MAX_HART_SLOTS,
        "wake hart {hart} beyond MAX_HART_SLOTS"
    );
    let (word, bit) = hart.bit();
    WAITING[word].fetch_and(!bit, Ordering::AcqRel);
}

/// hart 此刻是否登记为"正 WFI 等待"（`Acquire` 读；`sleep`/`wake` 用 `AcqRel` 写，
/// 本函数的 Acquire 与那两次配对——`scheduler::core::kick` 据此决定要不要发 IPI）。
///
/// **只是提示**：`false` ⇒ IPI 省掉。活已经进了落点核的队列，它下次进 `fetch` 自取
/// （唤醒不是正确性依赖，见 `scheduler::core::kick`）。
pub(crate) fn waiting(hart: HartId) -> bool {
    debug_assert!(
        hart.get() < crate::layout::MAX_HART_SLOTS,
        "waiting hart {hart} beyond MAX_HART_SLOTS"
    );
    let (word, bit) = hart.bit();
    WAITING[word].load(Ordering::Acquire) & bit != 0
}

/// **选核**（甲案：唤醒不再靠偷）：游标自增，从 `游标 % 64` 起在 WAITING 位图里找
/// 第一个 set bit——命中就挑那颗**正等着的**核（它一醒来就在自己的队列里看见活）。
///
/// 一个都没有（全忙）⇒ 取 `游标 % n` 并**跳过本核**（`n == 1` 时只能是本核）。跳本核
/// 的理由：本核正跑着调用方（S 态域任务空转不吃陷阱 ⇒ 永不 yield），把活放回自己
/// 队列就是 rig A 量到的 `starved=318/328` 那口井。
///
/// 只读、无失败域、不发 IPI：落点由 [`waiting`] 在 `kick` 里补一记定向 IPI。
/// 只写 `PICK_CURSOR`（`Relaxed` RMW），调用方可持任何锁调它——它不碰调度锁。
///
/// 只读、无失败域：位图里任何 set bit 都只可能是已启动的 hart（`boot` 按实际核数
/// 注册、`sleep` 的 debug 断言守着 `MAX_HART_SLOTS` 上界）⇒ 返回值恒是合法 hart 下标。
///
/// # 不再给 `pick` 加"负载信号"（用户裁决，此门关闭）
///
/// 曾议过给选核补一个信号（读各核积压的 `backlog`，或新开一个"最早空出时刻"`free_at`）。
/// 环境对齐后的实测把这条议案的**前提**取消了：`fallback=1`（落点核几乎总在等）、落点核
/// 自己取走活的比例 ~90%、`starved` 落到 0~7/328 ⇒ **入队侧已经健康**，没有需要修的
/// 偏差；而 `backlog` 那面镜像已随 `steal` 一起退休（它的唯一读者就是偷取预检）。
/// 故**不引入** `free_at` / `backlog` 选核信号。
///
/// # 照实记：选核曾经偏心过一次，而"位图里没有空闲核"那个结论是错的
///
/// 第一版实现先取本字 `trailing_zeros` 再算旋转距离，只把"本字最低位"当候选 ⇒ 游标
/// 转不到的位置永远轮不上（实测：位图上 0/1 同时在等时永远挑 0）。那组读数当时被读成
/// "hart 2/3 从没进过 WAITING 位图、空闲核对 `pick` 而言不存在"，**那个结论是错的**
/// ——那时逐核探针（已撤）量到 `pick` 落在 2/3 上 25+16=41 次，而 `fallback=1`：若 2/3
/// 从未在位图里，这 41 次落点只可能来自 `seat % n` 兜底，就该有 ≥41 次 `fallback`。
/// 故 2/3 确实进过位图，偏心在选核自己。
/// （`fetch::wait` 的 WFI 循环**整段保持睡眠位**，一次假醒也不会丢掉登记——那是当时
/// 那条解释的第二个错处。）改法：`bits.rotate_right(sm)` 之后取 `trailing_zeros`。
///
/// # 照实记：`starved≈95%` 曾经是 **QEMU `-icount auto,sleep=on`** 造成的，不是选核
///
/// 甲案之后 rig A 仍量到 `doom: starved` 312~324/328（"就绪却没上台"），当时读成"落点核
/// 是忙核/门铃不灵"，并据此试过三种补救（逐核重试、整字广播、空闲核 1 ms 有界兜底拍）。
/// **全部白费**：真因是台子与验收门跑在两个环境里——`scripts/boot.nu` 默认带
/// `-icount auto,sleep=on`，它按宿主时间给 vCPU 记账并让它睡够虚拟额度，于是 **WFI 里的核
/// 被 IPI 叫醒要等额度（实测毫秒级，延迟直方图众数 1~10 ms）**；而验收门（`examine.nu`）
/// 一直是关着 icount 跑的。
///
/// 与门对齐后（`QEMU_ICOUNT=`）同一颗 ELF：`starved` **312~324 → 0~7/328**、`nudged`
/// 1~9 → **324~328**（受害者真正"在台上被杀"）、落点核自己取走活 **24 → 896~1142**。
/// 5 轮共 1640 次试验：`rig: lost` 1（steal 开）/ 2（steal 关），`starved` 23 / 18。
/// 环境对齐已写进 `scripts/{stress,soak,load}.sh`（显式 `QEMU_ICOUNT=`）。
///
/// 顺带的两条结论：① **"空闲核加有界拍"不该做**（它在补 icount 的账，代价每核每秒
/// ~500-600 拍，已被裁决否决，读数留在 `fetch::WFI_FAR` 的照实记里）；② 跨核 `steal`
/// 依判据（task-3）删除——`steals` 从约 80/轮降到 0，而 `lost`/`starved` 都在噪声内。
pub(crate) fn pick() -> HartId {
    let seat = PICK_CURSOR.fetch_add(1, Ordering::Relaxed);
    let n = hart::hart_count();
    debug_assert!(n > 0, "pick with no hart");
    // 从 seat % 64 起扫位图：找**旋转序里最先碰到**的那个 set bit。
    // **必须先把字旋转再取最低位**（`rotate_right(sm)` + `trailing_zeros`）：先前那版
    // 先取本字 `trailing_zeros` 再算旋转距离，只把"本字最低位"当候选 ⇒ 恒定偏爱低位
    // hart（实测：位图上 0/1 同时在等时永远挑 0，2/3 一次也轮不到——那个落点分布骗过
    // 了整轮定位）。`best_rel` 用 `usize::MAX` 当"还没找到"的哨兵（相对距离恒 < 64）。
    let sm = seat % (usize::BITS as usize);
    let mut best_rel = usize::MAX;
    let mut best_bit = 0usize;
    for (w, word) in WAITING.iter().enumerate() {
        let bits = word.load(Ordering::Acquire);
        if bits == 0 {
            continue;
        }
        let rot = bits.rotate_right(sm as u32);
        let rel = rot.trailing_zeros() as usize;
        let bit = w * (usize::BITS as usize) + (sm + rel) % (usize::BITS as usize);
        if rel < best_rel {
            best_rel = rel;
            best_bit = bit;
        }
    }
    
    if best_rel != usize::MAX {
        HartId::new(best_bit)
    } else {
        // 没有核在等：退到轮转落点。本核要跳过——`seat % n == 我` 时推一格，`(seat + 1) % n`
        // 恒不等于本核（n ≥ 2；n == 1 时只能是本核，推也没处可推）。
        let me = hart::hart_id();
        let mut to = seat % n;
        if n > 1 && to == me.get() {
            to = (to + 1) % n;
        }
        HartId::new(to)
    }
}

// ── 唤醒侧只读计数（停机读出口打）──
//
// 甲案之后，"唤醒 ⇒ 上台"这一段不再有"等源核 yield"那一环：入队者就是 `pick` 挑中的
// 核，`kick` 顺手给它一记 IPI（若它在等）。故 `kicks` 的含义收窄成"**kick 发出的定向
// IPI 次数**"，`fallback` 则是它的反面：落点核**当时不在等**（IPI 省了，活靠它下次
// 进 `fetch` 自取）——`fallback` 高 = `pick` 挑核挑得不准（全忙，或"在等"的核没被选中）。
//
// **照实记**：这里原有 `steals/tries/miss` 三个偷取读数，随 `steal` 一起退休（判据与
// 实测见 `scheduler::core::fetch` 文件头的照实记）。

/// `kick` 发出定向 IPI 的次数（落点核在等，才发）。
static KICKS: AtomicUsize = AtomicUsize::new(0);
/// `kick` 的落点核**当时不在等**（IPI 省掉）的次数。
static FALLBACK: AtomicUsize = AtomicUsize::new(0);

/// `kick` 发出一记定向 IPI（落点核正在等）。`scheduler::core::kick` 调。
pub(crate) fn note_kick_ipi() {
    KICKS.fetch_add(1, Ordering::Relaxed);
}

/// `kick` 的落点核**当时不在等**（IPI 省了，活靠它下次进 `fetch` 自取）。
/// `scheduler::core::kick` 调。
pub(crate) fn note_fallback() {
    FALLBACK.fetch_add(1, Ordering::Relaxed);
}

/// **只读**：`(kick 发出的 IPI 数, 落点核不在等的次数)`——停机读出口打。
pub(crate) fn kick_stats() -> (usize, usize) {
    (
        KICKS.load(Ordering::Relaxed),
        FALLBACK.load(Ordering::Relaxed),
    )
}

/// 广播唤醒所有 WFI 等待 hart（**halt 屏障专用**）。mask = waiting 字保
/// 留 XLEN 位全部 set——保证全员到达 `halt()` 登记屏障。
///
/// 命名：动词（kick = 单点轻踢；yell = 喊全员；语义对仗）。
pub(super) fn yell() {
    for (w, word) in WAITING.iter().enumerate() {
        let waiting = word.load(Ordering::Acquire);
        if waiting == 0 {
            continue;
        }
        let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
            .args(SArgs {
                a0: waiting,
                a1: w * (usize::BITS as usize),
                ..Default::default()
            })
            .call();
    }
}

/// 定向轻踢单个 hart（kill 的 Running 分支）：给指定 hart 发 1-bit SSIP，迫使
/// 它在 trap 里查 `doomed` 集合自退。与 `scheduler::core::kick`（把活**搬**到落点核
/// 的队列、顺带叫醒它）不同——目标 hart 可能在跑任务，SSIP 直接打断它。a0=1<<bit、
/// a1=word·64，与 `kick` 的 IPI 同协议。
pub(super) fn nudge(hart: HartId) {
    let (word, bit) = hart.bit();
    let _ = sbi::IpiCall::new(fid::Ipi::SendIpi)
        .args(SArgs {
            a0: bit,
            a1: word * (usize::BITS as usize),
            ..Default::default()
        })
        .call();
}
