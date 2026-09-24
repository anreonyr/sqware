//! 门的牙口 —— **判据的判据**（搬自 `scripts/teeth.py`——**那一刀之后它已删**）。
//!
//! 判据只有一条：**一条断言如果在它守的那件事坏掉之后还不红，它就没有牙**。做法是外科式的——
//! 每次只改一处（都是**像样的 bug**，不是语法错），把那门跑一遍，记下"红在哪一条"，再还原。
//! 另有一条**对照**（只改注释）：它必须绿，否则红的是量具而不是被测的东西。
//!
//! # 三种名义
//!
//! | | 期望 | 说明 |
//! |---|---|---|
//! | 变异（名字里没前缀） | **红** | 该被逮住 |
//! | `等价·…` | 绿 | **不改变可观察行为**——实测出来的一档，不是"没牙" |
//! | `对照·…` | 绿 | 只改注释：量具本身对不对 |
//!
//! # 照实记（两条口径是从旧量具的教训里留下来的）
//!
//! - **红必须是"门红"，不是"编译红"**：第一版有一条变异是"编不过"（`slots.remove` 的返回值被
//!   多包了 `Some`），它也被记成"红"，但那不说明牙口，只说明编译器在岗。故 `MutateFailed::Compile`
//!   单列一档，**不记进账**。
//! - **补丁必须唯一**：锚点出现两次时 `replace(…, 1)` 会打在**前一处**，于是"绿"这个结论说的是
//!   别处的代码（实测栽过：`sweep_at` 那个锚点在两处各有一份）。故锚点数 ≠ 1 一律**拒绝应用**。
//!
//! # 照实记（两条限制在这一版里**取消了**）
//!
//! - 旧量具还原靠 `git checkout --`，故跑之前**必须**工作区干净（免得抹掉你正在写的东西）。
//!   这里还原是**把读到的原文写回去**（`Patch` 那个守卫，`Drop` 里做，panic 也还原）⇒ 你的手稿
//!   不会被抹，那条限制随之取消。
//! - 旧量具判"编译红"要靠**扫日志**（构建输出被 `soak.sh` 重定向进了每轮日志，工具拿到的
//!   stdout 里看不见）。这里 `rebuild` 直接返回 `Err` ⇒ 编译错误是一个**错误值**，不用猜。
//!
//! # 照实记（账：文件名没改，因为它不该因改名而失效）
//!
//! 账仍然落在 `target/teeth-ledger.json`——**键完全相同**（`名字|文件|sha1(锚点)[:12]`），故旧账
//! 直接接着用，默认一跑是**真空跑**（"这次没有实跑的"）。名字里的 `teeth` 是旧量具的名字；改名
//! 会让 30 分钟的机器侧复核白跑一遍，不值。
//!
//! 跑法：
//!
//! ```text
//!   cargo gate -- --ignored mutations                    # 只跑"没验过"的（第一次全跑，之后增量）
//!   GATE_ALL=1 cargo gate -- --ignored mutations         # 全跑（换机器 / 想复核时）
//!   GATE_ONLY=账 cargo gate -- --ignored mutations       # 只跑名字里带"账"的几条
//!   GATE_AT=soak cargo gate -- --ignored mutations       # 只跑机器那一侧
//!   GATE_AT=host cargo gate -- --ignored mutations       # 只跑宿主那一侧
//! ```

use crate::*;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 这一条变异**该**得到什么。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hope {
    Red,
    Equivalent,
}

/// 在哪台子上量。只给这张表用——表要能整行写成常量，而 `Image` 得先造出来。
#[derive(Clone, Copy)]
enum Where {
    /// 宿主靶（`crates/protocol-case`）；红 = `cargo test` 退出码非零。
    Host,
    /// 默认那一景走一趟（`soak::verdict`）；红 = 那一门的判据说不过。
    Soak,
}

/// 表里的一行（`Mutation` 的常量版）。
struct Raw {
    name: &'static str,
    path: &'static str,
    from: &'static str,
    to: &'static str,
    at: Where,
    hope: Hope,
}

/// 一条变异。
pub struct Mutation {
    pub name: &'static str,
    pub path: &'static str,
    /// 锚点：必须**恰好出现一次**。
    pub from: &'static str,
    pub to: &'static str,
    pub bench: Bench,
    pub hope: Hope,
}

/// 门的结论。
pub struct Verdict {
    pub red: bool,
    /// 红在哪（机器那侧 = 缺哪几条读数；宿主那侧 = 哪几条用例失败）。
    pub detail: String,
}

