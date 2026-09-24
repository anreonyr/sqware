//! 门 —— 本仓所有**要跑**的判据的落点。
//!
//! 台有两台，**一个契约**：吃一台、吐一份 [`Transcript`]、**起不动**才 `Err`。
//!
//! - **机器那台**（[`Bench::Machine`]）：起 QEMU 走一趟。QEMU 参数的**唯一出处**仍是
//!   `scripts/boot.nu`——这里只 spawn 它、喂 stdin、按行收两条流、到点收尾。
//! - **宿主那台**（[`Bench::Host`]）：在**宿主 target** 上跑一次 `cargo test`（那条
//!   `--target` 不写在门里：它是"宿主"这两个字的落点）。
//!
//! **被杀与内核 panic 不是 `Err`**：它们是 [`Transcript`] 里的事实，由门去判。
//!
//! # 照实记（两条流怎么合）
//!
//! 两台子各起两个读线程，**按行**把 stdout 与 stderr 添进同一个缓冲——不是"读完整份再拼"。
//! 这样既保住"一行就是一行"（整行形状的断言靠它），又不丢先后（今天的日志是 `2>&1` 出来的）。
//!
//! # 照实记（期限归谁）
//!
//! 机器那台**不自己杀 QEMU**：把期限交给 `boot.nu`（它用 `timeout` 管着自己那个子进程——这是
//! 今天验过、唯一真的杀得掉 QEMU 的机制）。Rust 这一侧只留一个**兜底**：期限 + 10 秒还没结束
//! 就 `kill` 掉 `nu`。`timeout` 杀出来那一条会变成退出码 124，与今天的读数同形。
//!
//! # 照实记（`QEMU_ICOUNT` 置空）
//!
//! `boot.nu` 的默认是 `-icount auto,sleep=on`：按宿主时间给 vCPU 记账 ⇒ WFI 里的核被 IPI 叫醒
//! 要等额度（实测毫秒级）。验收门一直是关着跑的，故这里**统一置空**——两台子上的读数才可比。

use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub mod mutations;
pub mod soak;

/// 宿主那一档的 target 名字。门里不写它——"宿主"这两个字的落点就在这一句。
pub const HOST: &str = "x86_64-unknown-linux-gnu";

/// 仓根（本 crate 在 `<根>/crates/gate`）。
pub(crate) fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/gate 的上一级就是仓根")
        .to_path_buf()
}

pub fn secs(n: u64) -> Deadline {
    Deadline(Duration::from_secs(n))
}

/// 一次跑的期限。
#[derive(Clone, Copy, Debug)]
pub struct Deadline(Duration);

impl Deadline {
    fn get(self) -> Duration {
        self.0
    }
}

/// 装配单（`SQWARE_ROOT`）：换的是**引导镜像**，由此换掉整张表。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    Root,
    Fair,
    Rig,
    Load,
    Group,
}

impl Scenario {
    fn name(self) -> &'static str {
        match self {
            Scenario::Root => "root",
            Scenario::Fair => "fair",
            Scenario::Rig => "rig",
            Scenario::Load => "load",
            Scenario::Group => "group",
        }
    }
}

/// 档位。**profile 与 feature 一次给全**：`framework` 那一档少了 `--features framework`
/// 就把用例整个编掉（"没有门的档 = 没有编译过的档"）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Debug,
    Release,
    Framework,
}

impl Profile {
    fn args(self) -> &'static [&'static str] {
        match self {
            Profile::Debug => &[],
            Profile::Release => &["--release"],
            Profile::Framework => &["--profile", "framework", "--features", "framework"],
        }
    }

    fn dir(self) -> &'static str {
        match self {
            Profile::Debug => "debug",
            Profile::Release => "release",
            Profile::Framework => "framework",
        }
    }
}

/// 一颗造好的、等着被跑的引导 ELF。**只有 [`build`] 造得出**（字段私有）。
#[derive(Clone, Debug)]
pub struct Image {
    elf: PathBuf,
}

impl Image {
    pub fn elf(&self) -> &Path {
        &self.elf
    }

    /// `initrd.img` 由 elf **同目录**推导——`boot.nu` 就是这么找的，故这里不存第二条路径。
    pub fn initrd(&self) -> PathBuf {
        self.elf.with_file_name("initrd.img")
    }
}

