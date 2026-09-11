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

use core::sync::atomic::{AtomicUsize, Ordering};

use env::{Name, PAIR_LEN, Pair, TaskId, TeamId};
use protocol::console::Console;
use protocol::dispatch::client::Directory;
use protocol::doom;
use runtime::core::handshake::{self, Pier, Quay, Refer, Referred};
use runtime::core::lock::Lock;
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

/// 清单里按名取程序 → `Build` 装域 → `Spawn` 产**未放行**的引导线程（启动参数为空）。
///
/// `build` = 建域权（root 启动时解封一次、此后一直用）：`Build` 的第一道门。
fn build_spawn(entries: &[manifest::Entry<'_>], name: &str, build: &NolePie) -> TaskId {
    let Some(e) = entries.iter().find(|e| e.name == name) else {
        say("root: missing program ");
        say(name);
        say("\n");
        exit_with(1);
    };
    let team: TeamId = match utask::build(e.elf, e.kind, e.name, build) {
        Ok(t) => t,
        Err(_) => exit_with(2),
    };
    match utask::spawn(team, 0, &[], 0) {
        Ok(t) => t,
        Err(_) => exit_with(3),
    }
}

/// 开上行孔（`dock`）→ 放行。返上行孔（root 侧）。
fn launch(child: TaskId) -> HolePie {
    let up = match handshake::dock(child) {
        Ok(u) => u,
        Err(_) => exit_with(4),
    };
    if utask::hatch(child).is_err() {
        exit_with(5);
    }
    up
}

/// 收报到：自证「这枚控制孔真是该子域授出来的」，返它在 root 侧的句柄。
fn report(child: TaskId, up: &HolePie) -> HolePie {
    let quay = match Quay::pull(up) {
        Ok(q) => q,
        Err(_) => exit_with(6),
    };
    let vestor = match mail::reserve(quay.hole()) {
        Ok((vestor, _owner)) => vestor.get(),
        Err(_) => exit_with(7),
    };
    if vestor != child.get() {
        say("root: report not from child\n");
        exit_with(8);
    }
    HolePie::from_token(quay.hole())
}

/// 转授一枚门闩给子域，并把**对端侧**的句柄经下行孔配给它（配给 = `Pier`）。
///
/// 这是本域对"我手里有一枚、某个子域需要它"的**唯一**出口：转授（`Accord`）+ 配给
/// （`Pier`）。失败即判死——配给没送到，子域会卡在等第一件配给上，症状比死更难看。
fn hand_over(down: &HolePie, token: usize, child: TaskId, subset: env::Permission) {
    let at_child = match PolePie::from_token(token).accord(child, subset) {
        Ok(t) => t,
        Err(_) => exit_with(19),
    };
    if Pier::new(at_child).push(down).is_err() {
        exit_with(20);
    }
}

/// 转授子集的两个常用面：设备要读写，自描述只读。
fn read_write() -> env::Permission {
    env::Permission::READ | env::Permission::WRITE
}

fn read_only() -> env::Permission {
    env::Permission::READ
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
        let ack = [serve(&dir, &kill).byte()];
        // 回信孔是**调用方自带**的（报文里那个号）；它若已消失，这一句静默失败。
        let _ = HolePie::from_token(kill.ack.get()).push(&ack);
    }
}

