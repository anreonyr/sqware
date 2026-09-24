#!/usr/bin/env python3
"""门的牙口 —— **量具，不是门**：逐条把某处不变量弄坏，看 `scripts/host.sh` 红不红、红在哪一条。

判据只有一条：**一条断言如果在它守的那件事坏掉之后还不红，它就没有牙**。做法是外科式的——
每次只改一处（都是**像样的 bug**，不是语法错），把门跑一遍，记下"红在哪一条用例"，再还原。
另有一条**对照**（只改注释）：它必须绿，否则红的是量具而不是被测的东西。

**它会改源码再还原**，故：

  - 跑之前**工作区必须干净**（下面第一件事就是查这个）；有未提交的改动它会当场退出，
    免得 `git checkout --` 把你正在写的东西抹掉；
  - 每次变异只碰一处、跑完立刻还原；中断了也只要 `git checkout -- <那个文件>`。

跑法（**默认只跑没验过的那几条**——账在 `target/teeth-ledger.json`，跑过一条记一条）：

    python3 scripts/teeth.py            # 只跑"没验过"的（第一次全跑，之后增量）
    python3 scripts/teeth.py --all      # 全跑（换机器 / 想复核时）
    python3 scripts/teeth.py 账         # 只跑名字里带"账"的几条
    python3 scripts/teeth.py --boot 时间 # 机器那一侧，只跑名字里带"时间"的

**照实记（为什么默认增量）**：全量跑一遍宿主那侧约一分钟、机器那侧约半小时——**用户不想等**
（原话："别每次都全量测"）。而每一条变异**只碰一处、锚点是逐字比对**：锚点一改，它的账就自动
失效（键里带锚点的指纹）⇒ "只跑没验过的"与"全跑"在**锚点没动**时等价，锚点一动必然重跑。

**今天两把量具的条数**（口径的锚点，免得各处的数字越飘越远）：

```text
  宿主那一侧（`scripts/teeth.py`）       47 条：45 红 + 1 对照 + 1 等价
  机器那一侧（`scripts/teeth.py --boot`） 28 条：26 红 + 2 等价
```

**这本账灌过一次**：里面每一条都在第 12 轮（宿主 **47 条**全量复核）与第 15 轮（机器 **28 条**
全量：26 红 + 2 **等价**）实测过 ⇒ 现在默认跑是**真空跑**（"这次没有实跑的"）。要复核用 `--all`。

**照实记一：`host.sh` 的日志名精确到秒且是追加写**——本量具第一次跑就栽在这一格：一秒内连跑
两次复用同一份日志，末了那句 `grep -q 'FAILED' "$log"` 扫到的是**上一轮**的失败 ⇒ 连"只改注释"
都被判成红。**已修**（日志名加 `-$$`，见 `scripts/host.sh`）。量具自己也加固了：每轮先清日志、
跨过秒边界。

**照实记三：这一遍量出一条"没牙"的**——`线·投不出去也置忙`（把 `deliver` 里 `post` 的失败
忽略掉）**全门照绿**：宿主靶的 `Pier` 桩恒答 `Ok`，故"推不出去 ⇒ 不置忙"那条契约**量不到**。
已给桩加一个**可关掉的失败开关**（线程局部，同那两台分配器）+ 一条用例
（`line` 靶::a_frame_that_cannot_be_posted_leaves_the_line_idle`），并把这一条变异放进
这一遍的清单里——**复跑它就红了**（17/17）。这正是这个量具的用处：不是"证明门很好"，
是**指出哪一格缺牙**，缺的那一格补上。

**照实记二：第一版有一条变异是"编不过"**（`slots.remove` 的返回值被我多包了 `Some`）——
它也被记成"红"，但那不说明牙口，只说明编译器在岗。故这一格的口径是：**红必须是"门红"，
不是"编译红"**；那一条改成真变异（把槽从表里移走 ⇒ 号=下标的错位）之后，红了三条用例。
"""

import re
import subprocess
import glob
import hashlib
import json
import os
import sys

ROOT = "/home/anreonyr/Develop/sqware"