static BUILT: Mutex<Vec<((Scenario, Profile), Image)>> = Mutex::new(Vec::new());

/// 造那颗要跑的。同一 `(Scenario, Profile)` 在一次测试进程里**只真构一次**。
pub fn build(scenario: Scenario, profile: Profile) -> Result<Image, BuildFailed> {
    let done = BUILT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, image)) = done.iter().find(|(k, _)| *k == (scenario, profile)) {
        return Ok(image.clone());
    }
    drop(done);
    rebuild(scenario, profile)
}

/// **绕开缓存**重造那颗，并顺手刷新缓存。
///
/// 变异那一门要的就是这个：改一处源码之后，磁盘上那颗必须是新的；而跑完还原之后，缓存里也
/// 不该留着改过的那一颗——否则同一个进程里后面的门会拿回变异过的 ELF。
pub(crate) fn rebuild(scenario: Scenario, profile: Profile) -> Result<Image, BuildFailed> {
    let root = root();
    let out = Command::new("cargo")
        .args(["build", "--manifest-path"])
        .arg(root.join("Cargo.toml"))
        .args(profile.args())
        .env("SQWARE_ROOT", scenario.name())
        .current_dir(&root)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| BuildFailed::Spawn(e.to_string()))?;

    let elf = root
        .join("target/riscv64gc-unknown-none-elf")
        .join(profile.dir())
        .join("sqware");

    if !out.status.success() {
        return Err(BuildFailed::Cargo {
            scenario,
            profile,
            code: out.status.code(),
            tail: tail_of(&String::from_utf8_lossy(&out.stderr), 12),
        });
    }
    if !elf.exists() {
        return Err(BuildFailed::NoElf(elf));
    }

    let image = Image { elf };
    let mut done = BUILT.lock().unwrap_or_else(|e| e.into_inner());
    done.retain(|(k, _)| *k != (scenario, profile));
    done.push(((scenario, profile), image.clone()));
    Ok(image)
}

#[derive(Debug)]
pub enum BuildFailed {
    Spawn(String),
    Cargo {
        scenario: Scenario,
        profile: Profile,
        code: Option<i32>,
        tail: String,
    },
    NoElf(PathBuf),
}

impl fmt::Display for BuildFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildFailed::Spawn(e) => write!(f, "cargo 起不动：{e}"),
            BuildFailed::Cargo {
                scenario,
                profile,
                code,
                tail,
            } => write!(
                f,
                "构建失败（{:?}/{:?}，退出码 {:?}）—— 末尾：\n{}",
                scenario, profile, code, tail
            ),
            BuildFailed::NoElf(p) => write!(f, "构建过，但 ELF 不在：{}", p.display()),
        }
    }
}

impl std::error::Error for BuildFailed {}

/// 喂键的**日程**（数据，不是原语）：三件喂键操作全落在这一格里。
pub enum Schedule {
    /// 定时重复喂（验收门那一支：等启动 → 重复喂一把 → 留够关机的时间）。
    Clocked(Vec<(Duration, String)>),
    /// 等标记再喂：招牌出现就喂 `then`；**上限到了照样喂**（今天就是这个口径）。
    OnMark {
        mark: &'static str,
        within: Duration,
        then: Vec<String>,
    },
    /// 按参数喂。
    Immediate(Vec<String>),
    /// 不喂（纯看）。
    None,
}

/// 一台要走的台子。
///
/// **照实记（`env` 那一格是落签名时才长出来的）**：忙机台那条债的开关**就是核数**
/// （`QEMU_SMP=1` 是机制隔离档、`=4` 是对照档，见 `scripts/load.sh` 旧头注），而它不是
/// `Scenario` 能表达的（同一张装配单要能跑两档）。故这一格不是"再想一个字段"，而是把
/// `boot.nu` **已经有的那套旋钮**原样透给它——起法仍是唯一出处，这里只转发。
pub enum Bench {
    Machine {
        image: Image,
        sched: Schedule,
        within: Deadline,
        /// 透给 `boot.nu` 的旋钮（`QEMU_SMP` / `QEMU_MEM` / …），按名传。
        env: &'static [(&'static str, &'static str)],
    },
    Host {
        /// `--manifest-path` 那一个（宿主那一档只有一个靶场时也只写一个）。
        manifest: &'static str,
        within: Deadline,
    },
}