#[derive(Debug)]
pub enum MutateFailed {
    Io(String),
    /// 锚点出现 0 次或多次——**拒绝应用**（见头注那条教训）。
    Anchor { hits: usize },
    Build(String),
    Bench(String),
    /// 编不过：**不算牙口**，也不记进账。
    Compile(String),
    /// 期望等价绿，却红了。
    UnexpectedRed(String),
}

impl std::fmt::Display for MutateFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MutateFailed::Io(e) => write!(f, "读写不动：{e}"),
            MutateFailed::Anchor { hits } => {
                write!(f, "**补丁不唯一，拒绝应用**（锚点出现 {hits} 次）")
            }
            MutateFailed::Build(e) => write!(f, "造不出那颗要跑的：{e}"),
            MutateFailed::Bench(e) => write!(f, "台子起不动：{e}"),
            MutateFailed::Compile(t) => write!(f, "**编译红**（不算牙口）：{t}"),
            MutateFailed::UnexpectedRed(d) => write!(f, "**期望等价绿，却红了**：{d}"),
        }
    }
}

impl std::error::Error for MutateFailed {}

/// 还原守卫：**`Drop` 里把原文写回去**，故中途 panic 也还原（旧量具靠 `git checkout --`）。
struct Patch {
    path: PathBuf,
    original: String,
}

impl Drop for Patch {
    fn drop(&mut self) {
        let _ = std::fs::write(&self.path, &self.original);
    }
}

/// 把表映射成能跑的：机器那一台的 `Image` **先造一次**（`build` 有缓存，28 条也只构一次）。
pub fn all() -> Result<Vec<Mutation>, BuildFailed> {
    let image = build(Scenario::Root, Profile::Debug)?;
    Ok(RAW
        .iter()
        .map(|r| Mutation {
            name: r.name,
            path: r.path,
            from: r.from,
            to: r.to,
            hope: r.hope,
            bench: match r.at {
                Where::Host => Bench::Host {
                    manifest: "crates/protocol-case/Cargo.toml",
                    within: secs(300),
                },
                Where::Soak => Bench::Machine {
                    image: image.clone(),
                    sched: soak::feed(),
                    within: secs(45),
                    env: &[],
                },
            },
        })
        .collect())
}

/// 这一条变异的身份：**名字 + 文件 + 锚点指纹**。锚点一改，指纹就变 ⇒ 自动回到"没验过"。
pub fn key_of(m: &Mutation) -> String {
    let mut h = Sha1::new();
    h.update(m.from.as_bytes());
    let hex = format!("{:x}", h.finalize());
    format!("{}|{}|{}", m.name, m.path, &hex[..12])
}

/// 挪一处 → 走一趟 → 按期望判 → **放回去**。
pub fn mutate(m: &Mutation) -> Result<Verdict, MutateFailed> {
    let path = root().join(m.path);
    let src = std::fs::read_to_string(&path).map_err(|e| MutateFailed::Io(format!("{}: {e}", path.display())))?;
    let hits = src.matches(m.from).count();
    if hits != 1 {
        return Err(MutateFailed::Anchor { hits });
    }
    let _guard = Patch {
        path: path.clone(),
        original: src.clone(),
    };
    std::fs::write(&path, src.replacen(m.from, m.to, 1))
        .map_err(|e| MutateFailed::Io(format!("{}: {e}", path.display())))?;

    // 机器那一台上的东西是**编进镜像**的 ⇒ 改完源码必须**绕开缓存**重造，否则跑的还是旧那颗。
    if let Bench::Machine { .. } = m.bench {
        rebuild(Scenario::Root, Profile::Debug).map_err(|e| MutateFailed::Build(e.to_string()))?;
    }

    let t = run(&m.bench).map_err(|e| MutateFailed::Bench(e.to_string()))?;

    let compile_tail = |t: &Transcript| {
        t.text()
            .lines()
            .find(|l| l.contains("error[E") || l.contains("error: could not compile"))
            .unwrap_or("")
            .to_string()
    };
    if !compile_tail(&t).is_empty() {
        return Err(MutateFailed::Compile(compile_tail(&t)));
    }

    let (red, detail) = match m.bench {
        Bench::Host { .. } => {
            let red = t.code() != Some(0);
            let names: Vec<&str> = t
                .text()
                .lines()
                .filter_map(|l| {
                    l.strip_prefix("test ")
                        .and_then(|r| r.split(" ... FAILED").next())
                        .filter(|_| l.ends_with("FAILED"))
                })
                .collect();
            let detail = if names.is_empty() {
                t.text()
                    .lines()
                    .filter(|l| l.contains("error") || l.contains("FAILED"))
                    .next_back()
                    .unwrap_or("（没有失败行）")
                    .to_string()
            } else {
                names.join(" · ")
            };
            (red, detail)
        }
        Bench::Machine { .. } => match soak::verdict(&t) {
            Ok(()) => (false, String::new()),
            // **`want` 与 `saw` 都要**：`want` 是"缺了哪一条"，`saw` 才是**名字**——`pair` 那一条
            // 把失败的那一例放在 `saw` 里（第一版只取 `want`，于是变异报的是"run=62 ok=61"而
            // 说不出是哪一例；这一格是跑完那三条复验当场看出来的）。
            Err(gaps) => (
                true,
                gaps.iter()
                    .map(|g| format!("{} —— {}", g.want, g.saw))
                    .collect::<Vec<_>>()
                    .join(" · "),
            ),
        },
    };

    if m.hope == Hope::Equivalent && red {
        return Err(MutateFailed::UnexpectedRed(detail));
    }
    Ok(Verdict { red, detail })
}

