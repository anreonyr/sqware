//! 看机 —— 起一台、等它起稳、喂一把、把读数原样交给你。
//!
//! 它是原先那三份**看机工具**（`scripts/fast.sh` / `quick.sh` / `probe.sh`）的落点：三份的差别
//! 只在"等多久、喂什么、看什么时序"，而**能收能喂**这一半是一样的。这里留那一样：
//!
//! ```text
//!   cargo gate -- --ignored --nocapture console        # 默认喂一条 `dir`
//!   GATE_FEED="dir tree" cargo gate -- --ignored --nocapture console
//! ```
//!
//! **照实记（末尾补一条 `exit`）**：喂完就把它放那儿不会自己停——旧那三份都在末尾补一条
//! `exit`（`quick.sh` 是 `sleep 1; printf 'exit\n'`）。这里照办：`GATE_FEED` 之后**总是**再喂
//! 一条 `exit`（已经写过 `exit` 也无害：写端早已断开，那句写不进去，而写不进去正是收尾）。
//!
//! **照实记（等标记，不按钟表）**：旧的三份各自 `settle 2.5` / `sleep 3` 地按钟表喂——那与启动
//! 期赛跑，`soak.sh` 头注里那两次假红记的就是这件事。这里统一**等 `echo: ready`**（上限 25 秒，
//! 到点照喂）。
//!
//! **照实记（`QEMU_MEM` 那两格没搬）**：`fast.sh` / `probe.sh` 把内存压到 192 起机。那是它们
//! 当时要的时序，不是这一台的判据；这里用 `boot.nu` 的默认（256）——192 那一档在认设备那一刀
//! 之后**每一轮都红**（`Not enough memory to place DTB after kernel/initrd`，见 `boot.nu` 头注）。
//!
//! **照实记（控制台是共享的，行不是原子的）**：第一次跑这一台就当场看到两行被**另一个域**的行
//! 插进了中间——
//!
//! ```text
//! echo: tree part=0 land=0 find=0 got=true trim=0 plate=19 pname=echo[case] probe-rule: ok …
//! [case] probe-rule: run opens_follows_the_opener_not_the_callerecho: op=0
//! ```
//!
//! **不是收的时候合的**：`read_line` 只按 `\n` 切，切不出这种行 ⇒ 是机器上两个域往同一个控制台
//! 写、而那一手不是整行动作。`soak` 那一台看不到它（它的喂键等 `echo: seq=0` 之后才动，两条流
//! 错开了），故那些整行形状从没被这一格撞过。**记在这里，是因为它是机器的事实，不是这一台的事。**

use gate::*;
use std::time::Duration;

const READY: &str = "echo: ready";

#[test]
#[ignore = "要起 QEMU（看机用，不是门）"]
fn console() {
    let image = build(Scenario::Root, Profile::Release).expect("造不出那颗要跑的");
    let feed = std::env::var("GATE_FEED").unwrap_or_else(|_| "dir".to_string());
    let mut then: Vec<String> = feed.split_whitespace().map(str::to_string).collect();
    then.push("exit".to_string());

    let t = run(&Bench::Machine {
        image,
        sched: Schedule::OnMark {
            mark: READY,
            within: Duration::from_secs(25),
            then,
        },
        within: secs(45),
        env: &[],
    })
    .expect("机器那一台起不动");

    let path = log_path("console");
    let _ = t.keep(&path);
    print!("{}", t.text());
    eprintln!("console: 读数留在 {}", path.display());
}