/// 一次跑的全部读数。
pub struct Transcript {
    text: String,
    outcome: Outcome,
    code: Option<i32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// 自己结束（退出码看 [`Transcript::code`]）。
    Exited,
    /// 没自退：被期限收掉的。
    Killed,
    /// 读数里有内核 panic 报告头。**先于另两格判**——panic 若走 halt 自旋，必然被超时杀，
    /// 根因是 panic（照 `runner.nu` 的照实记）。
    Panicked,
}

impl Transcript {
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    pub fn code(&self) -> Option<i32> {
        self.code
    }

    /// 整份读数里有没有这一串（子串，不是整行）。
    pub fn has(&self, needle: &str) -> bool {
        self.text.contains(needle)
    }

    /// 现场留下来（今天那些 `target/<门>/…` 的落点）。
    pub fn keep(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, &self.text)
    }
}

/// 走一趟。**两台子同一条路**：起、喂、收、到点收尾；只有"起不动"才是 `Err`。
///
/// **照实记（机器只有一台）**：这条路上有一把进程内的锁——门可以并行跑，**台不能**。一台机器
/// 就是 4 核 + 256 MB，两门同跑既会互相抢核（`timer:` 那几格是时序读数），也会让"读数不可比"
/// 这件事换个地方复活。锁在机器那一支上，故几门一起 `--include-ignored` 也是排着走的。
/// 中毒的锁照用（`into_inner`）：一台子出过事不该把后面的门全锁死。
pub fn run(bench: &Bench) -> Result<Transcript, BenchFailed> {
    static MACHINE: Mutex<()> = Mutex::new(());

    let root = root();
    let mut _machine = None;
    let (mut child, within) = match bench {
        Bench::Machine {
            image,
            within,
            env,
            ..
        } => {
            _machine = Some(MACHINE.lock().unwrap_or_else(|e| e.into_inner()));
            if !image.elf.exists() {
                return Err(BenchFailed::NoElf(image.elf.clone()));
            }
            let boot = root.join("scripts/boot.nu");
            if !boot.exists() {
                return Err(BenchFailed::NoBoot(boot));
            }
            let child = Command::new("nu")
                .arg(&boot)
                .arg(&image.elf)
                .env("QEMU_ICOUNT", "")
                .env("QEMU_TIMEOUT", within.get().as_secs().to_string())
                .envs(env.iter().copied())
                .current_dir(&root)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| BenchFailed::Spawn(format!("nu scripts/boot.nu：{e}")))?;
            (child, *within)
        }
        Bench::Host { manifest, within } => {
            let child = Command::new("cargo")
                .args(["test", "--manifest-path", manifest, "--target", HOST, "--tests"])
                .current_dir(&root)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| BenchFailed::Spawn(format!("cargo test：{e}")))?;
            (child, *within)
        }
    };

    let text = Arc::new(Mutex::new(String::new()));
    let readers = vec![
        read_lines(child.stdout.take(), Arc::clone(&text)),
        read_lines(child.stderr.take(), Arc::clone(&text)),
    ];

    // **写端攥到收尾那一步才放**：日程走完不等于可以把 stdin 关掉——今天的 `sleep 12`
    // 尾巴要的就是"别在 guest 收尾之前关掉写端"。这里不用魔法秒数，靠作用域。
    let mut stdin = child.stdin.take();
    if let Bench::Machine { sched, .. } = bench {
        drive(sched, &mut stdin, &text);
    }

    let started = Instant::now();
    let backstop = within.get() + Duration::from_secs(10);
    let mut killed = false;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) => {
                if started.elapsed() >= backstop {
                    let _ = child.kill();
                    let _ = child.wait();
                    killed = true;
                    break None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(BenchFailed::Wait(e.to_string())),
        }
    };
    drop(stdin);
    for r in readers {
        let _ = r.join();
    }

    let text = text.lock().unwrap().clone();
    let outcome = if text.contains("[panic] at") {
        Outcome::Panicked
    } else if killed || code == Some(124) {
        Outcome::Killed
    } else {
        Outcome::Exited
    };
    Ok(Transcript {
        text,
        outcome,
        code,
    })
}

