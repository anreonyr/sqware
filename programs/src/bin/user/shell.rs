#![no_std]
#![no_main]

//! shell — 命令解释器，经 **console 服务**显示、读命令并分发给系统能力。
//!
//! 分层：本 bin 是 Shell；`protocol::console` 的线对侧是本模块的 `Terminal` 适配器，
//! 终端渲染与行编辑住在 `prog-console` 域。**Terminal 是唯一 console 出口**——
//! Shell 的一切输出经 `Terminal::writeline`、一切输入经 `Terminal::readline`。
//!
//! **任务侧没有设备可直连**（`docs/driver.md` §10 第三步）：`IOCall` 已删，故
//! `Terminal` 之外再无第二条输出通路，连不上服务就是连不上——见 [`Terminal`]。
//!
//! 命令（系统能力巡演）：
//!   help  — 列命令
//!   clock — 读全局时钟（ChronoCall::Clock）
//!   ticks — 读 timebase 刻度
//!   alloc — 堆分配一页（MemoryCall::Allocate）
//!   echo  — 回显参数
//!   sleep — 阻塞 N 毫秒（RoomCall::Park）
//!   spawn — 派一个算 0..N 的闭包子任务并 join（UnitCall::Spawn + Join）
//!   heir  — 子域枚举（UnitCall::HeirCount + Heir）
//!   hole  — Hole 通道自测（unseal/push/pull/seal）
//!   cascade — 派生级联自检（三跳撤销 / 无关分支 / release 级联 / 任务消亡级联）
//!   churn — 任务生灭压测（主动探测：反复产生/回收，把关机终值变成刻度）
//!   reclaim — 资源寿命自检（引用回收 / 封印归属 / 开辟者消亡）
//!   spoof — 身份伪造自检（发送者由内核盖章，报文里的回信 token 不构成身份）
//!   name  — 名字权限自检（目录的名字空间由父域预约，注册只能填预约行）
//!   badslot — 非法 envcall 槽位自检（未知调用号 → 拒掉并续跑，绝不 panic）
//!   stray — 野 id 自检（从未入册的 task id 去 Join ⇒ 必须 Denied）
//!   req   — 走目录协议连接 echo 并调用一次（Connect + Service::call）
//!   dir   — 目录协议自省（Discover + Enumerate）
//!   exit  — 退出 shell（RoomCall::Reap）

extern crate alloc;
// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use core::cell::{Cell, RefCell};
use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use protocol::dispatch::{MSG_LEN, Name, Reply, Request};

use protocol::console::client::{Console, Readline};
use protocol::dispatch::client::{Directory, E_DENIED, E_NOT_FOUND, PAYLOAD_LEN};
use protocol::doom::{self, Ack, Doom};
use protocol::irq;
use runtime::core::handshake::{self, Pier, Quay};
use runtime::core::lock::Lock;
use runtime::core::unit;
use runtime::env::{
    chrono::{self, clock},
    mail::{self, AnyPie as _, HOLE_MTU_MAX, HolePie, PolePie},
    room::{self, sleep},
    task::{heir_at, heir_count, join as task_join},
};

/// 目录请求门闩在**本任务侧**的句柄（启动期握手拿到，此后只读）。
static DIR_ENTRY: AtomicUsize = AtomicUsize::new(0);

/// 控制台请求门闩在本任务侧的句柄（启动期握手拿到，此后只读）。
static CONSOLE_ENTRY: AtomicUsize = AtomicUsize::new(0);

// ── 控制台适配（Terminal 从"进程内模块"改为"服务的线对侧"）──────────────────

/// 前景色。**只留本 bin 真用到的两个**：旧 `term::Color` 是八色齐全，但全仓只有
/// `Green`（标题）与 `Cyan`（提示符）被构造过——逐条判 `allow(dead_code)`
/// 的同一条判据（零消费者且没有第二个在路上的不留），其余六个别再带着。
#[derive(Clone, Copy)]
enum Color {
    Green,
    Cyan,
}

impl Color {
    fn code(self) -> u8 {
        match self {
            Color::Green => 32,
            Color::Cyan => 36,
        }
    }
}

/// Shell 的终端门面：**与旧 `programs::term::Terminal` 同 API**，故 70 处调用点一字未改；
/// 内部从"直接读写 UART"改成"与控制台服务说话"。
///
/// # 会话寿命：**一条会话活到进程结束**（性能修正）
///
/// 第一版每次 `write`/`readline` 各开关一条会话，于是**每条输出**要付
/// 2× `Channel::open`（各含 unseal+accord）＋ 2 次往返。实测反馈"输出延迟很高"，
/// 根因就在这里：旧路径一条输出是 **1 次 envcall**，而那时是 4 次往返。
/// 现在开一次就不关了——服务本来就支持多客户端（`MAX_CLIENTS`），而 shell 自己
/// 不会并发等读，故"单读者"那条约束一次也不会撞上。
///
/// # 写缓冲：按行合并（同一次修正）
///
/// 协议一条消息只带 `PAYLOAD_LEN`（24）字节，`sq > ` 这类短串也要一次往返。故
/// `write` 先攒进 [`Terminal::buf`]，**遇换行或满 [`Terminal::CAP`] 才发**；发大块
/// 时再按 `PAYLOAD_LEN` 切片。读行前强制 flush——次序不能颠倒，否则提示符会晚于
/// 读行落屏。
///
/// # 服务连不上时没有退路（**这是设计**，不是缺口）
///
/// 设备在 console 服务手里，任务侧连 `IOCall` 都没有了（第三步删）。所以本门面只有
/// 一条通路：**写丢了不致命，读不到就收场**（[`Term::readline`]）。
/// 过渡期那条"退回直连设备"的护栏随 `IOCall` 一起删——它护的是一个正在消失的世界。
struct Terminal {
    /// 控制台会话（首次用时开，之后一直用）。`Cell` 便于"锁内取走、锁外建、放回"。
    session: Cell<Option<Console>>,
    /// 写缓冲：攒到换行/满再发。
    buf: RefCell<String>,
}

impl Terminal {
    /// 一行的上界（超过就先把缓冲发掉）。
    const CAP: usize = 128;
}

/// 连不上控制台服务时的退出原因码（域自己的编号；trace 里 `RoomEvent::Exit` 带它）。
const NO_CONSOLE: usize = 0x51;

/// 全局唯一的终端门面（70 处调用点传的都是它的引用）。
///
/// 用 `runtime::core::lock::Lock`（`const fn new` ⇒ 能进 `static`）而不是 `RefCell`：
/// 后者不是 `Sync`，进不了 `static`。**它不可重入**——故会话建立一律在锁外做完
/// 再进临界区（见 [`with_session`]），临界区里只有内存操作，自旋窗口极小。
/// 门面的新类型：`Lock<Terminal>` 是外部类型，不能为它写 inherent impl。
struct Term(Lock<Terminal>);

static TERM: Term = Term(Lock::new(Terminal {
    session: Cell::new(None),
    buf: RefCell::new(String::new()),
}));

/// 借出控制台会话：**锁内取走 → 锁外建（要跑内核调用）→ 锁内放回**。
///
/// 锁不可重入，故这一段的顺序是硬要求：`Console::open` 绝不能出现在临界区里。
/// 建不出来 → `None`——**没有第二通路可退**（设备在服务手里），调用方见 [`flush`]
/// 与 [`Term::readline`] 各自的处置。
fn with_session<T>(f: impl FnOnce(&Console) -> T) -> Option<T> {
    let mut session = TERM.0.with(|t| t.session.take());
    if session.is_none() {
        let token = console_entry()?;
        session = Console::open(HolePie::from_token(token)).ok();
    }
    let out = session.as_ref().map(f);
    TERM.0.with(|t| t.session.set(session));
    out
}

/// 把缓冲里的字节发给服务（服务侧同步落到设备）。
///
/// 空缓冲不发——这条让"没有输出的命令"不再欠一次往返。
fn flush(term: &Term) {
    let text = term.0.with(|t| {
        let mut buf = t.buf.borrow_mut();
        if buf.is_empty() {
            return None;
        }
        Some(core::mem::take(&mut *buf))
    });
    let Some(text) = text else {
        return;
    };
    // 发不出去就是发不出去：本侧没有设备可直连（设备在服务手里），也没有第二条
    // 通路。**不判死**——一段输出丢掉不是"域不可续"，而 `readline` 那边会判（见
    // [`Term::readline`]）：一个连不上控制台的 shell 只可能是个哑巴，那时才收场。
    let _ = with_session(|c| c.write(&text).is_ok());
}

impl Term {
    /// 裸写（不加 `\n`）：攒进缓冲，遇换行或满才发。
    ///
    /// 临界区里**只有内存操作**（push + 判满）；真正发出去（要跑内核调用）在锁外。
    fn write(&self, s: &str) {
        let due = self.0.with(|t| {
            let mut buf = t.buf.borrow_mut();
            buf.push_str(s);
            buf.len() >= Terminal::CAP || buf.ends_with('\n')
        });
        if due {
            flush(self);
        }
    }