// ── 账 ──────────────────────────────────────────────────────────────────────

/// 账本落在 `target/teeth-ledger.json`（**文件名没改**，见头注：账不该因改名而失效）。
pub fn ledger_path() -> PathBuf {
    root().join("target/teeth-ledger.json")
}

pub fn ledger_load() -> BTreeMap<String, String> {
    let Ok(text) = std::fs::read_to_string(ledger_path()) else {
        return BTreeMap::new();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return BTreeMap::new();
    };
    v.as_object()
        .map(|o| {
            o.iter()
                .filter(|(_, x)| x.is_string())
                .map(|(k, x)| (k.clone(), x.as_str().unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn ledger_save(book: &BTreeMap<String, String>) {
    let path = ledger_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut obj = serde_json::Map::new();
    obj.insert("_note".to_string(), serde_json::Value::String(
        "这一本账是**跳过提示**，不是判据：里面每一条都在旧量具那两遍全量里实测过（宿主 47 条、\
         机器 28 条）。锚点一改，那条的键就变、自动回到\"没验过\"。要复核用 `GATE_ALL=1`。"
            .to_string(),
    ));
    for (k, v) in book {
        obj.insert(k.clone(), serde_json::Value::String(v.clone()));
    }
    let _ = std::fs::write(
        &path,
        serde_json::to_string_pretty(&serde_json::Value::Object(obj)).unwrap_or_default(),
    );
}

// ── 表（从 `scripts/teeth.py` 机械抽出来的，一行未改；那份脚本已删）──────────────────────────

const RAW: &[Raw] = &[
    Raw { name: "树·剪掉时把槽移走（号=下标，后面全错位）", path: "crates/protocol/src/operator/core.rs", from: "        let taken = self.slots.get_mut(id.get())?.take()?;", to: "        if id.get() >= self.slots.len() {\n            return None;\n        }\n        let taken = self.slots.remove(id.get())?;", at: Where::Host, hope: Hope::Red },
    Raw { name: "树·落格不等容量（撤 try_reserve）", path: "crates/protocol/src/operator/core.rs", from: "                self.slots.try_reserve(1).map_err(|_| Fail::Full)?;\n                self.kids_mut(at)?.try_reserve(1).map_err(|_| Fail::Full)?;", to: "                // 变异：不 reserve", at: Where::Host, hope: Hope::Red },
    Raw { name: "树·不看 PANE_CAP（窗格可无限长）", path: "crates/protocol/src/operator/core.rs", from: "                if self.kids(at)?.len() >= Self::PANE_CAP {\n                    return Err(Fail::Full);\n                }", to: "                // 变异：不看条数闸", at: Where::Host, hope: Hope::Red },
    Raw { name: "树·find 不问死活（不剔死）", path: "crates/protocol/src/operator/core.rs", from: "        if vested_by(pie).is_none() {\n            let _ = self.unlink(id);\n            let _ = unship(pie);\n            return Err(Fail::Dead);\n        }", to: "        // 变异：不问死活", at: Where::Host, hope: Hope::Red },
    Raw { name: "树·opens 答授与人而不是开者", path: "crates/protocol/src/operator/core.rs", from: "        let opened_by = self.stamps.opened_by;", to: "        let opened_by = self.stamps.vested_by;", at: Where::Host, hope: Hope::Red },
    Raw { name: "树·trim 允许剪非空窗格", path: "crates/protocol/src/operator/core.rs", from: "                Node::Pane(inner) if inner.is_empty() => None,\n                Node::Pane(_) => return Err(Fail::NonEmpty),", to: "                Node::Pane(inner) if inner.is_empty() => None,\n                Node::Pane(_) => None,", at: Where::Host, hope: Hope::Red },
    Raw { name: "树·seek 不看路长上限", path: "crates/protocol/src/operator/core.rs", from: "        if road.len() > Self::ROAD_MAX {\n            return Err(Fail::Full);\n        }", to: "        // 变异：不看路长", at: Where::Host, hope: Hope::Red },
    Raw { name: "判·把 Err 读成 Deny（判不了塌成没资格）", path: "crates/protocol/src/operator/judge.rs", from: "    let Some(me) = (match roster.who(who) {\n        Ok(found) => found,\n        Err(()) => return Ruling::Unjudged,\n    }) else {\n        return Ruling::Deny;\n    };", to: "    let Some(me) = (match roster.who(who) {\n        Ok(found) => found,\n        Err(()) => return Ruling::Deny,\n    }) else {\n        return Ruling::Deny;\n    };", at: Where::Host, hope: Hope::Red },
    Raw { name: "判·Opens 那一格放行（不问门牌）", path: "crates/protocol/src/operator/judge.rs", from: "        Rule::Opens(at) => match door.opens(at) {", to: "        Rule::Opens(at) if at.get() == usize::MAX => Ruling::Deny,\n        Rule::Opens(_) => Ruling::Allow,\n        #[allow(unreachable_patterns)]\n        Rule::Opens(at) => match door.opens(at) {", at: Where::Host, hope: Hope::Red },
    Raw { name: "判·Opens 没有那一位时终态拒", path: "crates/protocol/src/operator/judge.rs", from: "            Ok(None) => Ruling::Unjudged,\n            Err(()) => Ruling::Unjudged,", to: "            Ok(None) => Ruling::Deny,\n            Err(()) => Ruling::Deny,", at: Where::Host, hope: Hope::Red },
    Raw { name: "账·所有权不看主人死活", path: "crates/protocol/src/operator/ledger.rs", from: "            Some(owner) => (self.vested_by)(owner.pie).is_none(),", to: "            Some(_owner) => false,", at: Where::Host, hope: Hope::Red },
    Raw { name: "账·陈旧那一行不销（照旧答它）", path: "crates/protocol/src/operator/ledger.rs", from: "        if fresh(self.lines[at].id) {\n            Some(at)\n        } else {", to: "        if true {\n            Some(at)\n        } else {", at: Where::Host, hope: Hope::Red },
    Raw { name: "账·grow 退成 fail-soft", path: "crates/protocol/src/operator/ledger.rs", from: "        self.lines.try_reserve(1).map_err(|_| Fail::Full)?;", to: "        let _ = self.lines.try_reserve(1);", at: Where::Host, hope: Hope::Red },
    Raw { name: "线·占两次不拒（同一格两个主人）", path: "crates/protocol/src/driver/line/core.rs", from: "            Some(Cell::Owned { .. }) => Err(Fail::Taken),", to: "            Some(Cell::Owned { .. }) => Ok(()),", at: Where::Host, hope: Hope::Red },
    Raw { name: "线·没主也投（推给一格空的）", path: "crates/protocol/src/driver/line/core.rs", from: "            Some(Cell::Idle) | None => Err(Fail::Unknown),\n        }\n    }\n\n    /// **exhaust**", to: "            Some(Cell::Idle) | None => Ok(()),\n        }\n    }\n\n    /// **exhaust**", at: Where::Host, hope: Hope::Red },
    Raw { name: "线·投不出去也置忙", path: "crates/protocol/src/driver/line/core.rs", from: "                lane.post(frame).map_err(|()| Fail::Denied)?;\n                *busy = true;\n                Ok(())", to: "                let _ = lane.post(frame);\n                *busy = true;\n                Ok(())", at: Where::Host, hope: Hope::Red },
    Raw { name: "线·排空不清忙", path: "crates/protocol/src/driver/line/core.rs", from: "            Some(Cell::Owned { busy, .. }) => {\n                *busy = false;\n                Ok(())\n            }", to: "            Some(Cell::Owned { .. }) => Ok(()),", at: Where::Host, hope: Hope::Red },
    Raw { name: "册·名册不看钥匙（谁都能写）", path: "crates/protocol/src/principal/core.rs", from: "        if from != self.assembler {\n            return Err(Fail::Denied);\n        }\n        if self.node(p).is_none() {", to: "        if self.node(p).is_none() {", at: Where::Host, hope: Hope::Red },
    Raw { name: "册·转换不看支（跨支也能领）", path: "crates/protocol/src/principal/core.rs", from: "        if !self.heir(p, q)? {\n            return Err(Fail::Denied);\n        }", to: "        // 变异：不看支", at: Where::Host, hope: Hope::Red },
    Raw { name: "等价·盟籍入不查重（表里多一行，四条读都看不见）", path: "crates/protocol/src/coalition/core.rs", from: "        if self.book.iter().any(|a| a.who == who && a.of == c) {\n            return Ok(());\n        }", to: "        // 变异：不查重", at: Where::Host, hope: Hope::Equivalent },
    Raw { name: "板·查名字不剔死（死的照旧答得出）", path: "crates/protocol/src/system/board/core.rs", from: "        mut ship: impl FnMut(PieToken),\n    ) -> Result<PieToken, Fail> {\n        let at = self.find(name).ok_or(Fail::Unknown)?;\n        self.sweep_at(at);", to: "        mut ship: impl FnMut(PieToken),\n    ) -> Result<PieToken, Fail> {\n        let at = self.find(name).ok_or(Fail::Unknown)?;\n        // 变异：不扫", at: Where::Host, hope: Hope::Red },
    Raw { name: "板·撤牌子不剔死（死的照旧拦人）", path: "crates/protocol/src/system/board/core.rs", from: "    pub fn unregister(&mut self, name: Name, who: TaskId) -> Result<(), Fail> {\n        let at = self.find(name).ok_or(Fail::Unknown)?;\n        self.sweep_at(at);", to: "    pub fn unregister(&mut self, name: Name, who: TaskId) -> Result<(), Fail> {\n        let at = self.find(name).ok_or(Fail::Unknown)?;\n        // 变异：不扫", at: Where::Host, hope: Hope::Red },
    Raw { name: "会·seat 不挡同名（同一位同记号能有两枚）", path: "crates/protocol/src/session/core.rs", from: "        if self.find(name).is_some() {\n            return Err(Seat::NoName);\n        }\n\n        // 本端那一枚：先铸", to: "        // 变异：不挡同名\n\n        // 本端那一枚：先铸", at: Where::Host, hope: Hope::Red },
    Raw { name: "会·扫表不排除已经用掉的那几枚", path: "crates/protocol/src/session/core.rs", from: "            if h.owner != Some(of)\n                || h.mark != mark\n                || piers\n                    .iter()\n                    .any(|p| p.hole == h.token || p.at_peer == Some(h.token))\n            {", to: "            if h.owner != Some(of) || h.mark != mark {", at: Where::Host, hope: Hope::Red },
    Raw { name: "会·两格判据只看记号（不看谁开的）", path: "crates/protocol/src/session/core.rs", from: "            if h.owner != Some(of)\n                || h.mark != mark", to: "            if h.mark != mark", at: Where::Host, hope: Hope::Red },
    Raw { name: "会·拆泊位不告诉对方", path: "crates/protocol/src/session/core.rs", from: "        let _ = p.post(&call::UNSEAT);", to: "        // 变异：不说那一句", at: Where::Host, hope: Hope::Red },
    Raw { name: "会·额度为零时不早退（Partial 塌成 Timeout，且白等一场）", path: "crates/protocol/src/session/core.rs", from: "        if self.piers.is_empty() {\n            // 我一条都没 seat 出去 ⇒ 没有额度可认领（对方无从知道该给我几条）。\n            return Err(Claim::Partial);\n        }", to: "        if false {\n            return Err(Claim::Partial);\n        }", at: Where::Host, hope: Hope::Red },
    Raw { name: "编·登记不查重名（同名能有两行）", path: "crates/protocol/src/system/desk.rs", from: "        if self.find(name).is_some() {\n            return Err(Fail::Unknown);\n        }", to: "        // 变异：不查重名", at: Where::Host, hope: Hope::Red },
    Raw { name: "编·摘身子把行也删了", path: "crates/protocol/src/system/desk.rs", from: "        if let Some(s) = self.row_mut(name) {\n            s.slot = Slot::None;\n            s.root = None;\n        }", to: "        if let Some(s) = self.row_mut(name) {\n            s.slot = Slot::None;\n            s.root = None;\n            s.name = Name::EMPTY;\n        }", at: Where::Host, hope: Hope::Red },
    Raw { name: "编·`Dead` 也算不该起（重发那一格没了）", path: "crates/protocol/src/system/core.rs", from: "        State::NeverStarted | State::Dead => Ok(()),", to: "        State::NeverStarted => Ok(()),\n        State::Dead => Err(Fail::NotReady),", at: Where::Host, hope: Hope::Red },
    Raw { name: "编·不宣布的那种也当「还没起来」", path: "crates/protocol/src/system/core.rs", from: "            (Announce::None, Slot::Live { .. }) => Ready::Up,", to: "            (Announce::None, Slot::Live { .. }) => Ready::Pending,", at: Where::Host, hope: Hope::Red },
    Raw { name: "帧·线那一格：长度不对也照收（猜）", path: "crates/protocol/src/driver/line/call.rs", from: "    if frame.len() != OCCUPY_LEN || frame[0] != OCCUPY {", to: "    if frame.is_empty() || frame[0] != OCCUPY {", at: Where::Host, hope: Hope::Red },
    Raw { name: "帧·判别号不认识也照收", path: "crates/protocol/src/driver/line/call.rs", from: "    Key::from_bytes(raw)\n}", to: "    Some(Key::region(u64::from_le_bytes(raw[..8].try_into().ok()?)))\n}", at: Where::Host, hope: Hope::Red },
    Raw { name: "帧·游标那一格不加一（零号被当成「没有」）", path: "crates/protocol/src/coalition/frame.rs", from: "        Some(at) => at.get() as u64 + 1,", to: "        Some(at) => at.get() as u64,", at: Where::Host, hope: Hope::Red },
    Raw { name: "帧·窗答不看条数（帧长与条数对不上也读）", path: "crates/protocol/src/coalition/frame.rs", from: "    if count > WINDOW_CAP || body.len() != count * 8 {", to: "    if count > WINDOW_CAP {", at: Where::Host, hope: Hope::Red },
    Raw { name: "帧·「有没有」那一格不用了（根被读成没有）", path: "crates/protocol/src/principal/frame.rs", from: "    out[1] = present as u8;", to: "    out[1] = 0;", at: Where::Host, hope: Hope::Red },
    Raw { name: "帧·路数那一格裁成上限（超长的被当成正好）", path: "crates/protocol/src/operator/frame.rs", from: "            out[1] = road.len().min(u8::MAX as usize) as u8;", to: "            out[1] = filled.min(u8::MAX as usize) as u8;", at: Where::Host, hope: Hope::Red },
    Raw { name: "帧·规矩那一格认不出的标号不退回 Public（当成一枚号）", path: "crates/protocol/src/operator/frame.rs", from: "        _ => Rule::Public,", to: "        _ => Rule::Is(id),", at: Where::Host, hope: Hope::Red },
    Raw { name: "帧·列答不看条数（帧长与条数对不上也读）", path: "crates/protocol/src/operator/frame.rs", from: "    if count > Operator::PANE_CAP || body.len() != count * 8 {", to: "    if count > Operator::PANE_CAP {", at: Where::Host, hope: Hope::Red },
    Raw { name: "帧·号答不看长度（多长都读）", path: "crates/protocol/src/operator/frame.rs", from: "    if body.len() != 8 {\n        return Err(BAD);\n    }", to: "    if body.len() < 8 {\n        return Err(BAD);\n    }", at: Where::Host, hope: Hope::Red },
    Raw { name: "帧·板那一面：不带的 seed 也照收（零号与「没有」分不开）", path: "crates/protocol/src/system/board/frame.rs", from: "    out[1 + env::wire::NAME_LEN..].copy_from_slice(&seed.to_bytes());", to: "    out[1 + env::wire::NAME_LEN..].copy_from_slice(&7u64.to_le_bytes());", at: Where::Host, hope: Hope::Red },
    Raw { name: "帧·板那一面：名字不判尾（整段当名字，补的零也算进去）", path: "crates/protocol/src/system/board/frame.rs", from: "    Name::from_bytes(raw).ok()", to: "    Name::from_slice(&raw).ok()", at: Where::Host, hope: Hope::Red },
    Raw { name: "供·两族视图收混族的位（`Access` 收下传递族）", path: "crates/env/src/wire/access.rs", from: "            Some(p) if p.bits() & ACCESS_MASK.bits() != p.bits() => None,", to: "            Some(_p) if false => None,", at: Where::Host, hope: Hope::Red },
    Raw { name: "供·单子那一条不看条数（帧长与条数对不上也读）", path: "crates/protocol/src/driver/supply/call.rs", from: "    if n > WANT_MAX || bytes.len() < HEAD_LEN + n * WANT_LEN {", to: "    if n > WANT_MAX {", at: Where::Host, hope: Hope::Red },
    Raw { name: "供·回单那一条不看步长（零头也编）", path: "crates/protocol/src/driver/supply/call.rs", from: "    if !records.len().is_multiple_of(PAIR_LEN) {", to: "    if false {", at: Where::Host, hope: Hope::Red },
    Raw { name: "供·已知坐标也去查表（`settle` 两格不分）", path: "crates/protocol/src/driver/supply/call.rs", from: "            At::Known(key) => key,", to: "            At::Known(_key) => of(\"\")?,", at: Where::Host, hope: Hope::Red },
    Raw { name: "对照·只改注释", path: "crates/protocol/src/operator/core.rs", from: "/// **开**：第 `id` 格**是谁的门牌**（那一枚句柄的开者）。", to: "/// **开**：第 `id` 格**是谁的门牌**（那一枚句柄的开者）。  ", at: Where::Host, hope: Hope::Equivalent },
    Raw { name: "机器·`foreign` 那一格改成公开（该 8 会答 0）", path: "harness/src/probe_rule.rs", from: "            Rule::Opens(principal),", to: "            Rule::Public,", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·撤掉问话孔的「先找后铸」（该 ask_same=1 会 0）", path: "crates/protocol/src/operator/client.rs", from: "    if let Some(have) = crate::session::call::find(me()?, ASK_MARK) {\n        return Ok(have);\n    }\n", to: "", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·编排域的判词不落（该有 `system: done`）", path: "programs/src/supervisor/system/main.rs", from: "    service::die(service::E_OK, \"system: done\")", to: "    service::die(service::E_OK, \"system: farewell\")", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·设备数那一条读数说谎（该 21 报 20）", path: "kernel/src/platform/devices.rs", from: "    crate::putln!(\"devices: {n} handed to root\");", to: "    crate::putln!(\"devices: {} handed to root\", n - 1);", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·「类 → 区」那条翻译换了形（四处一起）", path: "programs/src/supervisor/service.rs", from: "                \"system: {} {} -> {:#x}\",", to: "                \"system: {} {} => {:#x}\",", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·递单那一条读数换了形（`paired`/`post` 对调）", path: "programs/src/supervisor/service.rs", from: "        \"wire: {} bytes, paired={}, post={}\",", to: "        \"wire: {} bytes, post={}, paired={}\",", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·`passer` 不上板（该 `reg=0` 会答别的）", path: "harness/src/passer.rs", from: "    let reg = board::ask(talk, &link, board, bcall::REGISTER, me, entry, MS).unwrap_or(BAD);", to: "    let reg = board::ask(talk, &link, board, bcall::LOOKUP, me, entry, MS).unwrap_or(BAD);", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·客人要的名字树上没有（`find` 与判词两条一起变）", path: "harness/src/guest.rs", from: "const WANT: &str = \"router\";", to: "const WANT: &str = \"routr\";", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·boot 域把 dtb 那一类数进区（三条读数一起变）", path: "programs/src/supervisor/root/boot.rs", from: "                Some(DTB) => dtb += 1,", to: "                Some(DTB) => region += 1,", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·该出的那一位没出（入替了出 ⇒ `amid`/`band` 两条变）", path: "harness/src/member.rs", from: "    let left = coal.leave(c0, MS);", to: "    let left = coal.enter(c0, MS);", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·回声列名字那一条换了形", path: "programs/src/user/echo.rs", from: "    let _ = debug::put(&format!(\"echo: list names={names}\"));", to: "    let _ = debug::put(&format!(\"echo: list named={names}\"));", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·停机那一句判词不落（`task: all tasks exited`）", path: "kernel/src/work/room/conductor.rs", from: "        putln!(\"task: all tasks exited, system halted\");", to: "        putln!(\"task: all tasks gone, system halted\");", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·计时那行少一格（`tocks` 不报）", path: "kernel/src/work/room/conductor.rs", from: "\"timer: late_n={late_n} late_max_ms={max_ms} late_avg_ms={avg_ms} late_max_tick={late_max} traps={} tocks={tocks} mutes={mutes}\",", to: "\"timer: late_n={late_n} late_max_ms={max_ms} late_avg_ms={avg_ms} late_max_tick={late_max} traps={}\",", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·末日那行换了形（`doom:` 改 `doomed:`）", path: "kernel/src/work/room/conductor.rs", from: "putln!(\"doom: held={held} starved={starved} blocked={blocked} nudged={nudged}\");", to: "putln!(\"doomed: held={held} starved={starved} blocked={blocked} nudged={nudged}\");", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·调度那行少一格（`fallback` 不报）", path: "kernel/src/work/room/conductor.rs", from: "putln!(\"sched: kicks={kicks} fallback={fallback}\");", to: "putln!(\"sched: kicks={kicks}\");", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·中断那行多一格（`idle_busy` 报错位）", path: "kernel/src/work/room/conductor.rs", from: "putln!(\"irq: ring={ring} busy={busy} idle_ring={idle_ring} idle_busy={idle_busy}\");", to: "putln!(\"irq: ring={ring} busy={busy} idle_busy={idle_busy} idle_ring={idle_ring}\");", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·弃那一趟改成领根（`policy: adopt(out)` 那条变）", path: "harness/src/subject.rs", from: "    let outside = face.adopt(PrincipalId::new(OUTSIDE), MS);", to: "    let outside = face.adopt(PrincipalId::ROOT, MS);", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·「已经过去」那一格改成将来（`sleeper: past=` 那条变）", path: "harness/src/sleeper.rs", from: "    let past = refused(clock::arm(face, now.saturating_sub(1_000_000), MS));", to: "    let past = refused(clock::arm(face, now.saturating_add(60_000_000_000), MS));", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·盟号对调（`member: found=` 第二条报第一枚）", path: "harness/src/member.rs", from: "    let c1 = coal.found(MS);\n    say(&format!(\"member: found={}\", one_id(c1)));", to: "    let c1 = coal.found(MS);\n    say(&format!(\"member: found={}\", one_id(c0)));", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·回声那一串走错趟（`echo: seq=` 那条变）", path: "programs/src/user/echo.rs", from: "    let seq = serial(&tree, talk);", to: "    let seq = serial(&tree, talk).max(1);", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·uart 把那句话读反（`ier=rx` 改 `ier=tx`）", path: "programs/src/driver/uart/main.rs", from: "    say(&alloc::format!(\"uart: ier=rx at={base:#x}\"));", to: "    say(&alloc::format!(\"uart: ier=tx at={base:#x}\"));", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·rtc 那一行换了形（`armed at=` 改 `armed=`）", path: "programs/src/driver/rtc/main.rs", from: "                        \"rtc: armed at={at} ier={} alarm={}\",", to: "                        \"rtc: armed={at} ier={} alarm={}\",", at: Where::Soak, hope: Hope::Red },
    Raw { name: "等价·租赁那一趟不声明归自己（机器读数看不见；宿主靶管着那一格）", path: "harness/src/probe_lease.rs", from: "        ocall::Rule::Public,\n        true,", to: "        ocall::Rule::Public,\n        false,", at: Where::Soak, hope: Hope::Equivalent },
    Raw { name: "等价·接手那一趟反而声明归自己（同上：探针等的就是对方死）", path: "harness/src/probe_owner.rs", from: "        ocall::Rule::Public,\n        false,", to: "        ocall::Rule::Public,\n        true,", at: Where::Soak, hope: Hope::Equivalent },
    Raw { name: "机器·「该被拒」那一趟的判词换了名", path: "harness/src/probe_denied.rs", from: "const OK_NOTE: &str = \"probe-denied: denied as expected\";", to: "const OK_NOTE: &str = \"probe-denied: denied\";", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·回声去问一枚**铸过的**号（`name miss=true` 变 false）", path: "programs/src/user/echo.rs", from: "    let miss = operator::name(talk, link, EntryId::new(4095), MS).is_err();", to: "    let miss = operator::name(talk, link, EntryId::new(0), MS).is_err();", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·房客的表那一条读数说谎（`pies=9` 报 10）", path: "harness/src/lodger/main.rs", from: "    let pies = mail::table_size();", to: "    let pies = mail::table_size() + 1;", at: Where::Soak, hope: Hope::Red },
    Raw { name: "机器·房客的判词改了名（该有 `lodger: gone`）", path: "harness/src/lodger/main.rs", from: "            \"lodger: gone\"", to: "            \"lodger: farewell\"", at: Where::Soak, hope: Hope::Red },
];