fn drive(sched: &Schedule, stdin: &mut Option<std::process::ChildStdin>, text: &Arc<Mutex<String>>) {
    let Some(pipe) = stdin.as_mut() else { return };
    let started = Instant::now();
    match sched {
        Schedule::None => {}
        Schedule::Immediate(cmds) => {
            for c in cmds {
                let _ = writeln!(pipe, "{c}");
            }
            let _ = pipe.flush();
        }
        Schedule::Clocked(entries) => {
            for (at, line) in entries {
                let left = at.saturating_sub(started.elapsed());
                std::thread::sleep(left);
                let _ = writeln!(pipe, "{line}");
                let _ = pipe.flush();
            }
        }
        Schedule::OnMark { mark, within, then } => {
            let cap = started + *within;
            while Instant::now() < cap && !text.lock().unwrap().contains(mark) {
                std::thread::sleep(Duration::from_millis(20));
            }
            for line in then {
                let _ = writeln!(pipe, "{line}");
                let _ = pipe.flush();
                // 每喂一条留一口气：今天那两次 `exit` 之间那个 `sleep 3` 要的就是这个
                // （第一条往往已经收尾，第二条是保险）。
                std::thread::sleep(Duration::from_millis(300));
            }
        }
    }
}

fn read_lines(r: Option<impl Read + Send + 'static>, into: Arc<Mutex<String>>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let Some(r) = r else { return };
        let mut reader = BufReader::new(r);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => into.lock().unwrap().push_str(&line),
            }
        }
    })
}

fn tail_of(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[derive(Debug)]
pub enum BenchFailed {
    NoElf(PathBuf),
    NoBoot(PathBuf),
    Spawn(String),
    Wait(String),
}

impl fmt::Display for BenchFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BenchFailed::NoElf(p) => write!(f, "ELF 不在（先 `build`）：{}", p.display()),
            BenchFailed::NoBoot(p) => write!(f, "起法不在：{}", p.display()),
            BenchFailed::Spawn(e) => write!(f, "起不动：{e}"),
            BenchFailed::Wait(e) => write!(f, "等不动：{e}"),
        }
    }
}

impl std::error::Error for BenchFailed {}

/// 门的现场落点：`target/gate/<门>-<秒>.log`。
pub fn log_path(door: &str) -> PathBuf {
    root().join(format!("target/gate/{door}-{}.log", stamp()))
}

/// 门的现场**目录**：`trace/<门>-<秒>/`（要逐轮留一份的那几门）。
pub fn scene_dir(door: &str) -> PathBuf {
    root().join(format!("trace/{door}-{}", stamp()))
}

fn stamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ═══ 判据（全是吃 `&Transcript` 的纯函数）═══════════════════════════════════════