    /// 写一行（自动追加 `\n`）。
    fn writeline(&self, s: &str) {
        self.write(&format!("{s}\n"));
    }

    /// 清屏 + 光标回 home（`ESC[2J` + `ESC[H`）。
    fn clear(&self) {
        self.write("\x1b[2J\x1b[H");
    }

    /// 前景色（`ESC[3xm`）。
    fn fg(&self, color: Color) {
        self.write(&format!("\x1b[{}m", color.code()));
    }

    /// 复位 SGR（`ESC[0m`）。
    fn reset(&self) {
        self.write("\x1b[0m");
    }

    /// 读一整行。**先把缓冲刷出去、再读**——提示符必须先落屏（协议里 `Write` 同步）。
    ///
    /// 刻意**不持锁**：锁不可重入，而读行要先经 [`flush`] → [`with_session`] 取会话。
    fn readline(&self, prompt: &str) -> Readline {
        // 先刷掉暂存的输出：提示符必须**先**落屏，否则用户对着空行打字。
        flush(self);
        // prompt 交给客户端：它自己会先把它同步写出去（`Write` 的 Ok 即"已落屏"），
        // 再把长度与内容带进 `ReadLine` 请求——服务侧重绘要用它。
        match with_session(|c| c.readline(prompt)) {
            Some(Ok(r)) => r,
            // 服务连不上 / 会话死了：**任务侧没有设备可直连**（第三步删了 `IOCall`），
            // 再退也没有可退的地方。当场收场，让内核把原因码记进 trace——一个读不到
            // 命令的 shell 继续活着只会更难诊断。
            _ => runtime::env::room::exit_with(NO_CONSOLE),
        }
    }
}

/// 启动期握手：靠泊 → 自建控制孔并交给父域 → 报到 → 收配给（目录门闩由 dir 亲授）。
///
/// **只有目录那一枚是配给的**。控制台入口不配给，走**目录**取（见
/// [`console_entry`]）——理由：既有配给机制（`Refer{who}` → dir 控制线程 accord
/// 目录自己的入口）**只认目录自己**，报文里没有"配哪个服务"的身份位；而控制台
/// 已经注册进目录了，`Connect` 本来就是为这件事存在的操作。
fn shake() -> env::EnvResult<env::PieToken> {
    let up = handshake::moor()?;
    let down = HolePie::unseal(handshake::MTU)?;
    let sire = runtime::env::task::sire()?;
    let at_parent = down.accord(sire, env::Permission::READ | env::Permission::WRITE)?;
    Quay::new(at_parent).push(&up)?;
    Ok(Pier::pull(&down)?.token())
}

/// 控制台入口：**首次使用时**经目录 `Connect` 取一次，之后缓存。
///
/// 返回 `None` = 还没取到（目录没起、console 没注册、或取失败）——调用方降级到
/// 直连设备。**这是引导期窗口**：console 是 `shell` 之前启动的服务（见
/// `root/main.rs` 的启动次序），所以正常情况下第一次输出之前就已就绪。
fn console_entry() -> Option<usize> {
    let cached = CONSOLE_ENTRY.load(Ordering::Relaxed);
    if cached != 0 {
        return Some(cached);
    }
    let dir = DIR_ENTRY.load(Ordering::Relaxed);
    if dir == 0 {
        return None;
    }
    let session = Directory::open(HolePie::from_token(dir)).ok()?;
    // 只要**入口 token**，不要 `connect` 的回信通道：后者要先建再断，而
    // `disconnect` 会 release 掉刚拿到的入口——实测踩过这个坑。
    let token = session.connect_token("console").ok()?.get();
    if token != 0 {
        CONSOLE_ENTRY.store(token, Ordering::Relaxed);
        Some(token)
    } else {
        None
    }
}

/// 打开一次目录会话（每次新建 reply hole；entry 只是重建句柄）。
fn dir_session() -> env::EnvResult<Directory> {
    Directory::open(HolePie::from_token(DIR_ENTRY.load(Ordering::Relaxed)))
}

/// 按空白切词（保留空输入 = 空 Vec）。
fn split(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_string).collect()
}

/// 等目标结束。`Join` 返回真 ⇒ **收尾已完成**（内核契约：退出钩子已跑完），
/// 故调用方拿到真之后即可断言「它名下的门闩与通道都消失了」，无需重试。
///
/// 调用模式与 `MailCall::Wait` 同源：挂起过的那次只当「醒了一次」，须复探。
fn join_done(tid: env::TaskId) {
    loop {
        if task_join(tid, 0).unwrap_or(true) {
            return;
        }
        let _ = task_join(tid, usize::MAX);
    }
}

/// 派生级联自检（`cascade` 命令）。
///
/// 四段判据：
///   1. 三跳 A→B→C：撤销中间那跳 B，末端 C 必须失效；
///   2. 无关分支 D 不受波及，资源本体 A 仍可用；
///   3. `release` 同样级联：放下 A → D 随之下线；
///   4. 任务消亡级联：closure 授出的 Q 随它消亡而失效（退出钩子 `gate::doom`）
///      ——`Join` 返回真即已收尾，故当场断言、不重试。
///
/// 门闩是 per-task 的，同域两个线程也不能共享——故与 closure 的交接一律走
/// 共享内存槽 + `room` 键（与启动期握手同一手法）。
fn cascade(term: &Term) {
    const WAIT: usize = 5_000;

    let rw = env::Permission::READ | env::Permission::WRITE;
    let msg = [0x5au8; 8];
    let mut buf = [0u8; 8];

    let me = match unit::self_id() {
        Ok(id) => id,
        Err(_) => {
            term.writeline("cascade: no self id");
            return;
        }
    };
    let a = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(p) => p,
        Err(_) => {
            term.writeline("cascade: unseal failed");
            return;
        }
    };
    let b = match a.accord(me, rw | env::Permission::VEST) {
        Ok(t) => HolePie::from_token(t),
        Err(_) => {
            term.writeline("cascade: accord b failed");
            return;
        }
    };
    let d = match a.accord(me, rw) {
        Ok(t) => HolePie::from_token(t),
        Err(_) => {
            term.writeline("cascade: accord d failed");
            return;
        }
    };

    // ── 1+2：三跳撤销 + 无关分支 ──
    let c_slot: &'static [AtomicUsize; 1] = Box::leak(Box::new([AtomicUsize::new(0)]));
    let c_ptr = c_slot.as_ptr() as usize;
    let key_c = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let key_ack = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let key_rev = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;

    let join_c = unit::closure(move || {
        let _ = room::wait(key_c, WAIT);
        let c = HolePie::from_token(unsafe {
            (*(c_ptr as *const AtomicUsize)).load(Ordering::Relaxed)
        });
        let mut buf = [0u8; 8];
        // 撤销前：C 可用（B 还在）。
        let before = c.push(&msg).is_ok();
        let _ = c.pull(&mut buf); // 清槽：让 after 的失败只可能来自撤销
        let _ = room::wake(key_ack);
        // 等 shell 撤销 B。
        let _ = room::wait(key_rev, WAIT);
        let after = c.push(&msg).is_ok();
        (before, after)
    });
    let c = match b.accord(join_c.id(), rw) {
        Ok(t) => HolePie::from_token(t),
        Err(_) => {
            drop(join_c);
            term.writeline("cascade: accord c failed");
            return;
        }
    };
    c_slot[0].store(c.token(), Ordering::Relaxed);
    let _ = room::wake(key_c);
    let _ = room::wait(key_ack, WAIT);

    // 撤销 B：C 应随之失效（级联）。
    let revoked = b.revoke(me, env::PieToken::new(b.token())).is_ok();
    let _ = room::wake(key_rev);
    let (before, after) = join_c.join();

    let b_dead = b.push(&msg).is_err();
    let d_ok = d.push(&msg).is_ok();
    let _ = d.pull(&mut buf);
    let a_ok = a.push(&msg).is_ok();
    let _ = a.pull(&mut buf);
    term.writeline(&format!(
        "cascade: revoke={revoked} C.before={before} C.after={after} B.dead={b_dead} D={d_ok} A={a_ok}"
    ));

    // ── 3：release 级联 ──
    let released = a.release().is_ok();
    let d_dead = d.push(&msg).is_err();
    term.writeline(&format!("cascade: release={released} D.after={d_dead}"));

    // ── 4：任务消亡级联 ──
    let r = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(p) => p,
        Err(_) => {
            term.writeline("cascade: unseal r failed");
            return;
        }
    };
    let r2_slot: &'static [AtomicUsize; 1] = Box::leak(Box::new([AtomicUsize::new(0)]));
    let q_slot: &'static [AtomicUsize; 1] = Box::leak(Box::new([AtomicUsize::new(0)]));
    let r2_ptr = r2_slot.as_ptr() as usize;
    let q_ptr = q_slot.as_ptr() as usize;
    let key_r = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let key_q = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;

    let join_e = unit::closure(move || {
        let _ = room::wait(key_r, WAIT);
        let r2 = HolePie::from_token(unsafe {
            (*(r2_ptr as *const AtomicUsize)).load(Ordering::Relaxed)
        });
        if let Ok(q) = r2.accord(me, rw) {
            unsafe { (*(q_ptr as *const AtomicUsize)).store(q.get(), Ordering::Relaxed) };
        }
        let _ = room::wake(key_q);
    });
    let e_tid = join_e.id();
    let r2 = match r.accord(e_tid, rw | env::Permission::VEST) {
        Ok(t) => HolePie::from_token(t),
        Err(_) => {
            drop(join_e);
            term.writeline("cascade: accord r2 failed");
            return;
        }
    };
    r2_slot[0].store(r2.token(), Ordering::Relaxed);
    let _ = room::wake(key_r);
    let _ = room::wait(key_q, WAIT);
    let q = HolePie::from_token(q_slot[0].load(Ordering::Relaxed));
    drop(join_e);
    join_done(e_tid);
    // `Join` 返回真 ⇒ 退出钩子（`gate::doom`）已跑完 ⇒ Q 当场就死了——不必重试。
    let q_dead = q.push(&msg).is_err();
    term.writeline(&format!("cascade: task-exit q.dead={q_dead}"));

    let ok = revoked && before && !after && b_dead && d_ok && a_ok && released && d_dead && q_dead;
    term.writeline(if ok { "cascade: ok" } else { "cascade: FAIL" });
}

