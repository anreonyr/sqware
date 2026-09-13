#![no_std]
#![no_main]
//! root — 根服务域：产生系统里的**所有**域，并作为唯一的根授予源头。
//!
//! 内核 boot 只装它一个（`boot::spawn_root`）；此后 shell / echo / dir 全部由本域
//! 产生，权限也由本域分发。**退出即关机**：本域退出 ⇒ `doom` 级联扑杀全部子域
//! ⇒ 全部任务回收 ⇒ `conductor::done` 自然停机（srst）。无需外部 timeout。
//!
//! 流程：
//!   1. 启动参数 = [清单视图 VA, 清单长度, 配对块 VA, 设备条数]（boot 只读映射的
//!      initrd 区与配对块）；
//!   2. 解析清单（`manifest`）与**设备供给**（配对块），跳过自己；
//!   3. **串行握手**（每个子域一次往返）：
//!      `Build` + `Spawn`(Held) → `dock`（开上行孔并授副本）→ `Hatch` → 收 `Quay`
//!      （子域自建控制孔在父侧的句柄，校验「授与人 = 该子域」）→ 往那条孔 push `Pier`；
//!   4. 客户端的目录能力由 **dir 亲授**：`Refer{who, name}` 转达 → `Referred{token}`
//!      收结果 → `Pier{token}` 配给。**名字也在此预约**——目录的名字空间由本域
//!      播种，子域只能注册预约给它的名字（见 `docs/dispatch.md`）。本域手里没有
//!      任何服务孔（见 `docs/root.md`）。
//!   5. `Join(shell)`：用户会话结束 → 本域退出 → 级联 → 停机。
//!   6. **监护**（本域第二个线程）：等 console 死 → 按预算重发（`docs/root.md` §5.3）——
//!      服务从此有"余生"，而收线那条判据也才有读数。
//!
//! # 设备：本域是**第一个持有者**，也是转授者
//!
//! boot 把设备树扫成门闩、经配对块交到本域手里（`docs/driver.md` §3.3.3）。本域
//! **留源副本、不 Open**；唯一的例外是 UART——console 还没上线时，本域用它自己打印
//! （`say`），console 上线后**用同一枚门闩**授出去（`Accord`），此后本域不再碰设备，
//! 改作普通客户端走会话（§3.3.6）。
//!
//! 设备的名字是 boot 给的原样（设备树节点 basename）：本域不解释设备语义，
//! 只按名字挑出要交出去的那一枚。

extern crate alloc;
// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

use alloc::format;

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;

use env::{Name, PAIR_LEN, Pair, PieToken, TaskId, TeamId};
use protocol::console::Console;
use protocol::dispatch::client::Directory;
use protocol::doom;
use protocol::irq;
use runtime::core::handshake::{self, Pier, Quay, Refer, Referred};
use runtime::core::lock::Lock;
use runtime::env::chrono;
use runtime::env::mail::NolePie;
use runtime::env::mail::{self, AnyPie as _, HolePie, PolePie};
use runtime::env::room::{self, exit, exit_with};
use runtime::env::task as utask;

use programs::uart::Uart;

mod manifest;

/// 要交给 console 服务的那台设备（名字 = boot 给的设备树 basename）。
const CONSOLE_DEVICE: &str = "serial@10000000";

/// 中断线驱动要的三样：PLIC 的寄存器、设备树本体、内核的 `irq` 门闩。
const PLIC_DEVICE: &str = "interrupt-controller@c000000";
const DTB_DEVICE: &str = "devicetree";
const IRQ_CHANNEL: &str = "irq";

/// 服务线程要的两个号：目录句柄（`Refer` 的产物）、本域主线程的 task id。
///
/// 为什么走静态：门闩句柄是 **per-task** 的——同一个域的两个线程也各有各的表，
/// 能给另一个线程的只有"它表里的号"。同域 ⇒ 同一地址空间，故一行静态就是最直的
/// 通道（console 给它的输入线程交门闩走的也是这条：`IRQ_SESSION` / `IRQ_FROM`）。
///
/// 请求孔**不在**这里：它由服务线程自己开——客户端认服务靠 `Reserve(入口门闩).owner`
/// （门闩的**开辟者**），而门闩是 per-task 的 ⇒ **谁读它，就得谁开它**，否则客户端
/// 的回信会配到另一个 task 的表里（本设计踩过这一脚）。
static DOOM_DIR: AtomicUsize = AtomicUsize::new(0);
/// 主人（本域主线程）的 task id——只用来判一道门：**停服只归主人**。
static DOOM_OWNER: AtomicUsize = AtomicUsize::new(0);

/// 本域的输出出口：移交前是**设备**（同一枚 UART 门闩），移交后是**会话**。
///
/// 进静态是为了 [`say`] 能被到处调用（它没有 `self`）。锁序：先问会话、没有再问设备
/// ——两条路互不嵌套（会话走孔、设备走 MMIO），无环。
static OUT: Lock<Out> = Lock::new(Out::None);

/// 输出出口的三态：**没有** → **自己的设备视图** → **控制台会话**。
///
/// 三态而不是"设备 + 可选的会话"：后者的读法会出现"两个出口同时活着"，而本域对
/// 这件事的裁决是**移交即改用会话**（§6③）——状态机把这条裁决写进类型里。
enum Out {
    /// 还没拿到设备（启动参数坏掉之前的一瞬）。
    None,
    /// 自己持设备写（移交前）。
    Device(Uart),
    /// 作普通客户端走会话（移交后）。
    Session(Console),
}