/// 一条断言。三种形状就是原先 `soak.sh` 里那三行（`need` / `needE` / `need_absent`）。
///
/// **照实记（语义由类型分，不再由"哪个 grep"分）**：今天这三样靠 `grep -q` 与 `grep -qE`
/// 的差别扛着，而声明式对账器正是栽在那一格上（它把两者都当 ERE 判 ⇒ 22 条带括号的断言
/// 在它那里永远不命中、把"判了"记成"没人判"）。这里 `Literal` 是**子串**、`Shape` 是**ERE**，
/// 由类型说了算；行尾的 `\r` 在收的时候就统一剥掉了。
pub enum Mark {
    Literal(&'static str),
    Shape(&'static str),
    Absent(&'static str),
}

impl Mark {
    fn judges(&self) -> bool {
        !matches!(self, Mark::Absent(_))
    }

    fn raw_hits(&self, line: &str) -> bool {
        match self {
            Mark::Literal(s) | Mark::Absent(s) => line.contains(s),
            Mark::Shape(p) => ere(p).is_match(line),
        }
    }

    /// 这一条在这一次跑里**兑现了没有**。`Absent` 是"本不该出现" ⇒ 出现了就是不兑现。
    pub fn holds(&self, t: &Transcript) -> bool {
        let any = t
            .text()
            .lines()
            .any(|l| self.raw_hits(l.trim_end_matches('\r')));
        match self {
            Mark::Absent(_) => !any,
            _ => any,
        }
    }

    /// 报缺口时那一行（`Absent` 前面加个 `!`，与旧的 `missing` 同形）。
    pub fn describe(&self) -> String {
        match self {
            Mark::Literal(s) | Mark::Shape(s) => (*s).to_string(),
            Mark::Absent(s) => format!("! {s}"),
        }
    }
}

/// 读数表的一行：一个前缀，以及它属于哪一档。
pub struct Reading {
    pub prefix: &'static str,
    pub tier: Tier,
}

/// **这一档就是那句纪律**：`Auto` 逐行都要有人判；`Narrative` 打了、但只判其中几条形状，
/// **没判的那几行必须报出形状**（棘轮）；`Manual` 的判据在断言表外面，写理由。
///
/// "narrative 却没写形状"这个状态**不可表达**——原先它是运行时判的（而且判错过一次：
/// 白名单取回空串 ⇒ `$0 ~ ""` 匹配一切 ⇒ 判绿）。
pub enum Tier {
    Auto,
    Narrative { shapes: &'static [&'static str] },
    Manual { why: &'static str },
}

/// 兑不上的一条读数：**要什么** / **看到什么**。今天每一门报的就是这两半。
#[derive(Debug)]
pub struct Gap {
    pub want: String,
    pub saw: String,
}

impl fmt::Display for Gap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "要 {}；看到 {}", self.want, self.saw)
    }
}

impl std::error::Error for Gap {}

/// 对着读数表兑现一次跑，**一次报全部缺口**（不是第一条就返回——今天是"缺这几条"一起报）。
pub fn hold(t: &Transcript, marks: &[Mark], table: &[Reading]) -> Result<(), Vec<Gap>> {
    struct Tally<'a> {
        prefix: &'a str,
        ok: usize,
        bad: usize,
        stray: usize,
        undeclared: bool,
        first_bad: Option<String>,
        stranger: Option<String>,
    }

    let mut tallies: Vec<Tally> = Vec::new();
    for raw in t.text().lines() {
        let line = raw.trim_end_matches('\r');
        let Some(p) = prefix_of(line) else { continue };
        if is_noise(p) {
            continue;
        }
        let idx = match tallies.iter().position(|x| x.prefix == p) {
            Some(i) => i,
            None => {
                tallies.push(Tally {
                    prefix: p,
                    ok: 0,
                    bad: 0,
                    stray: 0,
                    undeclared: table.iter().all(|r| r.prefix != p),
                    first_bad: None,
                    stranger: None,
                });
                tallies.len() - 1
            }
        };
        let e = &mut tallies[idx];
        if e.undeclared {
            continue;
        }
        if marks.iter().any(|m| m.judges() && m.raw_hits(line)) {
            e.ok += 1;
            continue;
        }
        e.bad += 1;
        if e.first_bad.is_none() {
            e.first_bad = Some(line.to_string());
        }
        let tier = &table.iter().find(|r| r.prefix == p).unwrap().tier;
        if let Tier::Narrative { shapes } = tier
            && !shapes.iter().any(|s| ere(s).is_match(line))
        {
            e.stray += 1;
            if e.stranger.is_none() {
                e.stranger = Some(line.to_string());
            }
        }
    }

    let mut gaps = Vec::new();
    for e in &tallies {
        let sample = |o: &Option<String>| o.clone().unwrap_or_else(|| "—".to_string());
        if e.undeclared {
            gaps.push(Gap {
                want: format!("前缀 `{}:` 没在读数表里声明（新读数没人管）", e.prefix),
                saw: sample(&e.first_bad),
            });
            continue;
        }
        let tier = &table.iter().find(|r| r.prefix == e.prefix).unwrap().tier;
        match tier {
            Tier::Auto if e.bad > 0 => gaps.push(Gap {
                want: format!("`{}:` 每一行都要被判（auto 档逐行）", e.prefix),
                saw: format!("{} 行没人判 —— 例：{}", e.bad, sample(&e.first_bad)),
            }),
            Tier::Narrative { .. } => {
                if e.ok == 0 {
                    gaps.push(Gap {
                        want: format!("`{}:` 声明为 narrative，至少要有一条被判", e.prefix),
                        saw: sample(&e.first_bad),
                    });
                }
                if e.stray > 0 {
                    gaps.push(Gap {
                        want: format!("`{}:` 没判的行要落在声明的形状里（棘轮）", e.prefix),
                        saw: format!("{} 行出格 —— 例：{}", e.stray, sample(&e.stranger)),
                    });
                }
            }
            _ => {}
        }
    }
    if gaps.is_empty() { Ok(()) } else { Err(gaps) }
}