/// 任务生灭压测（`churn` 命令）——**主动探测**：不等间歇自己撞上来，把
/// 「产生 → 收尾 → 回收」这条链反复走到足够深的树上，让偶发的那条路径自己现形。
///
/// 判据不是「有没有崩」——**崩不崩是既有探针的事**，本命令的产物是**关机时的
/// 逐种类终值**：每轮每一层任务都带一份 `Arc<Task>`/`Arc<TaskIdent>`（内核侧
/// `Kind::Task`），回收漏一个，关机审计就点名报出来（`[audit] leak: task N`
/// 给的 N 就是**这一轮压测的刻度**：N 应该恒为 0，与轮数无关）。
///
/// 形状：树深 `1 + depth`（每层 `fan` 叉并行），每轮等整棵树 join 完再开下一轮
/// ——故「在飞」的任务数与树的大小同阶、不随轮数增长，压的是生灭**次数**而非
/// 并存量。默认 200 轮 × 深 2 × 2 叉 ≈ 1400 个任务；轮数由参数给，便于两端
/// 对照（`churn 1` 与 `churn 2000` 的关机账必须逐字相同：都是零）。
///
/// 一层的孩子全部 join 完才返回上一层——**没人「弃权」**（`Join::drop` 的 LEFT
/// 路径在本命令里一次都不会走到），故失败只可能来自内核侧（句柄/权限/内存），
/// 不会与「父方不等了」混在一起。
fn churn(term: &Term, rounds: usize, depth: usize, fan: usize) {
    /// 递归生灭一棵任务树。**全程可失败**：`churn` 的用途就是把池子压到耗尽，
    /// 而耗尽恰好发生在"产生任务"这一步；任何一处 `unit::closure`（失败即
    /// `panic`）都会让整个 `shell` 进程退出、压测不给结论（实测
    /// `exit tid=3434 reason=0xffffff01 note: task spawn failed: EnvError(-4)`）。
    ///
    /// 故这里每一层都用 `try_closure` 并把 `Spawn`/`Hatch` 的错**原样上抛**。
    fn tree(depth: usize, fan: usize) -> Result<Vec<usize>, env::EnvError> {
        if depth == 0 {
            return Ok(Vec::new());
        }
        let mut kids = Vec::new();
        for _ in 0..fan {
            let j = unit::try_closure(move || tree(depth - 1, fan)).map_err(|e| e.source)?;
            kids.push(j);
        }
        let mut out = Vec::new();
        for k in kids {
            // `join()` = `EnvResult<Vec<usize>>`（错误类型是 erra 包装的 `EnvError`）
            // ——子任务里的失败经它原样上抛，不再伪装成"生出来了且很干净"。
            match k.join() {
                Ok(v) => out.extend(v),
                Err(e) => return Err(env::EnvError::from_raw(e.code())),
            }
        }
        out.push(depth);
        Ok(out)
    }

    let t0 = clock().ok();
    for r in 0..rounds {
        // 压测自身的失败**必须现形**：`unit::closure` 失败即 panic，而 panic 发生在
        // 被压的那个子任务里 ⇒ 父方只看到 join 醒来，账面上「什么都没发生」。故这里
        // 自己接住 `Spawn`/`Hatch` 的错，把「没生出来」与「生出来且回收干净」分开
        // ——否则一次失败会伪装成「压过了、很干净」。
        // 闭包要 `'static`：两个量先按值抄进来（`dispatcher` 端 `fan` 随后还要用）。
        let (d, f) = (depth, fan);
        // **外层 spawn 也要可失败**：`churn` 是拿来把池子压到耗尽的工具，而
        // 「耗尽」恰好就发生在**产生任务**这一步。旧版这里用
        // `.expect("churn: outer closure spawn failed")` —— 于是压测把自己压死：
        // 一次 `Spawn` 拿不到帧 ⇒ 用户态 panic ⇒ 整个 `shell` 进程退出 ⇒
        // 系统停机、而**压测没给出结论**（实测：`exit tid=3469 reason=0xffffff01
        // note: task spawn failed: EnvError(-4)`，harness 只能干等到超时）。
        //
        // 内外层都走 `try_closure`，撞墙就**如实报告并正常收尾**：这是压测该有的
        // 行为——它要观测的是"生不出来"，不是"自己也死了"。
        let step = match unit::try_closure(move || -> Result<(), (isize, usize)> {
            let mut spawned = 0usize;
            for _ in 0..f {
                match unit::try_closure(move || tree(d - 1, f)) {
                    Ok(j) => {
                        spawned += 1;
                        // `join` 拿到的是子任务里的 `Result`——失败同样上抛，
                        // 不再伪装成"生出来了且很干净"。
                        if let Err(e) = j.join() {
                            return Err((e.code(), spawned));
                        }
                    }
                    Err(e) => {
                        return Err((e.source.code(), spawned));
                    }
                }
            }
            Ok(())
        }) {
            Ok(j) => j.join(),
            Err(e) => Err((e.source.code(), 0)),
        };
        if let Err((code, spawned)) = step {
            term.writeline(&format!(
                "churn: spawn failed with code {code} after {spawned}/{fan} at round {}",
                r + 1
            ));
            return;
        }
        if r % 20 == 19 {
            term.writeline(&format!("churn: {}/{}", r + 1, rounds));
        }
    }
    // 时钟是 `(秒, 纳秒)` 两段——先各自折成总纳秒再相减（与 `wait_verdict` 同一手法），
    // 避免「纳秒借位」时算出负数。
    let ms = match (t0, clock().ok()) {
        (Some(a), Some(b)) => {
            let ns = |t: (u64, u64)| t.0.saturating_mul(1_000_000_000).saturating_add(t.1);
            ns(b).saturating_sub(ns(a)) / 1_000_000
        }
        _ => 0,
    };
    term.writeline(&format!(
        "churn: {rounds} rounds x depth {depth} x fan {fan} done in {ms} ms"
    ));
}

/// 静息时刻的一行读数：水位 + **逐类在册帧数**（`when` = `before` / `after`）。
///
/// 分类水位是判漏该用的表：只盯池总量，任何一类在漏都长一个样——我为此从总量
/// 反推机制，编了三个错误结论。逐类看才能直接指到漏的是 `table` 还是 `stack`。
fn idle_kinds(term: &Term, when: &str) -> (usize, usize) {
    let mut buf = [0u8; 512];
    let w = runtime::env::memory::watermark_kinds(&mut buf).unwrap_or((0, 0));
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    let kinds = core::str::from_utf8(&buf[..end]).unwrap_or("<non-utf8>");
    term.writeline(&format!(
        "idle-{when} held={} walk={} | {kinds}",
        w.0, w.1
    ));
    w
}

