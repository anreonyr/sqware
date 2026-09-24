#!/usr/bin/env python3
"""门的牙口 —— **量具，不是门**：逐条把某处不变量弄坏，看 `scripts/host.sh` 红不红、红在哪一条。

判据只有一条：**一条断言如果在它守的那件事坏掉之后还不红，它就没有牙**。做法是外科式的——
每次只改一处（都是**像样的 bug**，不是语法错），把门跑一遍，记下"红在哪一条用例"，再还原。
另有一条**对照**（只改注释）：它必须绿，否则红的是量具而不是被测的东西。

**它会改源码再还原**，故：

  - 跑之前**工作区必须干净**（下面第一件事就是查这个）；有未提交的改动它会当场退出，
    免得 `git checkout --` 把你正在写的东西抹掉；
  - 每次变异只碰一处、跑完立刻还原；中断了也只要 `git checkout -- <那个文件>`。

跑法（约一分钟：13 条变异各一次增量重编 + 一个测试靶）：

    python3 scripts/teeth.py            # 全跑
    python3 scripts/teeth.py 账         # 只跑名字里带"账"的几条

**照实记一：`host.sh` 的日志名精确到秒且是追加写**——本量具第一次跑就栽在这一格：一秒内连跑
两次复用同一份日志，末了那句 `grep -q 'FAILED' "$log"` 扫到的是**上一轮**的失败 ⇒ 连"只改注释"
都被判成红。**已修**（日志名加 `-$$`，见 `scripts/host.sh`）。量具自己也加固了：每轮先清日志、
跨过秒边界。

**照实记三：这一遍量出一条"没牙"的**——`线·投不出去也置忙`（把 `deliver` 里 `post` 的失败
忽略掉）**全门照绿**：宿主靶的 `Pier` 桩恒答 `Ok`，故"推不出去 ⇒ 不置忙"那条契约**量不到**。
已给桩加一个**可关掉的失败开关**（线程局部，同那两台分配器）+ 一条用例
（`crates/line-case::a_frame_that_cannot_be_posted_leaves_the_line_idle`），并把这一条变异放进
这一遍的清单里——**复跑它就红了**（17/17）。这正是这个量具的用处：不是"证明门很好"，
是**指出哪一格缺牙**，缺的那一格补上。

**照实记二：第一版有一条变异是"编不过"**（`slots.remove` 的返回值被我多包了 `Some`）——
它也被记成"红"，但那不说明牙口，只说明编译器在岗。故这一格的口径是：**红必须是"门红"，
不是"编译红"**；那一条改成真变异（把槽从表里移走 ⇒ 号=下标的错位）之后，红了三条用例。
"""

import re
import subprocess
import sys

ROOT = "/home/anreonyr/Develop/sqware"