/// 同一形状的每一行**按序**交出（第一个捕获组）。
///
/// 关系本身留在门里：`me[0] != me[1] && me[0] == me[2]`（绑 ≠ 领 = 弃 是 policy 那一格的
/// 知识，做成通用原语就得造一个只有一格的关系枚举——那是拿原语装门规）。
pub fn values<'a>(t: &'a Transcript, shape: &str) -> Result<Vec<&'a str>, Gap> {
    let re = ere(shape);
    let mut out = Vec::new();
    for raw in t.text().lines() {
        let line = raw.trim_end_matches('\r');
        let Some(c) = re.captures(line) else { continue };
        let Some(g) = c.get(1) else {
            return Err(Gap {
                want: format!("形状 `{shape}` 要有一个捕获组"),
                saw: line.to_string(),
            });
        };
        out.push(g.as_str());
    }
    if out.is_empty() {
        Err(Gap {
            want: format!("形状 `{shape}` 至少要有一行"),
            saw: "一行都没有".to_string(),
        })
    } else {
        Ok(out)
    }
}

/// `[case]`：每个 `run` 都要有配对的 `ok`；不配对时**点名**跑了一半的那一例。
///
/// 这条判据是给"用例只报一次失败"那个形状用的：`assert!` 走 panic 通道，域当场死 ⇒ 失败那一例
/// **只留下 `run`**，故"最后一条 `run`"就是它的名字。
pub fn pair(t: &Transcript) -> Result<(), Gap> {
    let runs: Vec<&str> = t
        .text()
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| l.starts_with("[case] ") && l.contains(": run "))
        .collect();
    let oks = t
        .text()
        .lines()
        .filter(|l| l.trim_end_matches('\r').starts_with("[case] ") && l.contains(": ok "))
        .count();
    if runs.len() == oks {
        return Ok(());
    }
    Err(Gap {
        want: format!("每个 `[case] run` 都要有配对的 `ok`（run={} ok={oks}）", runs.len()),
        saw: format!("失败的那一例是：{}", runs.last().unwrap_or(&"—")),
    })
}

/// 一个形状在这一次跑里出现几行，对不对得上基线。
pub fn count(t: &Transcript, shape: &str, want: usize) -> Result<(), Gap> {
    let re = ere(shape);
    let got: Vec<&str> = t
        .text()
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| re.is_match(l))
        .collect();
    if got.len() == want {
        return Ok(());
    }
    Err(Gap {
        want: format!("`{shape}` 要 {want} 行"),
        saw: format!("实际 {} 行 —— 例：{}", got.len(), got.first().unwrap_or(&"—")),
    })
}

/// `[case]` 协议那一行的前缀（`cases.rs` 那个运行器打的）。
const CASE: &str = "[case]";

/// 读数行的前缀：`名字: …` 与 `[case] …` 两种形状（与今天的对账器同一口径）。
fn prefix_of(line: &str) -> Option<&str> {
    if line.starts_with(CASE) {
        return Some(CASE);
    }
    let (head, _) = line.split_once(": ")?;
    let mut cs = head.chars();
    let first = cs.next()?;
    if !first.is_ascii_lowercase() {
        return None;
    }
    if !head
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return None;
    }
    Some(head)
}

/// 构建噪声（rustc 的话，不是程序的读数）。
fn is_noise(prefix: &str) -> bool {
    matches!(prefix, "warning" | "help" | "error" | "note")
}

fn ere(pattern: &str) -> regex::Regex {
    regex::Regex::new(pattern)
        .unwrap_or_else(|e| panic!("形状不是合法 ERE：{pattern} —— {e}"))
}