/// 池水位**闭环校准**（`calib [rounds]` 命令）——先证读数可信，再拿它量泄漏。
///
/// # 为什么必须先校准
///
/// 先前所有泄漏结论都建立在「freelist 走链」这个读数上，而它与分配器自己的
/// `pagemeta` 记账对不上（走链说"放进去了"、空闲数却在跌）。**没有可信读数，
/// 一切速率都是编的**。
///
/// # 判据只有一条：净变化必须为 0
///
/// `alloc(1 页) → free(同一页)` 是**配平**的操作。重复 `rounds` 遍，只在首尾各
/// 读一次水位：**净变化不为 0 就是漏**，漏的速率 = `Δ / rounds`。
///
/// 关键在**保持循环里没有别的东西**：不收集地址（`Vec` 自己会分配/扩容，那是
/// 被测量对象不是测量工具）、不每轮打印（终端输出也走堆）。先前的版本每轮建
/// 一个 `Vec` + 打一行，读数里混进了这两样——**先让循环干净，再谈池子**。
///
/// 两个地址轮流用（`a`、`b`），既避免"同一个地址反复 alloc/free"这种退化路径，
/// 又不需要任何容器。
fn calib(term: &Term, rounds: usize) {
    const PAGE: usize = 4096;
    let rounds = rounds.max(1);

    let (w0_held, w0_walk) = runtime::env::memory::watermark().unwrap_or((0, 0));

    // ── 预热：先跑几轮把懒物化（页表/窗口）打掉，免得混进下面的读数 ──
    for _ in 0..200 {
        if let Ok(a) = runtime::env::memory::allocate(PAGE) {
            let _ = runtime::env::memory::deallocate(a, PAGE);
        }
    }

    let (b0_held, b0_walk) = runtime::env::memory::watermark().unwrap_or((0, 0));
    term.writeline(&format!(
        "calib warmup 200x alloc+free: held {w0_held}->{b0_held} (Δ{}), walk {w0_walk}->{b0_walk} (Δ{})",
        b0_held as i64 - w0_held as i64,
        b0_walk as i64 - w0_walk as i64,
    ));

    // ── 干净循环：只有 alloc/free 本身 ──
    let mut fails = 0usize;
    for _ in 0..rounds {
        match runtime::env::memory::allocate(PAGE) {
            Ok(a) => {
                if runtime::env::memory::deallocate(a, PAGE).is_err() {
                    fails += 1;
                }
            }
            Err(_) => fails += 1,
        }
    }

    let (b1_held, b1_walk) = runtime::env::memory::watermark().unwrap_or((0, 0));
    let d_held = b1_held as i64 - b0_held as i64;
    term.writeline(&format!(
        "calib clean rounds={rounds} fails={fails} held {b0_held}->{b1_held} (Δ{d_held}, per_round {}) walk {b0_walk}->{b1_walk} (Δ{})",
        d_held / rounds as i64,
        b1_walk as i64 - b0_walk as i64,
    ));

    // ── 对照：**一批** N 页一起分配再一起释放，问"残差是否随批次累加"──
    //
    // 这是与上面唯一的差别：干净循环在页与页之间把窗口腾空，批次不腾空。
    // 若残差集中在头几批后归零 ⇒ 内核为**峰值并发**留下的常驻容量（不是漏）；
    // 若每批都涨同样的量 ⇒ 真漏，且漏点与"同时在手页数"有关。
    for keep in [8usize, 32, 64, 128] {
        let (c0_held, _) = runtime::env::memory::watermark().unwrap_or((0, 0));
        let mut line = format!("calib batch n={keep}");
        for _pass in 1..=4u32 {
            let (p0_held, _) = runtime::env::memory::watermark().unwrap_or((0, 0));
            let mut addrs = Vec::new();
            for _ in 0..keep {
                if let Ok(a) = runtime::env::memory::allocate(PAGE) {
                    addrs.push(a);
                }
            }
            let (p1_held, _) = runtime::env::memory::watermark().unwrap_or((0, 0));
            for a in addrs.drain(..) {
                let _ = runtime::env::memory::deallocate(a, PAGE);
            }
            let (p2_held, _) = runtime::env::memory::watermark().unwrap_or((0, 0));
            line.push_str(&format!(
                " | allocΔ{} res{}",
                p1_held as i64 - p0_held as i64,
                p2_held as i64 - p0_held as i64,
            ));
        }
        let (c1_held, _) = runtime::env::memory::watermark().unwrap_or((0, 0));
        term.writeline(&format!(
            "{line} | totalΔ{}",
            c1_held as i64 - c0_held as i64
        ));
    }
    term.writeline("calib done");
}

/// 资源寿命自检（`reclaim` 命令）。
///
/// 三段判据：
///   1. 反复 unseal + release —— 泄漏则耗尽帧池（≈128 MB / 4 KB ≈ 3 万帧）；
///   2. 封印只归开辟者：他人 `Seal` 被拒；主人 `Seal` 后他人操作得 `Dead`；
///   3. 开辟者消亡 → 它开的资源随之回收（我手里的副本随 `doom` 失效）。
fn reclaim(term: &Term) {
    const ROUNDS: usize = 40_000;
    const WAIT: usize = 5_000;

    let rw = env::Permission::READ | env::Permission::WRITE;
    let msg = [0x5au8; 8];
    let mut buf = [0u8; 8];

    // ── 1：资源随最后一份能力回收 ──
    let mut failed_at = 0usize;
    for i in 1..=ROUNDS {
        match PolePie::unseal(4096) {
            Ok(p) => {
                let _ = p.release();
            }
            Err(_) => {
                failed_at = i;
                break;
            }
        }
    }
    term.writeline(&format!(
        "reclaim: unseal+release x{ROUNDS} failed_at={failed_at}"
    ));

    let me = match unit::self_id() {
        Ok(id) => id,
        Err(_) => {
            term.writeline("reclaim: no self id");
            return;
        }
    };

    // ── 2：封印只归开辟者 ──
    let a = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(p) => p,
        Err(_) => {
            term.writeline("reclaim: unseal a failed");
            return;
        }
    };
    let c_slot: &'static [AtomicUsize; 1] = Box::leak(Box::new([AtomicUsize::new(0)]));
    let c_ptr = c_slot.as_ptr() as usize;
    let k_ready = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let k_sealed = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let k_ack = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;

    let join = unit::closure(move || {
        let _ = room::wait(k_ready, WAIT);
        let c = HolePie::from_token(unsafe {
            (*(c_ptr as *const AtomicUsize)).load(Ordering::Relaxed)
        });
        let denied = c.seal().is_err(); // 非开辟者 → 应被拒
        let _ = room::wake(k_ack);
        let _ = room::wait(k_sealed, WAIT);
        let dead = c.push(&msg).is_err(); // 主人封印后 → 应失效
        (denied, dead)
    });
    let c = match a.accord(join.id(), rw) {
        Ok(t) => HolePie::from_token(t),
        Err(_) => {
            drop(join);
            term.writeline("reclaim: accord failed");
            return;
        }
    };
    c_slot[0].store(c.token(), Ordering::Relaxed);
    let _ = room::wake(k_ready);
    let _ = room::wait(k_ack, WAIT);
    let owner_sealed = a.seal().is_ok();
    let _ = room::wake(k_sealed);
    let (denied, dead) = join.join();
    term.writeline(&format!(
        "reclaim: other.seal_denied={denied} owner.seal={owner_sealed} other.after={dead}"
    ));

    // ── 3：开辟者消亡 → 资源随之回收 ──
    let q_slot: &'static [AtomicUsize; 1] = Box::leak(Box::new([AtomicUsize::new(0)]));
    let q_ptr = q_slot.as_ptr() as usize;
    let k_open = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let k_q = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let k_done = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;

    let join_e = unit::closure(move || {
        let _ = room::wait(k_open, WAIT);
        if let Ok(r) = HolePie::unseal(HOLE_MTU_MAX)
            && let Ok(q) = r.accord(me, rw)
        {
            unsafe { (*(q_ptr as *const AtomicUsize)).store(q.get(), Ordering::Relaxed) };
        }
        let _ = room::wake(k_q);
        // 等本侧检查完「退出前可用」再返回——否则 `doom` 会在检查之前就把 q 收走。
        let _ = room::wait(k_done, WAIT);
    });
    let e_tid = join_e.id();
    let _ = room::wake(k_open);
    let _ = room::wait(k_q, WAIT);
    let q = HolePie::from_token(q_slot[0].load(Ordering::Relaxed));
    let q_ok = q.push(&msg).is_ok();
    let _ = q.pull(&mut buf);
    let _ = room::wake(k_done);
    drop(join_e);
    join_done(e_tid);
    // `Join` 返回真 ⇒ 退出钩子已跑完 ⇒ q 当场失效（无需重试）。
    let q_dead = q.push(&msg).is_err();
    term.writeline(&format!(
        "reclaim: owner-exit q.before={q_ok} q.after={q_dead}"
    ));

    let ok = failed_at == 0 && denied && owner_sealed && dead && q_ok && q_dead;
    term.writeline(if ok { "reclaim: ok" } else { "reclaim: FAIL" });
}