# (名字, 文件, 原文, 改成)  —— 每一条都必须是**像样的** bug，不是语法错。
MUTATIONS = [
    # ── 树（operator-case）──────────────────────────────
    ("树·剪掉时把槽移走（号=下标，后面全错位）",
     "crates/protocol/src/operator/core.rs",
     "        let taken = self.slots.get_mut(id.get())?.take()?;",
     "        if id.get() >= self.slots.len() {\n            return None;\n        }\n        let taken = self.slots.remove(id.get())?;"),
    ("树·落格不等容量（撤 try_reserve）",
     "crates/protocol/src/operator/core.rs",
     "                self.slots.try_reserve(1).map_err(|_| Fail::Full)?;\n                self.kids_mut(at)?.try_reserve(1).map_err(|_| Fail::Full)?;",
     "                // 变异：不 reserve"),
    ("树·不看 PANE_CAP（窗格可无限长）",
     "crates/protocol/src/operator/core.rs",
     "                if self.kids(at)?.len() >= Self::PANE_CAP {\n                    return Err(Fail::Full);\n                }",
     "                // 变异：不看条数闸"),
    ("树·find 不问死活（不剔死）",
     "crates/protocol/src/operator/core.rs",
     "        if vested_by(pie).is_none() {\n            let _ = self.unlink(id);\n            let _ = unship(pie);\n            return Err(Fail::Dead);\n        }",
     "        // 变异：不问死活"),
    ("树·opens 答授与人而不是开者",
     "crates/protocol/src/operator/core.rs",
     "        let opened_by = self.stamps.opened_by;",
     "        let opened_by = self.stamps.vested_by;"),
    ("树·trim 允许剪非空窗格",
     "crates/protocol/src/operator/core.rs",
     "                Node::Pane(inner) if inner.is_empty() => None,\n                Node::Pane(_) => return Err(Fail::NonEmpty),",
     "                Node::Pane(inner) if inner.is_empty() => None,\n                Node::Pane(_) => None,"),
    ("树·seek 不看路长上限",
     "crates/protocol/src/operator/core.rs",
     "        if road.len() > Self::ROAD_MAX {\n            return Err(Fail::Full);\n        }",
     "        // 变异：不看路长"),
    # ── 门禁（judge-case）───────────────────────────────
    ("判·把 Err 读成 Deny（判不了塌成没资格）",
     "crates/protocol/src/operator/judge.rs",
     "    let Some(me) = (match roster.who(who) {\n        Ok(found) => found,\n        Err(()) => return Ruling::Unjudged,\n    }) else {\n        return Ruling::Deny;\n    };",
     "    let Some(me) = (match roster.who(who) {\n        Ok(found) => found,\n        Err(()) => return Ruling::Deny,\n    }) else {\n        return Ruling::Deny;\n    };"),
    ("判·Opens 那一格放行（不问门牌）",
     "crates/protocol/src/operator/judge.rs",
     "        Rule::Opens(at) => match door.opens(at) {",
     "        Rule::Opens(at) if at.get() == usize::MAX => Ruling::Deny,\n        Rule::Opens(_) => Ruling::Allow,\n        #[allow(unreachable_patterns)]\n        Rule::Opens(at) => match door.opens(at) {"),
    ("判·Opens 没有那一位时终态拒",
     "crates/protocol/src/operator/judge.rs",
     "            Ok(None) => Ruling::Unjudged,\n            Err(()) => Ruling::Unjudged,",
     "            Ok(None) => Ruling::Deny,\n            Err(()) => Ruling::Deny,"),
    ("账·所有权不看主人死活",
     "crates/protocol/src/operator/ledger.rs",
     "            Some(owner) => (self.vested_by)(owner.pie).is_none(),",
     "            Some(_owner) => false,"),
    ("账·陈旧那一行不销（照旧答它）",
     "crates/protocol/src/operator/ledger.rs",
     "        if fresh(self.lines[at].id) {\n            Some(at)\n        } else {",
     "        if true {\n            Some(at)\n        } else {"),
    ("账·grow 退成 fail-soft",
     "crates/protocol/src/operator/ledger.rs",
     "        self.lines.try_reserve(1).map_err(|_| Fail::Full)?;",
     "        let _ = self.lines.try_reserve(1);"),
    # ── 线（line-case）──────────────────────────────────
    ("线·占两次不拒（同一格两个主人）",
     "crates/protocol/src/driver/line/core.rs",
     r"            Some(Cell::Owned { .. }) => Err(Fail::Taken),",
     r"            Some(Cell::Owned { .. }) => Ok(()),"),
    ("线·没主也投（推给一格空的）",
     "crates/protocol/src/driver/line/core.rs",
     r"""            Some(Cell::Idle) | None => Err(Fail::Unknown),
        }
    }

    /// **exhaust**""",
     r"""            Some(Cell::Idle) | None => Ok(()),
        }
    }

    /// **exhaust**"""),
    ("线·投不出去也置忙",
     "crates/protocol/src/driver/line/core.rs",
     r"""                lane.post(frame).map_err(|()| Fail::Denied)?;
                *busy = true;
                Ok(())""",
     r"""                let _ = lane.post(frame);
                *busy = true;
                Ok(())"""),
    ("线·排空不清忙",
     "crates/protocol/src/driver/line/core.rs",
     r"""            Some(Cell::Owned { busy, .. }) => {
                *busy = false;
                Ok(())
            }""",
     r"""            Some(Cell::Owned { .. }) => Ok(()),"""),
    # ── 两本册子（principal-case：名册/谱系 + 盟籍）──────
    ("册·名册不看钥匙（谁都能写）",
     "crates/protocol/src/principal/core.rs",
     "        if from != self.assembler {\n            return Err(Fail::Denied);\n        }\n        if self.node(p).is_none() {",
     "        if self.node(p).is_none() {"),
    ("册·转换不看支（跨支也能领）",
     "crates/protocol/src/principal/core.rs",
     "        if !self.heir(p, q)? {\n            return Err(Fail::Denied);\n        }",
     "        // 变异：不看支"),
    ("等价·盟籍入不查重（表里多一行，四条读都看不见）",
     "crates/protocol/src/coalition/core.rs",
     "        if self.book.iter().any(|a| a.who == who && a.of == c) {\n            return Ok(());\n        }",
     "        // 变异：不查重"),
    # ── 板（board-case）────────────────────────────────
    ("板·查名字不剔死（死的照旧答得出）",
     "crates/protocol/src/system/board/core.rs",
     "        mut ship: impl FnMut(PieToken),\n    ) -> Result<PieToken, Fail> {\n        let at = self.find(name).ok_or(Fail::Unknown)?;\n        self.sweep_at(at);",
     "        mut ship: impl FnMut(PieToken),\n    ) -> Result<PieToken, Fail> {\n        let at = self.find(name).ok_or(Fail::Unknown)?;\n        // 变异：不扫"),
    ("板·撤牌子不剔死（死的照旧拦人）",
     "crates/protocol/src/system/board/core.rs",
     "    pub fn unregister(&mut self, name: Name, who: TaskId) -> Result<(), Fail> {\n        let at = self.find(name).ok_or(Fail::Unknown)?;\n        self.sweep_at(at);",
     "    pub fn unregister(&mut self, name: Name, who: TaskId) -> Result<(), Fail> {\n        let at = self.find(name).ok_or(Fail::Unknown)?;\n        // 变异：不扫"),
    # ── 会话（session-case：码头/泊位/认领）─────────────
    ("会·seat 不挡同名（同一位同记号能有两枚）",
     "crates/protocol/src/session/core.rs",
     "        if self.find(name).is_some() {\n            return Err(Seat::NoName);\n        }\n\n        // 本端那一枚：先铸",
     "        // 变异：不挡同名\n\n        // 本端那一枚：先铸"),
    ("会·扫表不排除已经用掉的那几枚",
     "crates/protocol/src/session/core.rs",
     "            if h.owner != Some(of)\n                || h.mark != mark\n                || piers\n                    .iter()\n                    .any(|p| p.hole == h.token || p.at_peer == Some(h.token))\n            {",
     "            if h.owner != Some(of) || h.mark != mark {"),
    ("会·两格判据只看记号（不看谁开的）",
     "crates/protocol/src/session/core.rs",
     "            if h.owner != Some(of)\n                || h.mark != mark",
     "            if h.mark != mark"),
    ("会·拆泊位不告诉对方",
     "crates/protocol/src/session/core.rs",
     "        let _ = p.post(&call::UNSEAT);",
     "        // 变异：不说那一句"),
    ("会·额度为零时不早退（Partial 塌成 Timeout，且白等一场）",
     "crates/protocol/src/session/core.rs",
     "        if self.piers.is_empty() {\n            // 我一条都没 seat 出去 ⇒ 没有额度可认领（对方无从知道该给我几条）。\n            return Err(Claim::Partial);\n        }",
     "        if false {\n            return Err(Claim::Partial);\n        }"),
    # ── 编排（system-case：账 / 判定 / 配给）─────────────
    ("编·登记不查重名（同名能有两行）",
     "crates/protocol/src/system/desk.rs",
     "        if self.find(name).is_some() {\n            return Err(Fail::Unknown);\n        }",
     "        // 变异：不查重名"),
    ("编·摘身子把行也删了",
     "crates/protocol/src/system/desk.rs",
     "        if let Some(s) = self.row_mut(name) {\n            s.slot = Slot::None;\n            s.root = None;\n        }",
     "        if let Some(s) = self.row_mut(name) {\n            s.slot = Slot::None;\n            s.root = None;\n            s.name = Name::EMPTY;\n        }"),
    ("编·`Dead` 也算不该起（重发那一格没了）",
     "crates/protocol/src/system/core.rs",
     "        State::NeverStarted | State::Dead => Ok(()),",
     "        State::NeverStarted => Ok(()),\n        State::Dead => Err(Fail::NotReady),"),
    ("编·不宣布的那种也当「还没起来」",
     "crates/protocol/src/system/core.rs",
     "            (Announce::None, Slot::Live { .. }) => Ready::Up,",
     "            (Announce::None, Slot::Live { .. }) => Ready::Pending,"),
    # ── 帧（line / principal / coalition 三份，这一刀新上宿主的那一栏）──
    ("帧·线那一格：长度不对也照收（猜）",
     "crates/protocol/src/driver/line/call.rs",
     "    if frame.len() != OCCUPY_LEN || frame[0] != OCCUPY {",
     "    if frame.is_empty() || frame[0] != OCCUPY {"),
    ("帧·判别号不认识也照收",
     "crates/protocol/src/driver/line/call.rs",
     "    Key::from_bytes(raw)\n}",
     "    Some(Key::region(u64::from_le_bytes(raw[..8].try_into().ok()?)))\n}"),
    ("帧·游标那一格不加一（零号被当成「没有」）",
     "crates/protocol/src/coalition/frame.rs",
     "        Some(at) => at.get() as u64 + 1,",
     "        Some(at) => at.get() as u64,"),
    ("帧·窗答不看条数（帧长与条数对不上也读）",
     "crates/protocol/src/coalition/frame.rs",
     "    if count > WINDOW_CAP || body.len() != count * 8 {",
     "    if count > WINDOW_CAP {"),
    ("帧·「有没有」那一格不用了（根被读成没有）",
     "crates/protocol/src/principal/frame.rs",
     "    out[1] = present as u8;",
     "    out[1] = 0;"),
    # ── 对照：什么都不改（应当全绿）──────────────────────
    ("对照·只改注释",
     "crates/protocol/src/operator/core.rs",
     "/// **开**：第 `id` 格**是谁的门牌**（那一枚句柄的开者）。",
     "/// **开**：第 `id` 格**是谁的门牌**（那一枚句柄的开者）。  "),
]