# (名字, 文件, 原文, 改成)  —— 每一条都必须是**像样的** bug，不是语法错。
MUTATIONS = [
    # ── 树（`operator` 靶）──────────────────────────────
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
    # ── 门禁（`judge` 靶）───────────────────────────────
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
    # ── 线（`line` 靶）──────────────────────────────────
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
    # ── 两本册子（`roster` 靶：名册/谱系 + 盟籍）──────
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
    # ── 板（`board` 靶）────────────────────────────────
    ("板·查名字不剔死（死的照旧答得出）",
     "crates/protocol/src/system/board/core.rs",
     "        mut ship: impl FnMut(PieToken),\n    ) -> Result<PieToken, Fail> {\n        let at = self.find(name).ok_or(Fail::Unknown)?;\n        self.sweep_at(at);",
     "        mut ship: impl FnMut(PieToken),\n    ) -> Result<PieToken, Fail> {\n        let at = self.find(name).ok_or(Fail::Unknown)?;\n        // 变异：不扫"),
    ("板·撤牌子不剔死（死的照旧拦人）",
     "crates/protocol/src/system/board/core.rs",
     "    pub fn unregister(&mut self, name: Name, who: TaskId) -> Result<(), Fail> {\n        let at = self.find(name).ok_or(Fail::Unknown)?;\n        self.sweep_at(at);",
     "    pub fn unregister(&mut self, name: Name, who: TaskId) -> Result<(), Fail> {\n        let at = self.find(name).ok_or(Fail::Unknown)?;\n        // 变异：不扫"),
    # ── 会话（`quay` 靶：码头/泊位/认领）─────────────
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
    # ── 编排（`judgement` 靶：账 / 判定 / 配给）─────────────
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
    # ── 帧（operator / board 两份，这一刀新上宿主的）──────
    ("帧·路数那一格裁成上限（超长的被当成正好）",
     "crates/protocol/src/operator/frame.rs",
     "            out[1] = road.len().min(u8::MAX as usize) as u8;",
     "            out[1] = filled.min(u8::MAX as usize) as u8;"),
    ("帧·规矩那一格认不出的标号不退回 Public（当成一枚号）",
     "crates/protocol/src/operator/frame.rs",
     "        _ => Rule::Public,",
     "        _ => Rule::Is(id),"),
    ("帧·列答不看条数（帧长与条数对不上也读）",
     "crates/protocol/src/operator/frame.rs",
     "    if count > Operator::PANE_CAP || body.len() != count * 8 {",
     "    if count > Operator::PANE_CAP {"),
    ("帧·号答不看长度（多长都读）",
     "crates/protocol/src/operator/frame.rs",
     "    if body.len() != 8 {\n        return Err(BAD);\n    }",
     "    if body.len() < 8 {\n        return Err(BAD);\n    }"),
    ("帧·板那一面：不带的 seed 也照收（零号与「没有」分不开）",
     "crates/protocol/src/system/board/frame.rs",
     "    out[1 + env::wire::NAME_LEN..].copy_from_slice(&seed.to_bytes());",
     "    out[1 + env::wire::NAME_LEN..].copy_from_slice(&7u64.to_le_bytes());"),
    ("帧·板那一面：名字不判尾（整段当名字，补的零也算进去）",
     "crates/protocol/src/system/board/frame.rs",
     "    Name::from_bytes(raw).ok()",
     "    Name::from_slice(&raw).ok()"),
    # ── 供单（`supply` 靶：帧 / 荷载 / 上限）＋搬进 env 的那两个类型 ──
    ("供·两族视图收混族的位（`Access` 收下传递族）",
     "crates/env/src/wire/access.rs",
     "            Some(p) if p.bits() & ACCESS_MASK.bits() != p.bits() => None,",
     "            Some(_p) if false => None,"),
    ("供·单子那一条不看条数（帧长与条数对不上也读）",
     "crates/protocol/src/driver/supply/call.rs",
     "    if n > WANT_MAX || bytes.len() < HEAD_LEN + n * WANT_LEN {",
     "    if n > WANT_MAX {"),
    ("供·回单那一条不看步长（零头也编）",
     "crates/protocol/src/driver/supply/call.rs",
     "    if !records.len().is_multiple_of(PAIR_LEN) {",
     "    if false {"),
    ("供·已知坐标也去查表（`settle` 两格不分）",
     "crates/protocol/src/driver/supply/call.rs",
     "            At::Known(key) => key,",
     "            At::Known(_key) => of(\"\")?,"),
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
    # ── 机器·第二批（补的是 soak 那 91 条断言里先前没有牙的那些）──
    ("机器·客人要的名字树上没有（`find` 与判词两条一起变）",
     "programs/src/user/guest.rs",
     'const WANT: &str = "router";',
     'const WANT: &str = "routr";'),
    ("机器·boot 域把 dtb 那一类数进区（三条读数一起变）",
     "programs/src/supervisor/root/boot.rs",
     "                Some(DTB) => dtb += 1,",
     "                Some(DTB) => region += 1,"),
    ("机器·该出的那一位没出（入替了出 ⇒ `amid`/`band` 两条变）",
     "programs/src/user/member.rs",
     '    say(&format!("member: leave(c0)={}", done(coal.leave(c0, MS))));',
     '    say(&format!("member: leave(c0)={}", done(coal.enter(c0, MS))));'),
    ("机器·回声列名字那一条换了形",
     "programs/src/user/echo.rs",
     '    let _ = debug::put(&format!("echo: list names={names}"));',
     '    let _ = debug::put(&format!("echo: list named={names}"));'),
    ("机器·停机那一句判词不落（`task: all tasks exited`）",
     "kernel/src/work/room/conductor.rs",
     '        putln!("task: all tasks exited, system halted");',
     '        putln!("task: all tasks gone, system halted");'),
    ("机器·计时那行少一格（`tocks` 不报）",
     "kernel/src/work/room/conductor.rs",
     '"timer: late_n={late_n} late_max_ms={max_ms} late_avg_ms={avg_ms} late_max_tick={late_max} traps={} tocks={tocks} mutes={mutes}",',
     '"timer: late_n={late_n} late_max_ms={max_ms} late_avg_ms={avg_ms} late_max_tick={late_max} traps={}",'),
    ("机器·末日那行换了形（`doom:` 改 `doomed:`）",
     "kernel/src/work/room/conductor.rs",
     'putln!("doom: held={held} starved={starved} blocked={blocked} nudged={nudged}");',
     'putln!("doomed: held={held} starved={starved} blocked={blocked} nudged={nudged}");'),
    ("机器·调度那行少一格（`fallback` 不报）",
     "kernel/src/work/room/conductor.rs",
     'putln!("sched: kicks={kicks} fallback={fallback}");',
     'putln!("sched: kicks={kicks}");'),
    ("机器·中断那行多一格（`idle_busy` 报错位）",
     "kernel/src/work/room/conductor.rs",
     'putln!("irq: ring={ring} busy={busy} idle_ring={idle_ring} idle_busy={idle_busy}");',
     'putln!("irq: ring={ring} busy={busy} idle_busy={idle_busy} idle_ring={idle_ring}");'),
    # ── 机器·第三批（清单剩下那几簇读数：policy / sleeper / member / echo / uart / rtc）──
    ("机器·弃那一趟改成领根（`policy: adopt(out)` 那条变）",
     "programs/src/user/subject.rs",
     "        done(face.adopt(PrincipalId::new(OUTSIDE), MS))",
     "        done(face.adopt(PrincipalId::ROOT, MS))"),
    ("机器·「已经过去」那一格改成将来（`sleeper: past=` 那条变）",
     "programs/src/user/sleeper.rs",
     "    let past = refused(clock::arm(face, now.saturating_sub(1_000_000), MS));",
     "    let past = refused(clock::arm(face, now.saturating_add(60_000_000_000), MS));"),
    ("机器·盟号对调（`member: found=` 第二条报第一枚）",
     "programs/src/user/member.rs",
     '    let c1 = coal.found(MS);\n    say(&format!("member: found={}", one_id(c1)));',
     '    let c1 = coal.found(MS);\n    say(&format!("member: found={}", one_id(c0)));'),
    ("机器·回声那一串走错趟（`echo: seq=` 那条变）",
     "programs/src/user/echo.rs",
     "    let seq = serial(&tree, talk);",
     "    let seq = serial(&tree, talk).max(1);"),
    ("机器·uart 把那句话读反（`ier=rx` 改 `ier=tx`）",
     "programs/src/driver/uart/main.rs",
     '    say(&alloc::format!("uart: ier=rx at={base:#x}"));',
     '    say(&alloc::format!("uart: ier=tx at={base:#x}"));'),
    ("机器·rtc 那一行换了形（`armed at=` 改 `armed=`）",
     "programs/src/driver/rtc/main.rs",
     '                        "rtc: armed at={at} ier={} alarm={}",',
     '                        "rtc: armed={at} ier={} alarm={}",'),
    # ── 机器·第四批（清单最后那几簇：三支树探针 / lodger / echo）──
    # 这两条是**等价变异**（预期绿），不是"没牙"：机器上那两支探针**等的就是对方死**
    # （`probe-owner` 有界重试到 `/sys/lease` 接得上为止），故"活着时别人顶不掉"这一格
    # 它根本量不到；而那一格由宿主靶管着——
    # `judge` 靶::a_living_owner_holds_the_slot_and_a_dead_one_does_not`（主人还在场
    # ⇒ 别人顶不掉）与 `a_rebind_rewrites_the_same_line_and_can_give_up_the_slot`
    # （`mine = false` 是**放弃归属** ⇒ 从此谁都能落）。实测：删掉 `mine` 那一位，机器读数
    # 一字不变（`lease land=0 id=…` 照样 0），故记成"等价"。
    ("等价·租赁那一趟不声明归自己（机器读数看不见；宿主靶管着那一格）",
     "programs/src/user/probe_lease.rs",
     "        ocall::Rule::Public,\n        true,",
     "        ocall::Rule::Public,\n        false,"),
    ("等价·接手那一趟反而声明归自己（同上：探针等的就是对方死）",
     "programs/src/user/probe_owner.rs",
     "        ocall::Rule::Public,\n        false,",
     "        ocall::Rule::Public,\n        true,"),
    ("机器·「该被拒」那一趟的判词换了名",
     "programs/src/user/probe_denied.rs",
     'const OK_NOTE: &str = "probe-denied: denied as expected";',
     'const OK_NOTE: &str = "probe-denied: denied";'),
    ("机器·回声去问一枚**铸过的**号（`name miss=true` 变 false）",
     "programs/src/user/echo.rs",
     "    let miss = operator::name(talk, link, EntryId::new(4095), MS).is_err();",
     "    let miss = operator::name(talk, link, EntryId::new(0), MS).is_err();"),
    ("机器·房客的表那一条读数说谎（`pies=9` 报 10）",
     "programs/src/user/lodger/main.rs",
     '    say(&format!("lodger: pies={}", mail::table_size()));',
     '    say(&format!("lodger: pies={}", mail::table_size() + 1));'),
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


def build_said_red(fresh):
    """构建是不是红了——**看那一轮新写出来的日志**，不看工具拿到的 stdout。

    照实记：`soak.sh` 把 `cargo run` 的**构建输出重定向进了每轮日志**（`> "$log" 2>&1`），
    故机器那一侧的编译错误在工具的 stdout 里**看不见**——实测栽过：一条引用未定义变量的变异
    （`member` 那条）编不过，工具却把它记成"红"（红的是 soak 的"无停机行"）。这一格就是补它。
    """
    # **只看这一轮新出现的那些文件**：照实记——第一版按"最新的三份"取，结果读到了上一次
    # 实验留在目录里的陈旧日志（那一份里有编译错误）⇒ 两条明明该红的变异被记成"编译红"。
    for f in fresh or []:
        try:
            txt = open(f, encoding="utf-8", errors="ignore").read()
        except OSError:
            continue
        if "error[E" in txt or "error: could not compile" in txt:
            return os.path.basename(f)
    return ""


LEDGER = f"{ROOT}/target/teeth-ledger.json"


def ledger_load():
    try:
        with open(LEDGER, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def ledger_save(book):
    os.makedirs(os.path.dirname(LEDGER), exist_ok=True)
    with open(LEDGER, "w", encoding="utf-8") as f:
        json.dump(book, f, ensure_ascii=False, indent=1, sort_keys=True)


def key_of(name, path, old):
    """一条变异的身份：**名字 + 文件 + 锚点指纹**。

    锚点一改，指纹就变 ⇒ 那条自动回到"没验过"，不必手工清账。
    """
    return f"{name}|{path}|{hashlib.sha1(old.encode()).hexdigest()[:12]}"


def sweep(mut_list, cmd_of, tag, parse=None, scratch=None, full=False):
    rows = []
    book = ledger_load()
    skipped = 0
    for name, path, old, new in mut_list:
        k = key_of(name, path, old)
        if not full and k in book:
            skipped += 1
            continue
        src = open(f"{ROOT}/{path}", encoding="utf-8").read()
        hits = src.count(old)
        if hits != 1:
            # **这一格是量具自己的牙口**：锚点出现两次时 `replace(…, 1)` 会打在**前一处**，
            # 于是"绿"这个结论说的是别处的代码（实测栽过：`sweep_at` 那个锚点在
            # `unregister` 与 `lookup_after` 里各有一份，我改的是前者，却得出"后者没牙"）。
            rows.append((name, "**补丁不唯一，拒绝应用**", f"锚点出现 {hits} 次"))
            continue
        open(f"{ROOT}/{path}", "w", encoding="utf-8").write(src.replace(old, new, 1))
        before = set(glob.glob(f"{ROOT}/{scratch}")) if scratch else set()
        try:
            p = run(cmd_of())
            out = p.stdout + p.stderr
            fresh = [f for f in glob.glob(f"{ROOT}/{scratch}") if f not in before] if scratch else []
            detail = ""
            from_log = build_said_red(fresh)
            if "error[E" in out or "error: could not compile" in out or from_log:
                verdict = "**编译红**（不算牙口）"
                if from_log:
                    detail = f"构建日志里编不过：{from_log}"
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
            # 只把**真结论**记进账：`编译红` / `补丁不唯一` / `打不上` 都不记（那几条要人管）。
            if verdict.startswith("红") or verdict.startswith("绿"):
                book[k] = verdict
        finally:
            # **无论怎么退出都要还原**：照实记——这一格原先在正常路径末尾，量具自己抛异常
            # （`TypeError`）时把改过的源码留在了工作区，下一次跑直接被"工作区不干净"挡住。
            run(f"git checkout -- {path}")
    if rows:
        ledger_save(book)
    if skipped:
        print(f"（跳过 {skipped} 条：账里已经验过；要全跑加 `--all`）")
    if not rows:
        print(f"\n{tag}：这次没有实跑的（都验过了）——`--all` 全跑，或给个名字只跑那几条。\n")
        return 0
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
    full = "--all" in sys.argv
    only = next((a for a in sys.argv[1:] if not a.startswith("--")), None)
    dirty = run("git status --porcelain").stdout.strip()
    if dirty:
        print("工作区不干净——本量具会 `git checkout --` 还原，先提交或存起来：\n" + dirty)
        return 1
    if boot:
        sel = [m for m in BOOT if not only or only in m[0]]
        return sweep(
            sel,
            lambda: "scripts/soak.sh 1",
            "机器那一侧（一轮 soak）",
            parse=missing_readings,
            scratch="target/soak/*.log",
            full=full,
        )
    sel = [m for m in MUTATIONS if not only or only in m[0]]
    return sweep(sel, lambda: "scripts/host.sh", "宿主那一侧（`host.sh`）", parse=failing_tests)


if __name__ == "__main__":
    sys.exit(main())