/// 身份伪造自检（`spoof` 命令）。
///
/// 修复前：目录按请求体里的回信 token 求 `vestor` 认人——**猜中别人的 token 即可
/// 冒充**。修复后：身份 = **内核在 `Push` 时盖章的发送者**，报文里的字段只当回信
/// 地址用，且必须**确实是该发送者授给目录的那一枚**。
///
/// 四段判据：
///   1. 内核盖章：自己推的消息，`pull_from` 回来的发送者是自己；
///   2. 正向对照：用自己那枚回信 token 注册 / 解绑**预约给本域**的名字 → 两次 `Ok`；
///   3. 攻击：把 `1..=200` 逐个当作「猜中的回信 token」发 `Unregister("echo")`
///      ——目录按 sender 认人，全部失败，echo 的名字仍在；
///   4. 攻击者收不到任何 `Ok`（回信地址不属于发送者即被丢弃）。
fn spoof(term: &Term) {
    const WAIT: usize = 1_000;
    const GUESS_MAX: usize = 200;
    const REPLY_AT: usize = protocol::dispatch::REPLY_AT;
    let rw = env::Permission::READ | env::Permission::WRITE;

    let me = unit::self_id().unwrap_or(env::TaskId::new(0));

    // ── 1：内核盖章 ──
    let self_stamp = (|| -> Option<bool> {
        let h = HolePie::unseal(64).ok()?;
        h.push(b"x").ok()?;
        let mut b = [0u8; 64];
        let (_, from) = h.pull_from(&mut b).ok()?;
        Some(from == me)
    })()
    .unwrap_or(false);

    let entry = HolePie::from_token(DIR_ENTRY.load(Ordering::Relaxed));
    let dir_id = match mail::reserve(env::PieToken::new(entry.token())) {
        Ok((_, owner)) => owner.get(),
        Err(_) => {
            term.writeline("spoof: no dir id");
            return;
        }
    };
    // 本任务自己的回信孔（攻击者身份就用它）。
    let mine = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(p) => p,
        Err(_) => {
            term.writeline("spoof: unseal failed");
            return;
        }
    };
    let at_dir = match mine.accord(env::TaskId::new(dir_id), rw) {
        Ok(t) => t.get(),
        Err(_) => {
            term.writeline("spoof: accord failed");
            return;
        }
    };
    let mut buf = [0u8; MSG_LEN];
    // `ms` = 等回复的上界：正路径用 WAIT，猜 token 时用 0（只探测、顺便排空）。
    let mut call = |req: &Request, reply_tok: usize, ms: usize| -> Option<Reply> {
        let mut msg = req.encode();
        msg[REPLY_AT..REPLY_AT + 8].copy_from_slice(&reply_tok.to_le_bytes());
        entry.push(&msg).ok()?;
        mine.pull_timeout(&mut buf, ms).ok()?;
        Reply::decode(&buf).ok()
    };

    // 入口门闩必须与回信孔分开：注销会**释放**目录侧那枚入口副本，若回信地址正是
    // 它，回复就无处可推（`unpublish` 先释放、回复后推）。
    let entry_hole = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(p) => p,
        Err(_) => {
            term.writeline("spoof: unseal failed");
            return;
        }
    };
    let entry_at_dir = match entry_hole.accord(env::TaskId::new(dir_id), rw) {
        Ok(t) => t,
        Err(_) => {
            term.writeline("spoof: accord failed");
            return;
        }
    };

    // 每次探测都用**新会话**：攻击循环可能把本任务自己的回信槽灌进一条陈旧回复
    //（攻击者只能污染自己的孔——`reachable` 检查挡住了替他人收信）。
    let before = dir_session()
        .and_then(|d| d.discover("echo"))
        .unwrap_or(false);

    // ── 2：正向对照（root 预约给本域的名字，自己的回信 token）──
    let own_ok = match Name::new("shell") {
        Ok(n) => {
            let reg = call(
                &Request::Register {
                    name: n,
                    entry: entry_at_dir,
                },
                at_dir,
                WAIT,
            );
            let unreg = call(&Request::Unregister { name: n }, at_dir, WAIT);
            matches!(reg, Some(Reply::Ok)) && matches!(unreg, Some(Reply::Ok))
        }
        Err(_) => false,
    };

    // ── 3：攻击——逐个猜回信 token ──
    if let Ok(name) = Name::new("echo") {
        for tok in 1..=GUESS_MAX {
            // 回复只可能落到被猜中的那枚 token 的孔里（攻击者看不见），此处探测
            // 仅用于排空本任务自己的回信槽——判据是「echo 还在不在」。
            let _ = call(&Request::Unregister { name }, tok, 0);
        }
    }
    let after = dir_session()
        .and_then(|d| d.discover("echo"))
        .unwrap_or(false);

    term.writeline(&format!(
        "spoof: stamp={self_stamp} own={own_ok} echo.before={before} echo.after={after} (guess {GUESS_MAX})"
    ));
    let ok = self_stamp && own_ok && before && after;
    term.writeline(if ok { "spoof: ok" } else { "spoof: FAIL" });
}

/// 名字权限自检（`name` 命令）。
///
/// 目录的表只能由**父域（root）的预约**产生：注册只能**填**已预约的行。判据六段：
///   1. 预约者注册自己的名字 → `Ok`，且 `discover` 为真；
///   2. 注销只摘实例：`discover` 转假（名字仍归本域）；
///   3. 非预约者注册别人的名字 → `Denied`；
///   4. 未预约的名字 → `NotFound`；
///   5. 实例门闩消亡（释放源门闩 → 目录侧副本随 `sire` 级联摘掉）→ 名字自动回到
///      「无实例」；
///   6. 死实例不锁名字：重新注册成功。
fn name(term: &Term) {
    let dir = match dir_session() {
        Ok(d) => d,
        Err(_) => {
            term.writeline("name: no session");
            return;
        }
    };
    let code = |r: env::EnvResult<()>| r.err().map(|e| e.into_source().code());

    // ── 1+2：预约者注册 / 注销只摘实例 ──
    let publish = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(h) => dir.register("shell", &h).is_ok(),
        Err(_) => false,
    };
    let visible = dir.discover("shell").unwrap_or(false);
    let unpublish = dir.unregister("shell").is_ok();
    let gone = !dir.discover("shell").unwrap_or(true);

    // ── 3+4：非预约者 / 未预约的名字（各用一枚门闩；末尾释放以清掉目录侧副本）──
    let (foreign, ghost) = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(h) => {
            let f = code(dir.register("echo", &h)) == Some(E_DENIED);
            let g = code(dir.register("ghost", &h)) == Some(E_NOT_FOUND);
            let _ = h.release();
            (f, g)
        }
        Err(_) => (false, false),
    };

    // ── 5：实例门闩消亡 → 目录侧副本随 sire 级联摘掉 → 名字回到「无实例」──
    let stale = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(h) => {
            let reg = dir.register("shell", &h).is_ok();
            let _ = h.release();
            reg && !dir.discover("shell").unwrap_or(true)
        }
        Err(_) => false,
    };

    // ── 6：死实例不锁名字；末尾注销，恢复干净状态 ──
    let reuse = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(h) => dir.register("shell", &h).is_ok(),
        Err(_) => false,
    };
    let clean = dir.unregister("shell").is_ok();

    term.writeline(&format!(
        "name: publish={publish} visible={visible} unpublish={unpublish} gone={gone} foreign={foreign} ghost={ghost} stale={stale} reuse={reuse}"
    ));
    let ok = publish && visible && unpublish && gone && foreign && ghost && stale && reuse && clean;
    term.writeline(if ok { "name: ok" } else { "name: FAIL" });
}

/// 封印唤醒探针：在同一轮里以 `seal` 为界各等一次，把两次等待的**结论**分开打印。
///
/// 判据与打印的对应关系（门断言 `wake=seal` 那一支）：
///   - `wake=timeout`：封印前那一等——孔里不会有东西来，等满了 `WAIT_MS` 期限；
///   - `wake=seal`   ：**封印后**那一等——孔已置死，等待当场拿到结论、不睡满期限。
///
/// 「不睡满期限」怎么定得死：有界 pull **只有走完 deadline 才会报 Busy**
/// （`HolePie::pull_timeout` 的注释即此契约：「只有 `clock()` 真的走完 `millis`
/// 才报 Busy」）。故「这次调用耗时 < `WAIT_MS`」只可能是**没等满就拿到了结论**
/// ——即被 seal 唤醒；等满期限那一支必然 ≥ `WAIT_MS`。`WAKE_SLACK_MS` 是给
/// 「seal 到了才判」留的读数余量（远比一次 seal 的实际耗时宽松）。
///
/// 这一支有牙的地方：**若 seal 不唤醒等待者**（`wipe` 那条路断了），第二次等待
/// 就会睡满期限 ⇒ 打出的是 `wake=timeout` ⇒ 门的断言挂。反向验证（本轮实跑）
/// 正是把这一支改坏来做的。
/// 野 id 自检：拿**从未入册**的 task id 去 `Join`，看内核怎么答。
///
/// 契约（`docs/root.md` §3、`messenger::join`）：`Join` 收的是 task id，而「从未分配」是
/// **非法 id** ⇒ `-1 Denied`；「已回收」⇒ 真。两者若被折成同一条路（判活有两个
/// 真相源时就会这样），调用方就再也分不清「它早结束了」与「你给错了 id」。
///
/// 三个野 id 各探一次：`9999`（远超已分配过的 id——id 单调递增、永不复用）、
/// `9999` 配永久等待（挂起路径）、`0`（「无任务」哨兵，同样从未入册）。
/// 输出一行计数：`stray: 3/3 illegal-id joins denied`（反向验证时它是 `0/3`）。
fn stray_probe(term: &Term) {
    let cases = [(9999usize, 0usize), (9999, usize::MAX), (0, 0)];
    let mut denied = 0;
    for (id, millis) in cases {
        let r = task_join(env::TaskId(id), millis);
        if r.is_err() {
            denied += 1;
        }
        term.writeline(&format!(
            "stray: join({id}, {millis}) → {}",
            if r.is_err() { "denied" } else { "accepted" }
        ));
    }
    term.writeline(&format!("stray: {denied}/3 illegal-id joins denied"));
}