fn say(s: &str) {
    OUT.with(|o| match o {
        Out::None => {}
        Out::Device(u) => u.put(s),
        Out::Session(c) => {
            let _ = c.write(s);
        }
    });
}

/// 从配对块里按名字找一枚设备门闩的 token。
///
/// 块是 boot 写、本域只读的借映区：`count` 条定长记录（`env::wire::Pair`）。
/// 找不到 → 本域没法干活（打印都要靠它），由调用方判死。
fn device_token(block: usize, count: usize, want: &str) -> Option<usize> {
    for i in 0..count {
        // SAFETY: 块是 boot 只读借映进本域的一段（页对齐、`count * PAIR_LEN` 字节），
        // 由启动参数告知；此处逐条只读。
        let record = unsafe {
            core::ptr::read_unaligned((block as *const u8).add(i * PAIR_LEN).cast::<Pair>())
        };
        if record.name().is_some_and(|n| n.as_str() == want) {
            return Some(record.token().get());
        }
    }
    None
}

/// 一步失败的编号（`exit_with` 的既有编号空间：它照旧指"死在启动握手的哪一步"）。
///
/// 为什么这些步骤返回 `Result` 而不是直接 `exit_with`：**重发路径要用同一批步骤**，而那边
/// 每一次尝试失败只是"计一次失败"（监护线程有预算），不是致命错误。主线程的处置见 [`fatal`]。
type Step = usize;

/// 主线程的失败处置：**启动期任何一步失败都是致命的**——配置错了就别装作起来了。
fn fatal<T>(r: Result<T, Step>) -> T {
    match r {
        Ok(v) => v,
        Err(code) => exit_with(code),
    }
}