# ── 机器那一侧（`--boot`）：每条变异跑一轮 soak ──────────────────
#
# 为什么单列：宿主靶那条路一两秒一轮，**机器这台一轮要重建 + 起一次 QEMU**（约两分钟）。
# 故这里只放几条**要害**：改一处程序的读数/判词/构造，看 soak 红不红、**报的是哪一条**。
BOOT = [
    ("机器·`foreign` 那一格改成公开（该 8 会答 0）",
     "programs/src/user/probe_rule.rs",
     "            Rule::Opens(principal),",
     "            Rule::Public,"),
    ("机器·撤掉问话孔的「先找后铸」（该 ask_same=1 会 0）",
     "crates/protocol/src/operator/client.rs",
     """    if let Some(have) = crate::session::call::find(me()?, ASK_MARK) {
        return Ok(have);
    }
""",
     ""),
    ("机器·编排域的判词不落（该有 `system: done`）",
     "programs/src/supervisor/system/main.rs",
     '    service::die(service::E_OK, "system: done")',
     '    service::die(service::E_OK, "system: farewell")'),
    # ── 第三轮新钉的那几条读数，各配一条（读数改了形/值 ⇒ 那条断言该红）──
    ("机器·设备数那一条读数说谎（该 21 报 20）",
     "kernel/src/platform/devices.rs",
     '    crate::putln!("devices: {n} handed to root");',
     '    crate::putln!("devices: {} handed to root", n - 1);'),
    ("机器·「类 → 区」那条翻译换了形（四处一起）",
     "programs/src/supervisor/service.rs",
     '                "system: {} {} -> {:#x}",',
     '                "system: {} {} => {:#x}",'),
    ("机器·递单那一条读数换了形（`paired`/`post` 对调）",
     "programs/src/supervisor/service.rs",
     '        "wire: {} bytes, paired={}, post={}",',
     '        "wire: {} bytes, post={}, paired={}",'),
    ("机器·`passer` 不上板（该 `reg=0` 会答别的）",
     "programs/src/user/passer.rs",
     "    let reg = board::ask(talk, &link, board, bcall::REGISTER, me, entry, MS).unwrap_or(BAD);",
     "    let reg = board::ask(talk, &link, board, bcall::LOOKUP, me, entry, MS).unwrap_or(BAD);"),
    ("机器·房客的判词改了名（该有 `lodger: gone`）",
     "programs/src/user/lodger/main.rs",
     '            "lodger: gone"',
     '            "lodger: farewell"'),
]