/// `kill <名字>`：**经 root 的他杀服务**收掉一个域。
///
/// 内核那枚 `RoomCall::Doom` 只认血缘（谁生的谁能杀，传递），而 root 是**全体域的
/// 祖先** ⇒ 跨血缘的"该不该"由它的政策回答。这就是 Linux `kill` 的形状：谁都能请求，
/// 够格的那个来执行（`docs/driver.md` §12）。
///
/// 回执**四态分开打**：`ok` 的含义是"**内核确认它回收完了**"（不是"收到了"）、
/// `dead` = 没这个目标、`denied` = 政策不许、`slow` = 已下令但没等到。判据取第一态。
fn kill_cmd(arg: Option<&str>, term: &Term) {
    let Some(name) = arg else {
        term.writeline("kill: usage: kill <name>");
        return;
    };
    let Ok(target) = Name::new(name) else {
        term.writeline(&format!("kill {name} -> bad name"));
        return;
    };
    let dir = match dir_session() {
        Ok(d) => d,
        Err(e) => {
            term.writeline(&format!("kill {name} err: dir {e:?}"));
            return;
        }
    };
    let entry = match dir.connect_token(doom::SERVICE) {
        Ok(t) => t,
        Err(_) => {
            term.writeline(&format!("kill {name} -> no doom service"));
            return;
        }
    };
    let svc = match Doom::open(HolePie::from_token(entry)) {
        Ok(s) => s,
        Err(_) => {
            term.writeline(&format!("kill {name} -> service refused"));
            return;
        }
    };
    match svc.kill(&target) {
        Ok(Ack::Ok) => term.writeline(&format!("kill {name} -> ok")),
        Ok(Ack::Dead) => term.writeline(&format!("kill {name} -> dead")),
        Ok(Ack::Denied) => term.writeline(&format!("kill {name} -> denied")),
        Ok(Ack::Slow) => term.writeline(&format!("kill {name} -> slow")),
        Err(e) => term.writeline(&format!("kill {name} err: {e:?}")),
    }
}

/// `line <设备名>`：**线的权威**的反证探针（`docs/driver.md` §12 甲）。
///
/// 本域**不是**任何设备名的属主（root 只把 console 那台交出去了）⇒ 这一句应当拿到
/// `not-yours`；而设备树里没有的名字应当拿到 `unknown`。两条合起来说清这件事：
/// **线号是名字的函数、名字的属主只能由 root 写**——报文里既没有线号，也没有任何
/// 可以让本域自证"这台设备是我的"的字段。
///
/// **判据有牙**：把驱动那条属主判据去掉，两条都会变成 `ok`（而且第一条真把 console
/// 的线抢走，输入随后退化成有界轮询）。
fn line_probe(arg: Option<&str>, term: &Term) {
    let Some(text) = arg else {
        term.writeline("line: usage: line <device-name>");
        return;
    };
    let Ok(name) = Name::new(text) else {
        term.writeline(&format!("line {text} -> bad name"));
        return;
    };
    let dir = match dir_session() {
        Ok(d) => d,
        Err(e) => {
            term.writeline(&format!("line {text} err: dir {e:?}"));
            return;
        }
    };
    let line = match irq::Line::connect(&dir) {
        Ok(l) => l,
        Err(_) => {
            term.writeline(&format!("line {text} -> no irq driver"));
            return;
        }
    };
    // 报文要一枚会话门闩（驱动往它投线号）。本探针**不会**读它——故登记万一被接受
    // （不该发生），当场封印：驱动下一次投递拿到 `Dead` ⇒ 它把这条线收掉（§12 ②）。
    let Ok(session) = HolePie::unseal(irq::LINE_LEN) else {
        term.writeline(&format!("line {text} -> no session"));
        return;
    };
    let verdict = match line.register(&name, &session) {
        Ok(irq::Ack::Ok) => "ok",
        Ok(irq::Ack::Refused(irq::Refused::Unknown)) => "unknown",
        Ok(irq::Ack::Refused(irq::Refused::Unclaimed)) => "unclaimed",
        Ok(irq::Ack::Refused(irq::Refused::NotYours)) => "not-yours",
        Ok(irq::Ack::Refused(irq::Refused::Taken)) => "taken",
        Err(_) => "no answer",
    };
    let _ = session.seal();
    term.writeline(&format!("line {text} -> {verdict}"));
}

fn seal_wake_probe(term: &Term) {
    /// 一次有界等待的期限：短到不拖慢门，长到足以让「等满」与「当场」区分开。
    const WAIT_MS: usize = 200;
    /// 「当场拿到结论」的读数余量：远大于一次 seal 的实际耗时（微秒级）。
    const WAKE_SLACK_MS: u64 = 5;

    let Ok(probe) = HolePie::unseal(HOLE_MTU_MAX) else {
        term.writeline("hole: wait-probe unseal failed");
        return;
    };
    let mut buf = [0u8; 8];

    // ① 封印前：等满期限（孔里不会有东西来）。
    term.writeline(&format!(
        "hole: wait-pre  {}",
        wait_verdict(&probe, &mut buf, WAIT_MS, WAKE_SLACK_MS)
    ));
    // 封印：本探针自己的孔，封印者 = 开辟者（本任务）。
    let sealed = probe.seal().is_ok();
    // ② 封印后：等待者当场拿到结论（seal 唤醒），故不会是 `timeout`。
    term.writeline(&format!(
        "hole: wait-seal sealed={} {}",
        sealed as u8,
        wait_verdict(&probe, &mut buf, WAIT_MS, WAKE_SLACK_MS)
    ));
    let _ = probe.release();
}

/// 一次有界等待的结论串：`wake=seal` / `wake=timeout` / `wake=msg`。
///
/// 前两者按「有没有睡满期限」区分（见 [`seal_wake_probe`] 的判据说明）；报错与
/// 两者都分开打——把错误折叠进 timeout 会把真实的失败伪装成正常结局。
fn wait_verdict(pie: &HolePie, buf: &mut [u8], wait_ms: usize, slack_ms: u64) -> String {
    // 时钟是 `(秒, 纳秒)` 两段——各段分段相减会在「纳秒借位」时算错，故先各自折成
    // 总纳秒再相减（`ns()` 只此一处）。
    fn ns(t: (u64, u64)) -> u64 {
        t.0.saturating_mul(1_000_000_000).saturating_add(t.1)
    }
    let t0 = clock().ok();
    let r = pie.pull_timeout(buf, wait_ms);
    let elapsed_ms = match (t0, clock().ok()) {
        (Some(a), Some(b)) => ns(b).saturating_sub(ns(a)) / 1_000_000,
        _ => u64::MAX, // 时钟读不到 ⇒ 按「等满了」算：宁漏报 seal，不误报
    };
    match r {
        Ok(_) => format!("wake=msg elapsed={elapsed_ms}ms"),
        Err(_) if elapsed_ms.saturating_add(slack_ms) < wait_ms as u64 => {
            format!("wake=seal elapsed={elapsed_ms}ms")
        }
        Err(e) => format!("wake=timeout elapsed={elapsed_ms}ms err={e:?}"),
    }
}

