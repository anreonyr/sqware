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

/// 宿主那一档的 target 名字。门里不写它——"宿主"这两个字的落点就在这一句。
pub const HOST: &str = "x86_64-unknown-linux-gnu";

/// 仓根（本 crate 在 `<根>/crates/gate`）。
fn root() -> PathBuf {
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

/// 造那颗要跑的。同一 `(Scenario, Profile)` 在一次测试进程里**只真构一次**。
pub fn build(scenario: Scenario, profile: Profile) -> Result<Image, BuildFailed> {
    static DONE: Mutex<Vec<((Scenario, Profile), Image)>> = Mutex::new(Vec::new());

    let mut done = DONE.lock().unwrap();
    if let Some((_, image)) = done.iter().find(|(k, _)| *k == (scenario, profile)) {
        return Ok(image.clone());
    }

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
pub enum Bench {
    Machine {
        image: Image,
        sched: Schedule,
        within: Deadline,
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
pub fn run(bench: &Bench) -> Result<Transcript, BenchFailed> {
    let root = root();
    let (mut child, within) = match bench {
        Bench::Machine {
            image,
            within,
            ..
        } => {
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
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    root().join(format!("target/gate/{door}-{now}.log"))
}