def run(cmd, **kw):
    return subprocess.run(cmd, shell=True, cwd=ROOT, capture_output=True, text=True, **kw)


def missing_readings(out):
    """机器那一侧：soak 逐条报出来的"缺这几条"。"""
    return re.findall(r"^  (.+)$", out, re.M)


def failing_tests(out):
    """宿主那一侧：从 `host.sh` 打出的失败行里取用例名（它自己会说"有用例失败"）。"""
    log = re.search(r"（(\S+\.log)）", out)
    names = []
    if log:
        try:
            txt = open(f"{ROOT}/{log.group(1)}", encoding="utf-8", errors="ignore").read()
        except OSError:
            txt = ""
        names = re.findall(r"^test (\S+) \.\.\. FAILED", txt, re.M)
    return names


def sweep(mut_list, cmd_of, tag, parse=None):
    rows = []
    for name, path, old, new in mut_list:
        src = open(f"{ROOT}/{path}", encoding="utf-8").read()
        hits = src.count(old)
        if hits != 1:
            # **这一格是量具自己的牙口**：锚点出现两次时 `replace(…, 1)` 会打在**前一处**，
            # 于是"绿"这个结论说的是别处的代码（实测栽过：`sweep_at` 那个锚点在
            # `unregister` 与 `lookup_after` 里各有一份，我改的是前者，却得出"后者没牙"）。
            rows.append((name, "**补丁不唯一，拒绝应用**", f"锚点出现 {hits} 次"))
            continue
        open(f"{ROOT}/{path}", "w", encoding="utf-8").write(src.replace(old, new, 1))
        p = run(cmd_of())
        out = p.stdout + p.stderr
        detail = ""
        if "error[E" in out or "error: could not compile" in out:
            verdict = "**编译红**（不算牙口）"
        elif p.returncode == 0:
            # 三种名义：**变异**（该红）、**对照**（该绿，量具本身对不对）、**等价**
            #（该绿，因为它不改变可观察行为——实测出来的一档，不是"没牙"）。
            is_ctrl = name.startswith("对照")
            is_equiv = name.startswith("等价")
            if is_ctrl:
                verdict = "绿（对照，理应如此）"
            elif is_equiv:
                verdict = "绿（等价变异，理应如此）"
            else:
                verdict = "**绿**（没牙）"
        else:
            verdict = "红"
            # "红在哪"这一栏是这张表的价值所在，故按模式各解析各的：
            #   机器那一侧 = soak 报的**缺哪几条读数**；宿主那一侧 = 日志里**哪几条用例失败**。
            # 照实记：这一栏原先一律抓"行首两空格的行"，结果抓到的是 host.sh 自己那些
            # `  crates/xxx: test result: ok. …` 汇总行——满栏噪声。
            items = parse(out) if parse else []
            shown = " · ".join(items[:4])
            if len(items) > 4:
                shown += f" … （共 {len(items)} 条）"
            detail = shown[:220] if items else out.strip().splitlines()[-1][:120]
        rows.append((name, verdict, detail))
        run(f"git checkout -- {path}")
    print(f"\n{'变异':<40}{'门':<12}红在哪")
    for n, v, d in rows:
        print(f"{n:<40}{v:<12}{d}")
    red = sum(1 for _, v, _ in rows if v == "红")
    ctrl = [v for n, v, _ in rows if n.startswith("对照")]
    equiv = [v for n, v, _ in rows if n.startswith("等价")]
    n_mut = len(rows) - len(ctrl) - len(equiv)
    line = f"\n{tag}：{red}/{n_mut} 条变异被逮住"
    if ctrl:
        line += "；对照" + ("绿（量具可信）" if ctrl[0].startswith("绿") else f"**红了**（{ctrl[0]}——量具坏了）")
    if equiv:
        line += f"；等价变异 {len(equiv)} 条（预期绿）"
    print(line + "\n")
    return 0


def main():
    """默认量**宿主那一侧**（快：一两秒一轮）；`--boot` 量**机器那一侧**（约两分钟一轮）。

    两边的口径同一条：改一处 → 跑那门 → 记下"红不红、报的是哪一条/哪几条" → 还原。
    """
    boot = "--boot" in sys.argv
    only = next((a for a in sys.argv[1:] if not a.startswith("--")), None)
    dirty = run("git status --porcelain").stdout.strip()
    if dirty:
        print("工作区不干净——本量具会 `git checkout --` 还原，先提交或存起来：\n" + dirty)
        return 1
    if boot:
        sel = [m for m in BOOT if not only or only in m[0]]
        return sweep(sel, lambda: "scripts/soak.sh 1", "机器那一侧（一轮 soak）", parse=missing_readings)
    sel = [m for m in MUTATIONS if not only or only in m[0]]
    return sweep(sel, lambda: "scripts/host.sh", "宿主那一侧（`host.sh`）", parse=failing_tests)


if __name__ == "__main__":
    sys.exit(main())