/// 清单里按名取程序 → `Build` 装域 → `Spawn` 产**未放行**的引导线程（启动参数为空）。
///
/// `build` = 建域权（root 启动时解封一次、此后一直用）：`Build` 的第一道门。
fn build_spawn(
    entries: &[manifest::Entry<'_>],
    name: &str,
    build: &NolePie,
) -> Result<TaskId, Step> {
    let Some(e) = entries.iter().find(|e| e.name == name) else {
        say("root: missing program ");
        say(name);
        say("\n");
        return Err(1);
    };
    let team: TeamId = match utask::build(e.elf, e.kind, e.name, build) {
        Ok(t) => t,
        Err(_) => return Err(2),
    };
    match utask::spawn(team, 0, &[], 0) {
        Ok(t) => Ok(t),
        Err(_) => Err(3),
    }
}

/// 开上行孔（`dock`）→ 放行。返上行孔（root 侧）。
fn launch(child: TaskId) -> Result<HolePie, Step> {
    let up = match handshake::dock(child) {
        Ok(u) => u,
        Err(_) => return Err(4),
    };
    if utask::hatch(child).is_err() {
        return Err(5);
    }
    Ok(up)
}

/// 收报到：自证「这枚控制孔真是该子域授出来的」，返它在 root 侧的句柄。
fn report(child: TaskId, up: &HolePie) -> Result<HolePie, Step> {
    let quay = match Quay::pull(up) {
        Ok(q) => q,
        Err(_) => return Err(6),
    };
    let vestor = match mail::reserve(quay.hole()) {
        Ok((vestor, _owner)) => vestor.get(),
        Err(_) => return Err(7),
    };
    if vestor != child.get() {
        say("root: report not from child\n");
        return Err(8);
    }
    Ok(HolePie::from_token(quay.hole()))
}

/// 转授一枚门闩给子域，并把**对端侧**的句柄经下行孔配给它（配给 = `Pier`）。
///
/// 这是本域对"我手里有一枚、某个子域需要它"的**唯一**出口：转授（`Accord`）+ 配给
/// （`Pier`）。失败即判死——配给没送到，子域会卡在等第一件配给上，症状比死更难看。
fn hand_over(
    down: &HolePie,
    token: usize,
    child: TaskId,
    subset: env::Permission,
) -> Result<(), Step> {
    let at_child = match PolePie::from_token(token).accord(child, subset) {
        Ok(t) => t,
        Err(_) => return Err(19),
    };
    if Pier::new(at_child).push(down).is_err() {
        return Err(20);
    }
    Ok(())
}

/// 转授子集的两个常用面：设备要读写，自描述只读。
fn read_write() -> env::Permission {
    env::Permission::READ | env::Permission::WRITE
}

fn read_only() -> env::Permission {
    env::Permission::READ
}

/// 把一件**事实**交给子域：写进一枚一次性孔，按配给递出（用的是既有原语——
/// `UnsealHole` + `Accord` + `Pier`，没有为它新增任何机制）。
///
/// 目前只有一件这样的事实：**设备的名字**。名字不是资源，是 boot 从设备树里原样搬来的
/// 身份（配对块的 `(名字, token)`）——本域是它的读者，子域要用就得由本域转达，别处
/// 没有第二个来源（`docs/driver.md` §12 甲）。
fn hand_name(down: &HolePie, name: &str, child: TaskId) -> Result<(), Step> {
    let Ok(hole) = HolePie::unseal(env::NAME_LEN) else {
        return Err(40);
    };
    if hole.push(name.as_bytes()).is_err() {
        return Err(41);
    }
    let at_child = match hole.accord(child, read_only()) {
        Ok(t) => t,
        Err(_) => return Err(42),
    };
    if Pier::new(at_child).push(down).is_err() {
        return Err(43);
    }
    Ok(())
}

/// 目录面 + 建域权 + 清单视图：**产生一个服务并把它送上线所需的全部通道**。
///
/// 主线程与监护线程各持一份（门闩是 per-task 的 ⇒ 每一样都得先 `Accord` 到对方表里，
/// 见 [`Kit`]）。它本身不认识具体服务：四个字段正好对应 [`spawn_service`] 的四步。
struct Face {
    /// 清单视图（boot 借映的 initrd 区；`Build` 从它取 ELF 字节）。同域共享内存，给引用即可。
    blob: &'static [u8],
    /// 建域权（`Build` 的第一道门）。
    build: NolePie,
    /// 目录请求门闩（`Connect`；也是"本域有资格用目录"的凭据）。
    dir: PieToken,
    /// 目录控制孔（`Refer::named`：预约服务名）。
    control: HolePie,
    /// 目录上行孔（收 `Referred`）。
    up: HolePie,
}

/// 产生一个服务并把它送上线：`Build` → `Spawn` → `dock` → `Hatch` → 收 `Quay` →
/// 预约名字 → 配给目录门闩。返"子域 + 它在父侧的下行孔"（接着配给设备等）。
///
/// **首次与重发走的是同一个函数**（重发因此不是第二条路径）：两次的差别只有"谁是调用方"
/// ——主线程用自己那三枚，监护线程用它自己那三枚，其余一字不差。
fn spawn_service(face: &Face, name: &str) -> Result<(TaskId, HolePie), Step> {
    let entries = match manifest::parse(face.blob) {
        Some(e) => e,
        None => {
            say("root: malformed manifest\n");
            return Err(10);
        }
    };
    let child = build_spawn(&entries, name, &face.build)?;
    let up = launch(child)?;
    let down = report(child, &up)?;
    let refer = match Name::new(name) {
        Ok(n) => Refer::named(child, n),
        Err(_) => return Err(15),
    };
    if refer.push(&face.control).is_err() {
        return Err(11);
    }
    let referred = match Referred::pull(&face.up) {
        Ok(r) => r,
        Err(_) => return Err(12),
    };
    if referred.token().get() == 0 {
        say("root: directory refused to grant\n");
        return Err(13);
    }
    if Pier::new(referred.token()).push(&down).is_err() {
        return Err(14);
    }
    Ok((child, down))
}

/// console 的两件配给 + 它那台设备的属主。
///
/// **次序是硬要求**：**先把属主写给中断驱动，再交名字**。客户端一拿到名字就会去登记
/// （`Register`），而登记读的正是本域刚写的那一行；两件事走的是同一条 FIFO 队列，故先推的
/// `Refer` 一定先被处理（`docs/driver.md` §12 甲）。
fn wire_console(face: &Face, down: &HolePie, child: TaskId, uart: usize) -> Result<(), Step> {
    hand_over(down, uart, child, read_write())?;
    refer_device(face.dir, child, CONSOLE_DEVICE)?;
    hand_name(down, CONSOLE_DEVICE, child)
}

/// plic 收三件：PLIC 的寄存器、设备树本体、内核的 `irq` 门闩。
/// 它自己不认识"串口"——线号由客户端按名字登记（§3.2.6）。
fn wire_plic(
    down: &HolePie,
    child: TaskId,
    plic: usize,
    dtb: usize,
    irq: usize,
) -> Result<(), Step> {
    hand_over(down, plic, child, read_write())?;
    hand_over(down, dtb, child, read_only())?;
    hand_over(down, irq, child, read_write())
}

// ── 他杀服务（`kill` 的机制在核、政策在这里）─────────────────────────────
//
// 内核那枚 `RoomCall::Doom` 只认**血缘**（谁生的谁能杀，传递）；跨血缘的"该不该"
// 没有内核判据——那本来就是政策的活。而 root 是**全体域的祖先**（boot 只装它），
// 它对任何域的杀都够格 ⇒ 它就是 Linux `kill` 里"够格的那一个"：谁都能请求，够格的
// 那个来执行（`docs/driver.md` §12）。

/// 他杀服务：**本域的第二个线程**，无界等请求孔，按请求办事。
///
/// 为什么是另一个线程（而不是把主线程改成事件循环）：主线程的活是"等 shell 死 ⇒
/// 关机"，那本来就是**阻塞**的形状（`wait_dead`）；把请求处理塞进去就得改成有界轮询，
/// `kill` 也会平白带上一拍延迟。分成两个线程，两边各自"等着"——与 console 的
/// "主线程 + 输入线程"同款。
///
/// **请求孔由本线程自己开**（不是主线程开了再授过来）：客户端认服务靠
/// `Reserve(入口门闩).owner`——门闩的**开辟者**；门闩是 per-task 的，谁读谁就得谁开，
/// 否则客户端的回信孔会配到另一个 task 的表里，回执永远出不来。
extern "C" fn doom_service() -> ! {
    let dir_tok = DOOM_DIR.load(Ordering::Acquire);
    let owner = DOOM_OWNER.load(Ordering::Acquire);
    if dir_tok == 0 || owner == 0 {
        exit_with(33);
    }
    let entry = match HolePie::unseal(doom::REQ_LEN) {
        Ok(h) => h,
        Err(_) => exit_with(34),
    };
    let dir = match Directory::open(HolePie::from_token(dir_tok)) {
        Ok(d) => d,
        Err(_) => exit_with(35),
    };
    // 名字在启动期已预约给本线程（主线程转达的 `Refer`），故注册必成。
    if dir.register(doom::SERVICE, &entry).is_err() {
        exit_with(36);
    }
    let mut msg = [0u8; doom::REQ_LEN];
    loop {
        // 无界等待：请求是**事件**，不是节拍——本线程没有别的活。
        //
        // 等不到（门被封印 / 副本没了）即**收场**：本线程服务的门没了，活也就没了。
        let Ok((n, from)) = entry.pull_from(&mut msg) else {
            exit_with(0);
        };
        match msg[0] {
            // 停服：**只归主人**（本域主线程在收摊时说的那一句）。
            doom::OP_QUIT if from.get() == owner => exit_with(0),
            doom::OP_KILL => {}
            _ => continue,
        }
        let Some(kill) = doom::Kill::decode(&msg[..n]) else {
            continue;
        };
        let ack = [collect(&dir, kill.target.as_str()).byte()];
        // 回信孔是**调用方自带**的（报文里那个号）；它若已消失，这一句静默失败。
        let _ = HolePie::from_token(kill.ack.get()).push(&ack);
    }
}

/// 收**一个名字**：解析 → 内核下令 → 有界等它真的没了。
///
/// 三步的分工就是本设计的全部：内核回答"能不能"（血缘判据），**目录**回答"这个名字
/// 现在是谁"（名字的账在它那儿，本域不存第二份），而 `Ok` 这个回执的含义是"内核确认
/// 它**回收完了**"——不是"收到了"。
///
/// **两个调用者共用它**：他杀服务（用户发来的 `Kill`）与**关机收尾**（`main` 的 4.3）。
/// 两件事本来就是同一件——把"这个名字此刻指向的那个域"收掉，并等到它真的没了；差别只在
/// 前者要把结局回给调用方，后者只想知道"收干净没有"。
///
/// **等它走用的是"探手里这枚副本还在不在"，不是 `Join`**：`Join` 的授权是**任务粒度**的
/// （谁生的谁能等，`docs/driver.md` §8.1.20 实测监护线程等不到主线程生下的那个），而
/// console 可能正是监护线程重发的 ⇒ 关机收尾那条路不能依赖 `Join`。
///
/// "该不该"只剩一层，且不是判据：**够得着就能请求**——入口门闩只亲授给 root 引荐过的
/// 域（`Refer` 的产物），沙箱里的域连不上目录，也就拿不到这扇门的副本。
fn collect(dir: &Directory, target: &str) -> doom::Ack {
    // 名字 → 活实例 → 它的属主 task：目录给的入口门闩副本，`owner` 就是开它的那个域的
    // 主线程（`vestor` 会被转发改写，`owner` 不会）。解析完当场放下这枚副本——它不是
    // 我们的资源。
    let Ok(entry) = dir.connect_token(target) else {
        return doom::Ack::Dead;
    };
    let owner = match mail::reserve(entry) {
        Ok((_, owner)) => owner,
        Err(_) => {
            let _ = mail::release(entry.get());
            return doom::Ack::Dead;
        }
    };
    if owner.get() == 0 {
        let _ = mail::release(entry.get());
        return doom::Ack::Dead;
    }
    match room::doom(owner) {
        Ok(()) => {}
        // `Dead`(-2) = 内核那侧对不上号（已回收 / 从未入册）；其余如实报"不许"。
        Err(e) if e.source.code() == -2 => {
            let _ = mail::release(entry.get());
            return doom::Ack::Dead;
        }
        Err(_) => {
            let _ = mail::release(entry.get());
            return doom::Ack::Denied;
        }
    }
    // 有界等"它真的没了"：探的是**手里这枚副本还在不在**——目标一死，内核的退出钩子
    // 沿派生链把它一起摘掉（"客户端死亡"用的同一条机制）。副本没了才回 `Ok`；
    // 探完还在就如实说"已下令、没等到"（`Slow`），**不假装成功**。
    for _ in 0..doom::GONE_ROUNDS {
        if mail::reserve(entry).is_err() {
            return doom::Ack::Ok;
        }
        let _ = room::sleep(core::time::Duration::from_millis(
            doom::GONE_ROUND_MS as u64,
        ));
    }
    // 副本还在 ⇒ 目标还活着；这枚副本是解析时拿的，用完放下（它不属于我们）。
    let _ = mail::release(entry.get());
    doom::Ack::Slow
}

/// 等目标回收（`Join` 的复探模式：挂起过的那一次只当「醒了一次」）。
fn wait_dead(task: TaskId) {
    loop {
        if utask::join(task, 0).unwrap_or(true) {
            return;
        }
        let _ = utask::join(task, usize::MAX);
    }
}

/// 本域自己的目录请求门闩：**启动期问一次，此后一直用**（`docs/root.md` §5.3）。
///
/// 为什么本域现在要留一枚（此前每次用就连一次，见 §10）：目录的控制面是**一问一答的单槽**
/// ——`Refer` 推给控制孔、`Referred` 从上行孔取；而"重发服务名"要在**运行期**再问一次，
/// 那是监护线程的活。一条单槽问答通道只能有一个长期用户 ⇒ 启动期归主线程，此后**整条让给
/// 监护线程**；主线程留这枚请求门闩当凭据，运行期只走 `Connect`。
fn my_entry(control: &HolePie, up: &HolePie) -> Result<PieToken, Step> {
    let Ok(me) = utask::self_id() else {
        return Err(9);
    };
    if Refer::new(me).push(control).is_err() {
        return Err(11);
    }
    let referred = match Referred::pull(up) {
        Ok(r) => r,
        Err(_) => return Err(12),
    };
    if referred.token().get() == 0 {
        return Err(13);
    }
    Ok(referred.token())
}

/// 按名字连上一个服务：**用本域那枚长期凭据**（见 [`my_entry`]）。
///
/// `Connect` 是请求孔上的一问一答，且**回信通道由发起方自备** ⇒ 多个客户端并发提问互不
/// 干扰。这正是"问答"整条让给监护线程之后，主线程仍能连服务的原因。
fn connect(entry: PieToken, service: &str) -> Option<PieToken> {
    let dir = Directory::open(HolePie::from_token(entry.get())).ok()?;
    dir.connect_token(service).ok()
}

/// 作普通客户端连上控制台服务：**本域也一样走目录**。
///
/// 时序：只在 shell 已退出之后调用——那时 console 一定已注册（shell 用过它），故不需要任何
/// 重试。连不上 → `None`，调用方退回自己那台设备。
fn connect_console(entry: PieToken) -> Option<Console> {
    let door = connect(entry, "console")?;
    Console::open(HolePie::from_token(door.get())).ok()
}

/// 对服务说一句"停服"（同样经目录连上它）。
///
/// 为什么需要这一句：服务线程在本域、是本域的**兄弟线程**，不在血缘里——级联收不到它，
/// 而请求孔又是它自己开的（本域 seal 不动）⇒ 只能由协议说一句话收场。
fn quit_doom(entry: PieToken) {
    let Some(door) = connect(entry, doom::SERVICE) else {
        return;
    };
    let _ = HolePie::from_token(door.get()).push(&[doom::OP_QUIT]);
    let _ = mail::release(door.get());
}

/// 把一台设备的名字交给它的属主：**属主只能由 root 写**（`docs/driver.md` §12 甲）。
///
/// 与 `dispatch` 的 `Refer` 同形：谁能用哪个名字不是运行时判定，而是表里有没有写你的行。
/// 这一句**必须早于**把名字交给客户端——客户端拿到名字就会去登记，而登记读的正是本域刚写
/// 的这一行。驱动的报文队列是 FIFO，故先推的 `Refer` 一定先被处理。
fn refer_device(entry: PieToken, who: TaskId, device: &str) -> Result<(), Step> {
    let Some(door) = connect(entry, irq::SERVICE) else {
        return Err(44);
    };
    let ok = match (irq::Line::at(door), Name::new(device)) {
        (Ok(line), Ok(name)) => matches!(line.refer(&name, who), Ok(irq::Ack::Ok)),
        _ => false,
    };
    // 用完放下：本域**不留**服务孔（这一枚只用一次，留着就是"手里有孔"）。
    let _ = mail::release(door.get());
    if !ok {
        say("root: line authority refused the console device\n");
        return Err(44);
    }
    Ok(())
}

// ── 监护与重发（`docs/root.md` §5.3）────────────────────────────────────
//
// 一个服务的一生：**主线程起第一次，监护线程管余生**。这是分工，不是权宜——主线程的正事是
// "等 shell 死 ⇒ 收场"，监护线程的正事是"等 console 死 ⇒ 重发"；两条命各有一个任务**无界**
// 地等着。root 此前零节拍，为了发现"它死了"引入一圈轮询是浪费（而且监护的延迟会由那圈节拍
// 决定，而不是由"它什么时候死"决定）。

/// 重发预算：**窗口内至多 [`RESTART_MAX`] 次**（"活够一个窗口就重置计数"⇒"偶发崩溃"与
/// "崩溃循环"分得开）。**政策值不是裁决**：改值不动账。
const RESTART_MAX: usize = 3;
const RESTART_WINDOW_MS: u64 = 10_000;

/// 探针的周期（毫秒，见 [`wait_gone`]）与"拿探针"的重试次数 × 间隔。
const WATCH_PROBE_MS: u64 = 20;
const DOOR_RETRY: usize = 20;
const DOOR_RETRY_MS: u64 = 25;

/// 监护线程的行李：目录面（含建域权、清单视图）+ console 那台设备的门闩。
///
/// 每一枚门闩都是**主线程 `Accord` 到它表里**的那一份（门闩是 per-task 的）。同域两线程只能
/// 经共享内存交接，故这个形状与 doom 服务的 `DOOM_DIR`/`DOOM_OWNER` 同款，只是行李更多。
/// 写入在 `Hatch` 之前（`Spawn` 恒产 `Held`）⇒ 线程读到的必然是写好的值。
struct Kit {
    face: Face,
    /// UART 门闩（**本任务表里**的 token）——console 的配给。
    uart: usize,
}

static KIT: Lock<Option<Kit>> = Lock::new(None);

/// 主线程开始收摊（**监护线程据此收场**）：它不在血缘里，级联收不到同域的兄弟线程。
static STOPPING: AtomicBool = AtomicBool::new(false);

/// 现在几点（毫秒，单调钟）：只用来量"这个实例活了多久"。
fn now_ms() -> u64 {
    let (secs, nanos) = chrono::clock().unwrap_or((0, 0));
    secs.saturating_mul(1000).saturating_add(nanos / 1_000_000)
}

/// 监护线程：**等 console 死，按预算重发**。
///
/// 收场有三条路，每条都有界——"一直不能重启"的终局必须是停机，不是挂住：
///   ① 主线程开始收摊（[`STOPPING`]）⇒ 本线程跟着走；
///   ② 预算耗尽 ⇒ 打印一行 + 走（**退回"没有恢复"的世界**：主线程等的是 shell，与本线程
///      无关，故 console 起不来也挡不住收场）；
///   ③ 重发成功 ⇒ 接着等新实例。
///
/// **一条次序依赖**（如实记）：本线程收场会连带走它授出去的副本（UART 门闩、目录那两孔、
/// 名字孔）——那些副本的持有者是 console，而它那时**已经死透**（`wait_dead` 保证），故无事
/// 发生；顺序反了就会把还活着的实例的门闩摘掉。
extern "C" fn watcher() -> ! {
    let Some(kit) = KIT.with(|k| k.take()) else {
        exit_with(50);
    };
    let mut born = now_ms();
    let mut tries = 0usize;
    // 首个实例已经在线上（主线程起的）——先拿一枚指向它的入口副本当**探针**。
    let mut door = wait_door(&kit).unwrap_or(PieToken::new(0));
    loop {
        wait_gone(door);
        if STOPPING.load(Ordering::Acquire) {
            exit_with(0);
        }
        // 活够一个窗口 ⇒ 计数重置（上一次崩溃按"偶发"算）。
        if now_ms().saturating_sub(born) >= RESTART_WINDOW_MS {
            tries = 0;
        }
        tries += 1;
        if tries > RESTART_MAX {
            say("root: console unrecoverable, giving up\n");
            exit_with(51);
        }
        match restart(&kit) {
            Ok(next) => {
                door = next;
                born = now_ms();
                say("root: console restarted\n");
            }
            // 没成：**不假装成功**，也不空转——把"死在哪一步"说出来（与主线程的
            // `exit_with(code)` 同一套编号），再试下一次，直到预算用尽（有界）。
            Err(step) => {
                door = PieToken::new(0);
                say(&format!("root: console restart failed at step {step}\n"));
            }
        }
    }
}

/// 重发一次：产生 + 配给 + 写属主 + **拿一枚指向新实例的探针**。
/// **与首次走的是同两个函数**（[`spawn_service`] 与 [`wire_console`]），差别只有"谁是调用方"。
fn restart(kit: &Kit) -> Result<PieToken, Step> {
    let (child, down) = spawn_service(&kit.face, "console")?;
    wire_console(&kit.face, &down, child, kit.uart)?;
    wait_door(kit).ok_or(60 as Step)
}

/// 拿一枚**指向当前实例**的入口副本（`Connect`）：有界重试——console 注册进目录的时机是
/// 它自己的事（本域刚把名字交给它）。
fn wait_door(kit: &Kit) -> Option<PieToken> {
    for _ in 0..DOOR_RETRY {
        if let Some(t) = connect(kit.face.dir, "console") {
            return Some(t);
        }
        let _ = room::sleep(Duration::from_millis(DOOR_RETRY_MS));
    }
    None
}

/// 等它没了：**探能力链**——本线程手里那枚指向它的入口副本还在不在。
///
/// 为什么不是 `Join`（本该是最贴的形状）：`Join` 的授权是**任务粒度**的"它是不是我生的"
/// （`envcall.rs`：`me.heir(target.team.id)`），而监护线程**不是** console 的生父——生它的是
/// 主线程 ⇒ `Denied`（实测）。故改用与 root 的 `doom` 服务同一条机制：**副本没了 = 它走了**
/// （`gate::doom` 的 BFS 沿派生链把本域手里那一枚一起摘掉，§12 ②）。
///
/// **代价如实记**：这是一圈**有界间隔的探测**（[`WATCH_PROBE_MS`]），不是纯事件等待。
/// 要换成 `Join` 只有一条路——**让监护线程当生父**（从第一次起全包），那要另加一条
/// "console 上线"的报到通道（两条路的取舍记在 `docs/root.md` §5.3）。
fn wait_gone(door: PieToken) {
    loop {
        if STOPPING.load(Ordering::Acquire) {
            return;
        }
        if mail::reserve(door).is_err() {
            return;
        }
        let _ = room::sleep(Duration::from_millis(WATCH_PROBE_MS));
    }
}

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 借映视图：清单（initrd 区）+ 配对块（设备供给）。长度与条数经启动参数告知。
    let args = utask::args();
    let (view, len, pairs, pairs_n) = match args {
        [va, len, pairs, n, ..] => (*va as *const u8, *len, *pairs, *n),
        _ => {
            say("root: no boot args\n");
            exit_with(9);
        }
    };
    // SAFETY: boot 把 initrd 区只读映射到该 VA，长度即 region.size。
    let blob = unsafe { core::slice::from_raw_parts(view, len) };
    let entries = match manifest::parse(blob) {
        Some(e) => e,
        None => {
            say("root: malformed manifest\n");
            exit_with(10);
        }
    };

    // 1.2 先接设备：**打印也要靠它**。boot 已经把这枚门闩放进本域的表里，
    //     本域只是开闩（映射）——不是第二个来源。
    let Some(uart_token) = device_token(pairs, pairs_n, CONSOLE_DEVICE) else {
        exit_with(17);
    };
    let uart = match Uart::open(PolePie::from_token(uart_token)) {
        Ok(u) => u,
        Err(_) => exit_with(18),
    };
    OUT.with(|o| *o = Out::Device(uart));

    // 1.3 中断面要的那几枚（本域只经手，不开闩、不看内容）。
    let Some(plic_token) = device_token(pairs, pairs_n, PLIC_DEVICE) else {
        exit_with(21);
    };
    let Some(dtb_token) = device_token(pairs, pairs_n, DTB_DEVICE) else {
        exit_with(22);
    };
    let Some(irq_token) = device_token(pairs, pairs_n, IRQ_CHANNEL) else {
        exit_with(23);
    };

    // 1.5 建域权：**解封一枚 Nole**，此后每次 `Build` 都带它。
    //
    // 为什么由 root 自己解封（而不是 boot 铸好塞进它表里）：`UnsealNole` 有 S 态门，
    // 故"谁能铸"已经由政策划界；而"铸出来的这枚能转授给谁、怎么收回"由能力代数
    // 回答（accord/narrow/revoke）。boot 铸那一条要多一个"往还不存在的任务里塞 pie"
    // 的时序 hack，收益为零。
    let build_right = match NolePie::unseal() {
        Ok(b) => b,
        Err(_) => exit_with(16),
    };

    // 2. dir：先建（客户端要它的门闩）。它的控制孔即后续引入请求的通道。
    let dir_task = fatal(build_spawn(&entries, "dir", &build_right));
    let up_dir = fatal(launch(dir_task));
    let control_dir = fatal(report(dir_task, &up_dir));

    // 1.7 本域自己的目录请求门闩：**问一次、此后一直用**（见 [`my_entry`]）。
    let my_dir = fatal(my_entry(&control_dir, &up_dir));

    // 1.8 目录面：产生并上线一个服务所需的四样（清单视图 + 建域权 + 那两孔 + 请求门闩）。
    let face = Face {
        blob,
        // 同一个句柄再造一份（token 是**本任务表里**的号，句柄可以再取）：主线程与监护线程
        // 各要一份建域权，后者那份由下面的 `Accord` 真正复制过去。
        build: NolePie::from_token(build_right.token()),
        dir: my_dir,
        control: control_dir,
        up: up_dir,
    };

    // 3. plic / echo / console / shell：请 dir 亲授目录请求门闩的 R|W 副本，再配给
    //    客户端；同时把名字预约给该子域（目录只接受预约者的注册）。
    //    次序 = 依赖序：plic 是中断面（console 要连它），console 是交互面（shell 要
    //    连它），故 plic 最先、console 次之、shell 最后。
    let mut shell_task = TaskId(0);
    for name in ["plic", "echo", "console", "shell"] {
        let (child, down) = fatal(spawn_service(&face, name));
        // 各服务自己的那几件配给——**重发走的是同一对函数**。
        match name {
            "console" => fatal(wire_console(&face, &down, child, uart_token)),
            "plic" => fatal(wire_plic(&down, child, plic_token, dtb_token, irq_token)),
            _ => {}
        }
        if name == "shell" {
            shell_task = child;
        }
    }

    // 3.5 他杀服务：**本域第二个线程**（内核那枚 `Doom` 只认血缘，本域是全体域的祖先，
    //     跨血缘的"该不该"因此落在本域）。
    //
    //     "谁能请求"不设判据表：**能连上它就是有资格**——目录门闩只亲授给 root 引荐过
    //     的域，客户端的回信孔又得先拿到入口门闩的副本（`Connect`）才配得出去。
    //     独占不靠判据，靠没有第二个创建入口（沙箱里的域连不上目录，也就够不着这里）。
    let svc = match utask::spawn(TeamId(0), doom_service as *const () as usize, &[], 0) {
        Ok(t) => t,
        Err(_) => exit_with(26),
    };
    // 名字预约给**服务线程**（目录只接受预约者的注册）。`control_dir` 是 dir 的控制孔。
    let refer = match Name::new(doom::SERVICE) {
        Ok(n) => Refer::named(svc, n),
        Err(_) => exit_with(27),
    };
    if refer.push(&face.control).is_err() {
        exit_with(28);
    }
    let referred = match Referred::pull(&face.up) {
        Ok(r) => r,
        Err(_) => exit_with(29),
    };
    if referred.token().get() == 0 {
        exit_with(30);
    }
    let me = match utask::self_id() {
        Ok(t) => t,
        Err(_) => exit_with(31),
    };
    DOOM_DIR.store(referred.token().get(), Ordering::Relaxed);
    DOOM_OWNER.store(me.get(), Ordering::Relaxed);
    if utask::hatch(svc).is_err() {
        exit_with(32);
    }

    // 3.6 监护线程：**等 console 死，按预算重发**（§5.3）。行李逐件 `Accord` 到它表里
    //     （门闩是 per-task 的），再写静态、最后 `Hatch`（`Spawn` 恒产 `Held`）。
    let watch_task = match utask::spawn(TeamId(0), watcher as *const () as usize, &[], 0) {
        Ok(t) => t,
        Err(_) => exit_with(45),
    };
    // 它自己的目录请求门闩：请 dir **亲授给它**——`Refer` 的产物是"对方表里的号"，主线程
    // 手里那枚跨任务不通用。
    if Refer::new(watch_task).push(&face.control).is_err() {
        exit_with(46);
    }
    let w_dir = match Referred::pull(&face.up) {
        Ok(r) if r.token().get() != 0 => r.token(),
        _ => exit_with(47),
    };
    // 控制孔与上行孔：**问答那一条链整条交给它**（主线程此后只走 `Connect`，见 [`my_entry`]）。
    let w_control = match face.control.accord(watch_task, read_write()) {
        Ok(t) => t,
        Err(_) => exit_with(48),
    };
    let w_up = match face.up.accord(watch_task, read_write()) {
        Ok(t) => t,
        Err(_) => exit_with(49),
    };
    // UART 门闩与建域权：**带 `VEST`、不带 `BACK`**。
    //   - `VEST`：它产生新实例时要**再授一次**给那个子域（门闩是 per-task 的）；
    //   - **不能带 `BACK`**：`BACK` 是"回授目标自由"的那一位——**带它的源只能授给 sire 的
    //     持有者**（`gate::snap::vestable`），而监护线程要授给的正是它刚生的子域 ⇒ 那一授
    //     会被 `Denied`（实测：重发卡在 `hand_over` 的 `Err(19)`，三次都用完预算）。
    let vest_only = env::Permission::READ | env::Permission::WRITE | env::Permission::VEST;
    let w_uart = match PolePie::from_token(uart_token).accord(watch_task, vest_only) {
        Ok(t) => t,
        Err(_) => exit_with(52),
    };
    let w_build = match build_right.accord(watch_task, vest_only) {
        Ok(t) => t,
        Err(_) => exit_with(53),
    };
    // **委托写权**（`docs/root.md` §5.3）：重发出来的 console 是个**新 task id**，行表的属主
    // 得跟着换——而驱动认的是"推者是不是我的 `sire`"，监护线程答不上这一条（它不是域）。
    // 故由本线程（`sire` 本人）亲口把**这一个名字**的写权委托给它。这一句必须在 `Hatch`
    // 之前说：它一起来就可能重发，而重发要写属主。
    let Some(door) = connect(face.dir, irq::SERVICE) else {
        exit_with(55);
    };
    let delegated = match (irq::Line::at(door), Name::new(CONSOLE_DEVICE)) {
        (Ok(line), Ok(name)) => matches!(line.delegate(&name, watch_task), Ok(irq::Ack::Ok)),
        _ => false,
    };
    let _ = mail::release(door.get());
    if !delegated {
        say("root: line authority refused delegation\n");
        exit_with(56);
    }
    KIT.with(|k| {
        *k = Some(Kit {
            face: Face {
                blob,
                build: NolePie::from_token(w_build),
                dir: w_dir,
                control: HolePie::from_token(w_control.get()),
                up: HolePie::from_token(w_up.get()),
            },
            uart: w_uart.get(),
        })
    });
    if utask::hatch(watch_task).is_err() {
        exit_with(54);
    }

    // 4. 用户会话结束（shell 退出或崩溃）→ 本域退出 → 级联 → 全部回收 → 停机。
    //    最后这句**经控制台服务**说：本域此时是普通客户端（§3.3.6）。
    wait_dead(shell_task);
    // 4.1 服务线程要**显式收场**：它在本域、是本域的第二个线程，**不在血缘里**——
    //     级联（父死子随）收的是子**域**，收不到同域的兄弟线程；而请求孔是它自己开的，
    //     本域也 seal 不动。故按协议说一句 `Quit`（服务只在收到主人这一句时收场）。
    //     次序在 `say` 之前：让"session over"仍是最后一行。
    quit_doom(my_dir);
    wait_dead(svc);
    if let Some(console) = connect_console(my_dir) {
        OUT.with(|o| *o = Out::Session(console));
    }
    // 4.2 告诉监护线程"本域开始收摊"：它不在血缘里（同域的兄弟线程不被级联收走），而它等的
    //     "console 之死"在本域退出后会由级联带来 ⇒ 它据此收场，而不是把它当成一次崩溃去重发。
    STOPPING.store(true, Ordering::Release);
    say("root: session over, shutting down\n");

    // 4.3 **先收子域，再放本域自己的空间**——次序是判据，不是顺手。
    //
    // 为什么不能只靠 `exit()` 的级联：级联管的是"父死子随"，而本域退场时**自己那几件资源
    // 也要收回**（UART 那页设备内存是本域的所有物，`hand_over` 只把副本交给 console）。这两
    // 件事不是一件原子事（4 核）⇒ 子域可能落在"映射已收、任务还没死"的缝里再读一次设备。
    // 实测那道缝：`no map for user page fault: Load at VA(0x23001)`——VA 正是 UART 那页、读的
    // 是 `IER`（符号化 = `programs/src/uart.rs:114 ← :104`，console 输入线程"进门关中断"那一
    // 步）；对照见 `docs/root.md` §7.6（本轮配置 4 轮里 3 轮出现，干净 master 的 4 轮 0 轮）。
    //
    // 收法与用户杀服务**共用 [`collect`]**（两件事是同一件），且**按名字收、不按 id**：
    // console 可能被重发过（重发出来的是**另一个** id），而"哪个是当前实例"这条账在**目录**
    // 那儿——本域不存第二份。等它走也走 `collect` 里那条判据（探手里那枚副本），不走 `Join`：
    // `Join` 是任务粒度的，重发出来的那个本线程等不到（§8.1.20）。
    //
    // 目录自己**没有名字**（它就是名字的账）：按 id 收，它是本域亲生的 ⇒ `Join` 等得到。
    match Directory::open(HolePie::from_token(my_dir)) {
        Ok(dir) => {
            for name in ["console", irq::SERVICE, "echo"] {
                match collect(&dir, name) {
                    // `Ok` = 收干净了；`Dead` = 它本来就已经没了（`kill` 那两步收过的就是它）
                    // ——两种都是这一步要的结果。
                    doom::Ack::Ok | doom::Ack::Dead => {}
                    // `Slow` = 已下令、没等到：**不假装收干净了**，说出来。
                    doom::Ack::Slow => say(&format!("root: {name} still alive after doom\n")),
                    // `Denied` = 不在血缘里（本域是全体域的祖先 ⇒ 结构不对）：也说一句。
                    doom::Ack::Denied => say(&format!("root: {name} not in lineage\n")),
                }
            }
        }
        Err(_) => say("root: directory unreachable, children not collected\n"),
    }
    if room::doom(dir_task).is_ok() {
        wait_dead(dir_task);
    }
    exit()
}