/// 各系统能力命令。全部输出经 `term`（唯一 console 出口）。
/// 返回 false = 退出（exit 命令）。
fn exec(cmd: &str, args: &[String], term: &Term) -> bool {
    match cmd {
        "help" => {
            term.writeline(
                "help / clock / ticks / alloc / echo / sleep / spawn / heir / hole / req / dir / kill / line / cascade / churn / reclaim / spoof / name / badslot / stray / exit",
            );
        }
        "clock" => {
            let (s, n) = clock().unwrap_or((0, 0));
            term.writeline(&format!("clock {s}.{:09} sec", n));
        }
        "ticks" => {
            let t = chrono::ticks().unwrap_or(0);
            term.writeline(&format!("ticks {t}"));
        }
        "alloc" => {
            let addr = runtime::env::memory::allocate(4096).unwrap_or(0);
            term.writeline(&format!("alloc -> {addr:#x}"));
        }
        "echo" => {
            term.writeline(&args.join(" "));
        }
        "sleep" => {
            let ms = args
                .first()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            term.writeline(&format!("sleep {ms}ms"));
            let _ = sleep(Duration::from_millis(ms));
            term.writeline("woke");
        }
        "spawn" => {
            // 闭包 join：算 0..N。
            let n = args
                .first()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(1000);
            let sum = unit::closure(move || {
                let mut acc: u64 = 0;
                for i in 0..n {
                    acc = acc.wrapping_add(i);
                }
                acc
            })
            .join();
            term.writeline(&format!("spawnjoin -> {sum}"));
        }
        "heir" => {
            // 血缘枚举：我生的子域（heir）——先 count 再逐个取 TeamId。
            let n = heir_count().unwrap_or(0);
            if n == 0 {
                term.writeline("heir: none");
            } else {
                term.writeline(&format!("heir: {n} children"));
                for i in 0..n {
                    let tid = heir_at(i).unwrap_or(env::TeamId::new(0));
                    term.writeline(&format!("  heir[{i}] = team {tid:?}"));
                }
            }
        }
        "hole" => {
            let msg = b"hi from shell";
            let pie = HolePie::unseal(HOLE_MTU_MAX).unwrap();
            let mut buf = [0u8; 64];
            let mut m = [0u8; 64];
            m[..msg.len()].copy_from_slice(msg);
            pie.push(&m).ok();
            pie.pull(&mut buf).ok();
            term.writeline(&format!(
                "hole got {:?}",
                core::str::from_utf8(&buf).unwrap_or("?")
            ));
            // ── 封印唤醒探针（**只加观测量**：上面那句与本命令既有语义逐字未动）──
            // 既有自测走到了 `seal`（`wipe` 的 hole 调用方），却没有任何断言说
            // 「seal 释放了等待者」。下面在同一轮里以 seal 为界各等一次：**封印前**
            // 等满期限、**封印后**当场就绪——两个结论都被打印出来，门断言
            // `hole: wait-seal sealed=1 wake=seal`（只被 seal 唤醒才可能打出的那一支）。
            seal_wake_probe(term);
            pie.seal().ok();
        }
        "cascade" => {
            cascade(term);
        }
        "churn" => {
            // 任务生灭压测：`churn [rounds] [depth] [fan] [rest]`（默认 200 × 2 × 2）。
            // `rest` = 同一启动内**连跑 rest 遍**，每遍之间水位由内核探针打点。
            // 为什么要连跑：判「漏」的唯一干净问题是**同一状态下重复同一操作，
            // 静息水位是否上台阶**。跑一遍只能看到"涨了"，分不清是稳态占用还是
            // 每遍新漏的——连跑两遍的差值就把这两者分开了。
            // `args` 已去掉命令词本身（见 `main`：`split` 后传的是 `&args[1..]`），故从 0 起。
            let n = args.first().and_then(|s| s.parse().ok()).unwrap_or(200);
            let d = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2);
            let f = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2);
            let rest = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
            for pass in 1..=rest {
                // ── 静息水位：此刻**没有任何任务在飞**（上一遍已全部收尾）──
                //
                // 这是判"漏"唯一干净的采样点。在 churn **跑的过程中**采样读到的
                // `held` 混着"当时活着的任务占的栈/页表"，那部分随并发波动，与
                // 是否泄漏无关——我先前就是拿运行中的采样当"静息水位"，把并发占用
                // 误读成了每遍新增的漏量。
                //
                // 同时取**逐类在册帧数**：只盯池总量，漏哪一类都长一个样；逐类看
                // 才能直接指到是 `table` 还是 `stack` 在涨。
                let before = idle_kinds(term, "before");
                churn(term, n, d, f);
                let after = idle_kinds(term, "after");
                term.writeline(&format!(
                    "churn pass {pass}/{rest} idle Δheld {} Δwalk {}",
                    after.0 as i64 - before.0 as i64,
                    after.1 as i64 - before.1 as i64,
                ));
            }
        }
        "carve" => {
            // **最小自证**：证明"随后的页分配是从一个大空闲块里切出来的"，
            // 即 `split_block` 会消费一块物理连续的大块。
            //
            // 判据：分配 N 页后，**最大相邻连续段**（按地址排序后跑一遍）若远大于
            // 单页，则这些页来自同一大块的不同位置 —— 与"分配器总从固定基址首次
            // 适配切出"一致。这解释了为何一次释放能把大块丢进幽灵态（§ 诊断文档）。
            let n = args.first().and_then(|s| s.parse().ok()).unwrap_or(64usize);
            let mut a: Vec<usize> = Vec::new();
            for _ in 0..n {
                match runtime::env::memory::allocate(4096) {
                    Ok(x) => a.push(x),
                    Err(_) => break,
                }
            }
            a.sort_unstable();
            let (mut best, mut run) = (1usize, 1usize);
            for w in a.windows(2) {
                if w[1] == w[0] + 4096 {
                    run += 1;
                    best = best.max(run);
                } else {
                    run = 1;
                }
            }
            let lo = a.first().copied().unwrap_or(0);
            let hi = a.last().copied().unwrap_or(0);
            term.writeline(&format!(
                "carve n={} lo={lo:#x} hi={hi:#x} span={:#x} max_run={best}",
                a.len(),
                hi.saturating_sub(lo)
            ));
            for x in a.drain(..) {
                let _ = runtime::env::memory::deallocate(x, 4096);
            }
        }
        "pagedrain" => {
            // **只做 order-0 分配的抽取实验**（默认 4000 页）。
            //
            // 目的：`walk` 塌陷是否与 `split_block`（拆分大块）有关。本命令只逐页
            // `allocate(4096)` 并一直持有，**从不请求大块** ⇒ 走不到多级拆分；再逐页
            // 释放。若 `walk` 照样塌，则病灶与拆分无关；若 `walk` 保持，则病在拆分。
            let n = args.first().and_then(|s| s.parse().ok()).unwrap_or(4000usize);
            let mut addrs: Vec<usize> = Vec::new();
            let (h0, w0) = runtime::env::memory::watermark().unwrap_or((0, 0));
            // `live` = 累计分配 − 累计释放（帧数）。它若每轮精确回到基线，说明
            // **每一页都走到了 `deallocate`**；那丢失就发生在 `deallocate` 内部。
            let l0 = runtime::env::memory::live_frames().unwrap_or(0);
            let mut fails = 0usize;
            for _ in 0..n {
                match runtime::env::memory::allocate(4096) {
                    Ok(a) => addrs.push(a),
                    Err(_) => {
                        fails += 1;
                        break;
                    }
                }
            }
            let (h1, w1) = runtime::env::memory::watermark().unwrap_or((0, 0));
            term.writeline(&format!(
                "pagedrain held {} pages fails={fails} | held {h0}->{h1} walk {w0}->{w1}",
                addrs.len()
            ));
            // 逐页释放，并记录每一页释放后 `walk` 的**增量**。
            //
            // 判据：一次 `free` 之后 `walk` 应当增加该块的大小（合并还会更多）。
            // 某一步增量为 0 或异常 ⇒ **丢帧就发生在这一步**，比猜机制直接。
            // 释放**顺序**实验（判据仍是同一个：`walk` 净变化）：
            //   · `fwd`（默认）升序；`rev` 降序；`even` 只放偶数下标（留洞）。
            // 若 `rev` 能让 `walk` 恢复 ⇒ 合并依赖释放顺序；若都漏 ⇒ 是块状态问题。
            let mode = args.get(1).cloned().unwrap_or_else(|| "fwd".into());
            let trace = mode == "trace";
            if mode == "rev" {
                addrs.reverse();
            } else if mode == "even" {
                let evens: Vec<usize> = addrs.iter().step_by(2).copied().collect();
                addrs = evens;
            }
            let mut zero_steps = 0usize;
            let mut shown = 0usize;
            let mut sum_delta: i64 = 0;
            let mut wprev = w1;
            for (i, a) in addrs.drain(..).enumerate() {
                let _ = runtime::env::memory::deallocate(a, 4096);
                let (_, wn) = runtime::env::memory::watermark().unwrap_or((0, 0));
                let d = wn as i64 - wprev as i64;
                sum_delta += d;
                if d <= 0 {
                    zero_steps += 1;
                }
                if trace && shown < 40 {
                    term.writeline(&format!("  free#{i} {a:#x} walk_delta={d} walk={wn}"));
                    shown += 1;
                }
                wprev = wn;
            }
            let (mb, mm, mc, mo) = runtime::env::memory::merge_census().unwrap_or((0, 0, 0, 0));
            term.writeline(&format!(
                "pagedrain deltas sum={sum_delta} zero_or_neg={zero_steps} | merge ok={mo} 拒: bound={mb} meta={mm} chain={mc}"
            ));
            // 拒绝的**逐 order 分布**：看主因（chain 拒）集中在哪个 power。
            let mut line = alloc::string::String::new();
            for p in 0..8usize {
                let (rm, rc) = runtime::env::memory::reject_by_power(p).unwrap_or((0, 0));
                let _ = core::fmt::Write::write_fmt(&mut line, format_args!(" p{p}:m{rm}/c{rc}"));
            }
            term.writeline(&format!("pagedrain reject by power{line}"));
            let (h2, w2) = runtime::env::memory::watermark().unwrap_or((0, 0));
            let l2 = runtime::env::memory::live_frames().unwrap_or(0);
            // 逐 order 的 `pagemeta空闲块首/链上块数`：看那几百帧是从哪个 order 掉的。
            let mut kbuf = [0u8; 512];
            let _ = runtime::env::memory::watermark_kinds(&mut kbuf);
            let kend = kbuf.iter().position(|b| *b == 0).unwrap_or(kbuf.len());
            let kinds = core::str::from_utf8(&kbuf[..kend]).unwrap_or("");
            term.writeline(&format!(
                "pagedrain freed | held ->{h2} walk ->{w2} live {l0}->{l2} | {kinds}"
            ));
        }
        "bigalloc" => {
            // **行为判据**：一次性请求 N 页（默认 2048 页 = 8 MiB）。
            //
            // 用途：`walk`（freelist 走链）与 `meta`（pagemeta 求和）长期背离，
            // 一个说池子空了、一个说还有一万多帧。读数之间争不出结果，就用**行为**
            // 定论：静息时刻请求一大块 —— 成功 ⇒ `walk` 在少算、池子还有内存；
            // 失败 ⇒ 池子真的空了。
            let n = args.first().and_then(|s| s.parse().ok()).unwrap_or(2048usize);
            let bytes = n * 4096;
            let (held0, walk0) = runtime::env::memory::watermark().unwrap_or((0, 0));
            let r = runtime::env::memory::allocate(bytes);
            let (held1, walk1) = runtime::env::memory::watermark().unwrap_or((0, 0));
            match r {
                Ok(a) => {
                    term.writeline(&format!(
                        "bigalloc {n}p OK at {a:#x} | held {held0}->{held1} walk {walk0}->{walk1}"
                    ));
                    let _ = runtime::env::memory::deallocate(a, bytes);
                    let (held2, walk2) =
                        runtime::env::memory::watermark().unwrap_or((0, 0));
                    term.writeline(&format!(
                        "bigalloc freed | held ->{held2} walk ->{walk2}"
                    ));
                }
                Err(e) => term.writeline(&format!(
                    "bigalloc {n}p FAILED {e:?} | held={held0} walk={walk0}"
                )),
            }
        }
        "calib" => {
            // **闭环校准**：先证明读数可信，再拿它量泄漏。
            let n = args.first().and_then(|s| s.parse().ok()).unwrap_or(200);
            calib(term, n);
        }
        "reclaim" => {
            reclaim(term);
        }
        "spoof" => {
            spoof(term);
        }
        "name" => {
            name(term);
        }
        "badslot" => {
            // 非法 envcall 槽位（`a7` 由 U 态完全控制）：空号 class 1 idx 5 +
            // **未分配**的 class 8 + 越界。判据 = 内核把调用号当**参数**拒掉
            // （a0 回负码）并续跑本任务，而不是 panic 打死整机。此处不走
            // `EnvCall` 解码（那正是要绕过的正常路径），直入 ABI 的唯一汇编入口
            // `trap`。
            let mut neg = 0usize;
            for slot in [0x1_0000_0005usize, 0x8_0000_0000, usize::MAX] {
                // SAFETY: 照 ABI 摆 slot + 6 参数；本调用只探错误路径，不依赖返回语义。
                let (a0, _a1) = unsafe { env::ecall::trap(slot, [0; 6]) };
                if (a0 as isize) < 0 {
                    neg += 1;
                }
            }
            term.writeline(&format!("badslot: {neg}/3 rejected, kernel alive"));

            // ── 对照：**带原因码退场**的任务同样被内核收干净（同一条命令，紧接上面三发）──
            //
            // 上面三发是**非法**调用被拒掉（内核把调用号当参数拒掉、续跑调用方）；这一发
            // 是**正常受理**的退场 `RoomCall::Reap { reason }`——同一枚原语，只是带上原因码。
            // 两者对内核的要求是同一条：**调用方按各自语义了结、内核活**。
            //
            // 为什么用**一次性子任务**当靶子：`Reap` 是发散调用（退场即不返回），挨刀的
            // 只能是发起调用的那个任务。子任务正是本仓反复用的"可弃靶子"（`cascade` /
            // `reclaim` 同款），而它死了之后本任务还能打印结论——"让 shell 自己退场"的
            // 任何写法都做不到（那一支连结论都打不出来）。
            //
            // 判据（`Join` 返真 ⇔ 退出钩子已跑完）：内核受理 ⇒ 子任务当场被收掉、内核活；
            // 内核若把"域级退场"实现成打死整机（本仓一度如此）⇒ 本行与这台机器一起没了。
            match unit::try_closure(|| runtime::env::room::exit_with(0x5A5A)) {
                Ok(j) => {
                    let tid = j.id();
                    // 子任务**不释放盒子**（退场路径不经 join 的结果回收），本侧丢。
                    drop(j);
                    // **按终态等，不按钟等**：判据是「退出钩子已跑完」，而 `Join` 的
                    // 契约正是「返真 ⇔ 已到终态」——本仓既有 [`join_done`] 就照这个写
                    // （探一发 `0`，未到终态就无条件等）。本探针第一版拿 `0` 与 `500ms`
                    // 两种"看一眼"的形态去问，两次都误报 FAIL：那一刻退场已受理、终态
                    // 未落地（同一条教训换了副面孔又来一次——**等状态，别等钟**）。
                    join_done(tid);
                    term.writeline("badslot: 1/1 abnormal exit reaped, kernel alive");
                }
                Err(_) => term.writeline("badslot: spawn failed"),
            }
        }
        "req" => {
            // 走目录协议：Directory::open 取会话 → Connect("echo") 拿服务入口门闩
            // → Service::call 一次往返（echo 对载荷字节 +1）。
            let dir = match dir_session() {
                Ok(d) => d,
                Err(e) => {
                    term.writeline(&format!("req dir err: {e:?}"));
                    return true;
                }
            };
            let svc = match dir.connect("echo") {
                Ok(s) => s,
                Err(e) => {
                    term.writeline(&format!("req connect err: {e:?}"));
                    return true;
                }
            };
            let txt = args.first().map(|s| s.as_str()).unwrap_or("hello-service");
            let mut payload = [0u8; PAYLOAD_LEN];
            let bytes = txt.as_bytes();
            let n = bytes.len().min(PAYLOAD_LEN - 1);
            payload[..n].copy_from_slice(&bytes[..n]);
            match svc.call(&payload) {
                Ok(got) => {
                    term.writeline(&format!(
                        "req echo -> {:?}",
                        core::str::from_utf8(&got).unwrap_or("?")
                    ));
                }
                Err(e) => {
                    term.writeline(&format!("req echo err: {e:?}"));
                }
            }
            let _ = svc.disconnect();
        }
        "dir" => {
            // 目录协议：Discover（纯探测）+ Enumerate（按名排序分页）。
            let dir = match dir_session() {
                Ok(d) => d,
                Err(e) => {
                    term.writeline(&format!("dir open err: {e:?}"));
                    return true;
                }
            };
            match dir.discover("echo") {
                Ok(true) => term.writeline("discover echo -> found"),
                Ok(false) => term.writeline("discover echo -> not found"),
                Err(e) => term.writeline(&format!("dir discover err: {e:?}")),
            }
            let mut after: Option<String> = None;
            loop {
                match dir.list(after.as_deref()) {
                    Ok(Some(name)) => {
                        term.writeline(&format!("  {}", name.as_str()));
                        after = Some(String::from(name.as_str()));
                    }
                    Ok(None) => break,
                    Err(e) => {
                        term.writeline(&format!("dir list err: {e:?}"));
                        break;
                    }
                }
            }
        }
        "stray" => {
            stray_probe(term);
        }
        "kill" => {
            kill_cmd(args.first().map(|s| s.as_str()), term);
        }
        "line" => {
            line_probe(args.first().map(|s| s.as_str()), term);
        }
        "exit" => {
            term.writeline("bye");
            return false;
        }
        _ => {
            term.writeline(&format!("unknown: {cmd} (try help)"));
        }
    }
    true
}

#[unsafe(no_mangle)]
extern "C" fn main() {
    // 启动期握手：拿到目录请求门闩的句柄（身份由 Owned 从门闩自身求得）。
    let dir_token = match shake() {
        Ok(t) => t,
        Err(_) => runtime::env::room::exit_with(1),
    };
    DIR_ENTRY.store(dir_token.get(), Ordering::Relaxed);

    let term = &TERM;
    term.clear();
    term.fg(Color::Green);
    term.writeline("SQware shell");
    term.reset();
    term.writeline("type 'help' for commands.");

    loop {
        term.fg(Color::Cyan);
        // readline 收纳 prompt：Terminal 内部打 prompt + 行编辑 + 清行重绘含 prompt。
        let line = match term.readline("sq > ") {
            Readline::Line(s) => s,
            Readline::Eof | Readline::Interrupt => {
                term.reset();
                continue;
            }
        };
        term.reset();
        let args = split(&line);
        if args.is_empty() {
            continue;
        }
        let cmd = args[0].clone();
        let rest = &args[1..];
        if !exec(&cmd, rest, term) {
            break;
        }
    }

    room::exit()
}