/// 一次请求：**解析名字 → 内核下令 → 有界等回收**。
///
/// 三步的分工就是本设计的全部：内核回答"能不能"（血缘判据），**目录**回答"这个名字
/// 现在是谁"（名字的账在它那儿，本域不存第二份），而 `Ok` 这个回执的含义是"内核确认
/// 它**回收完了**"——不是"收到了"。
///
/// "该不该"只剩一层，且不是判据：**够得着就能请求**——入口门闩只亲授给 root 引荐过的
/// 域（`Refer` 的产物），沙箱里的域连不上目录，也就拿不到这扇门的副本。
fn serve(dir: &Directory, kill: &doom::Kill) -> doom::Ack {
    // 名字 → 活实例 → 它的属主 task：目录给的入口门闩副本，`owner` 就是开它的那个域的
    // 主线程（`vestor` 会被转发改写，`owner` 不会）。解析完当场放下这枚副本——它不是
    // 我们的资源。
    let Ok(entry) = dir.connect_token(kill.target.as_str()) else {
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

/// 作普通客户端连上控制台服务：**本域也一样走目录**（`Refer` 引荐自己 → dir 亲授
/// 请求门闩 → `connect_token("console")`）。
///
/// 时序：只在 shell 已退出之后调用——那时 console 一定已注册（shell 用过它），
/// 故不需要任何重试。连不上 → `None`，调用方退回自己那台设备。
fn connect_console(control: &HolePie, up: &HolePie) -> Option<Console> {
    let me = utask::self_id().ok()?;
    Refer::new(me).push(control).ok()?;
    let referred = Referred::pull(up).ok()?;
    if referred.token().get() == 0 {
        return None;
    }
    let dir = Directory::open(HolePie::from_token(referred.token())).ok()?;
    let entry = dir.connect_token("console").ok()?;
    Console::open(HolePie::from_token(entry)).ok()
}

/// 对服务说一句"停服"：**本域也走目录**（`Refer` 引荐自己 → dir 亲授请求门闩 →
/// `connect_token("doom")`），与 [`connect_console`] 同一条路。
///
/// 为什么需要这一句：服务线程在本域、是本域的**兄弟线程**，不在血缘里——级联收不到它，
/// 而请求孔又是它自己开的（本域 seal 不动）⇒ 只能由协议说一句话收场。
fn quit_doom(control: &HolePie, up: &HolePie) {
    let Ok(me) = utask::self_id() else { return };
    if Refer::new(me).push(control).is_err() {
        return;
    }
    let Ok(referred) = Referred::pull(up) else {
        return;
    };
    let Ok(dir) = Directory::open(HolePie::from_token(referred.token())) else {
        return;
    };
    let Ok(door) = dir.connect_token(doom::SERVICE) else {
        return;
    };
    let _ = HolePie::from_token(door).push(&[doom::OP_QUIT]);
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
    let dir_task = build_spawn(&entries, "dir", &build_right);
    let up_dir = launch(dir_task);
    let control_dir = report(dir_task, &up_dir);

    // 3. plic / echo / console / shell：请 dir 亲授目录请求门闩的 R|W 副本，再配给
    //    客户端；同时把名字预约给该子域（目录只接受预约者的注册）。
    //    次序 = 依赖序：plic 是中断面（console 要连它），console 是交互面（shell 要
    //    连它），故 plic 最先、console 次之、shell 最后。
    let mut shell_task = TaskId(0);
    for name in ["plic", "echo", "console", "shell"] {
        let child = build_spawn(&entries, name, &build_right);
        let up = launch(child);
        let down = report(child, &up);
        let refer = match Name::new(name) {
            Ok(n) => Refer::named(child, n),
            Err(_) => exit_with(15),
        };
        if refer.push(&control_dir).is_err() {
            exit_with(11);
        }
        let referred = match Referred::pull(&up_dir) {
            Ok(r) => r,
            Err(_) => exit_with(12),
        };
        if referred.token().get() == 0 {
            say("root: directory refused to grant\n");
            exit_with(13);
        }
        if Pier::new(referred.token()).push(&down).is_err() {
            exit_with(14);
        }
        // console 多收一件：**设备门闩**（本域手里那枚的 R|W 副本）。交出去之后
        // 本域不再碰设备——`say` 在会话建立后走会话。
        if name == "console" {
            hand_over(&down, uart_token, child, read_write());
        }
        // plic 收三件：PLIC 的寄存器、设备树本体、内核的 `irq` 门闩。
        // 它自己不认识"串口"——线号是客户端来登记的（§3.2.6）。
        if name == "plic" {
            hand_over(&down, plic_token, child, read_write());
            hand_over(&down, dtb_token, child, read_only());
            hand_over(&down, irq_token, child, read_write());
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
    let svc = match utask::spawn(TeamId(0), doom_service as usize, &[], 0) {
        Ok(t) => t,
        Err(_) => exit_with(26),
    };
    // 名字预约给**服务线程**（目录只接受预约者的注册）。`control_dir` 是 dir 的控制孔。
    let refer = match Name::new(doom::SERVICE) {
        Ok(n) => Refer::named(svc, n),
        Err(_) => exit_with(27),
    };
    if refer.push(&control_dir).is_err() {
        exit_with(28);
    }
    let referred = match Referred::pull(&up_dir) {
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

    // 4. 用户会话结束（shell 退出或崩溃）→ 本域退出 → 级联 → 全部回收 → 停机。
    //    最后这句**经控制台服务**说：本域此时是普通客户端（§3.3.6）。
    wait_dead(shell_task);
    // 4.1 服务线程要**显式收场**：它在本域、是本域的第二个线程，**不在血缘里**——
    //     级联（父死子随）收的是子**域**，收不到同域的兄弟线程；而请求孔是它自己开的，
    //     本域也 seal 不动。故按协议说一句 `Quit`（服务只在收到主人这一句时收场）。
    //     次序在 `say` 之前：让"session over"仍是最后一行。
    quit_doom(&control_dir, &up_dir);
    wait_dead(svc);
    if let Some(console) = connect_console(&control_dir, &up_dir) {
        OUT.with(|o| *o = Out::Session(console));
    }
    say("root: session over, shutting down\n");
    exit()
}
