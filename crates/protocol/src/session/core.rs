//! session 的核心 —— **码头、泊位、失败域，与那几个动作**。
//!
//! 本文件**不碰内核**：判据只有一条可机械检查的纪律——
//!
//! > `core.rs` 里不出现 `runtime::`。
//!
//! 表、额度、齐没齐都在这里说清楚；"扫我的表""铸一枚孔""放下"那几手全在
//! [`call`]。这样会话的规矩喂一张假表就能推理，换载体不必重写。

use alloc::vec::Vec;

use env::{Name, PieToken, TaskId};

use super::call;

// ── 结构 ────────────────────────────────────────────────────

/// 一条路：本端叫它什么 + 本端开的那一枚 + （认下之后）对端那一枚。
///
/// 三者缺一就不能用：名字是双方的坐标，孔是本端**收话**用的那一枚，"对端放给我的那枚
/// 孔的本地句柄"是本端**说话**用的那一枚（它只在**本端这张表**里有意义，故与对端身份
/// 绑在一起存——对端手里那个"种在本端表里的号"本端拿着用不动）。
#[derive(Clone, Copy)]
pub struct Pier {
    name: Name,
    /// 本端那一枚孔：**我读**（对端往它推）。铸它的那一刻，**记号就是 `name`**。
    hole: PieToken,
    /// 对端放给我的那枚孔的**本地句柄**：**我写**（往它推，对端读）。
    ///
    /// 注意不是"种在对端表里的号"——那个号只在对端表里有意义，本端拿它推会被拒。
    /// 本端要写，就得有本端表里的一个句柄，而那一枚**由对端交过来**（对端铸它时记号
    /// 刻的是同一条路的名字，故认领时归得到这条路上）；还没交来时这一格是 `None`。
    at_peer: Option<PieToken>,
}

impl Pier {
    /// 这条泊位叫什么。
    pub fn name(&self) -> Name {
        self.name
    }

    /// 往这条泊位说一句话（对端会收到）。
    ///
    /// 对端那一枚还没交过来（[`Pier::paired`] 为假）时报 `Err(())`——**没有写端就发不
    /// 出去**，不猜、不空转。
    pub fn post(&self, msg: &[u8]) -> Result<(), ()> {
        match self.at_peer {
            Some(at_peer) => call::post(at_peer, msg),
            None => Err(()),
        }
    }

    /// 同 [`Pier::post`]，但**槽满当场答 `Err`**（不等）——给"不能堵在这里"的调用方。
    ///
    /// 孔是单槽：对端还没取走上一条时 `post` 会等（背压）。而**两边同时等**就成死锁——
    /// 各自堵在"往对方的槽里推"上，谁也回不去取自己那一格（实测：路由者投递 ↔ 客户端
    /// 说排空，两边各堵一次，机器当场不动）。故**通知类**的那一路用这一条：它的语义是
    /// "该取一次了"，不是"这条消息必须送达"——推不出去由调用方按幂等的**状态**处理。
    pub fn try_post(&self, msg: &[u8]) -> Result<(), ()> {
        match self.at_peer {
            Some(at_peer) => call::try_post(at_peer, msg),
            None => Err(()),
        }
    }

    /// 本端读的那一枚（诊断用）。
    pub fn hole(&self) -> PieToken {
        self.hole
    }

    /// 本端写的那一枚（诊断用）；还没认下来 = `None`。
    pub fn at_peer(&self) -> Option<PieToken> {
        self.at_peer
    }

    /// 从这条泊位收一句话（有界等：`ms` 属**上限族**，三态口径见 `env::fid` 文件头的定式）。
    ///
    /// 与 [`Pier::post`] 成对：那一边说的是**对端的孔**，这一边收的是**本端的孔**。
    /// 收下来的字节数由返回值给出（`Err(())` = 期限内没等到）。
    pub fn pull(&self, buf: &mut [u8], ms: usize) -> Result<usize, ()> {
        call::pull_own(self.hole, buf, ms)
    }

    /// 这条路能不能走了（两头都齐：本端那一枚 + 对端那一枚）。
    pub fn paired(&self) -> bool {
        self.at_peer.is_some()
    }
}

/// 一条泊位装不上时，坏在哪一步。
///
/// 四个变体对应**四种不同的下一步**。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Seat {
    /// 名字非法（空 / 超 31 字节）/ 同名的那一条已经装上了——调用方写错了。
    NoName,
    /// 铸不出孔（资源）。
    NoHole,
    /// 这枚交不出去：没资格交 / 子集越界 / 对端已不在。
    NoSeed,
    /// 账腾不出来（本端那本账备不下这一格）。
    NoRoom,
}

/// 一批泊位认领不齐时，差在哪。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Claim {
    /// 期限内一笔都没到。
    Timeout,
    /// 到了一些，不齐；**或一条额度都没有**——本端一条泊位都没 `seat` 出去过
    /// （[`Quay::claim`] 开头那一格：对方无从知道该给我几条，这不是"白等"）。
    Partial,
    /// 我的表读不动（枚举本身失败）。
    Unread,
}

/// 一座码头：对端 + 一批泊位。
///
/// **对端只存一份**：同一批泊位都交给同一个人，不必每条各存；`open` 那一刻定下，
/// 此后不变（换对端会让已有的泊位全部失效，故没有那样的动作）。
pub struct Quay {
    peer: TaskId,
    piers: Vec<Pier>,
}

impl Quay {
    /// 起一座码头：对端定下，泊位一条都还没有。
    pub fn open(to: TaskId) -> Quay {
        Quay {
            peer: to,
            piers: Vec::new(),
        }
    }

    /// 对端是谁。
    pub fn peer(&self) -> TaskId {
        self.peer
    }

    /// 按名字找那一条泊位。
    pub fn find(&self, name: Name) -> Option<&Pier> {
        self.piers.iter().find(|p| p.name == name)
    }

    /// 已经认领下来的泊位（对端那一枚到手了的那些）。
    pub fn live(&self) -> impl Iterator<Item = &Pier> {
        self.piers.iter().filter(|p| p.paired())
    }

    // ── 动作 ────────────────────────────────────────────────

    /// 装上一条泊位，并把本端那一枚孔交给对端。
    ///
    /// **记号就是这条泊位的名字**：铸孔那一刻刻上去（`call::mint`），副本过线之后仍然
    /// 是它——故对端认领时按"谁开的 + 记号"就能把这一枚归到同名的这条路上。
    ///
    /// 返**泊位**：以后从它说话。对端随后也会交给本端一枚（刻着**同一条路的名字**）——
    /// 那一枚到的时候，[`Quay::claim`] 把它归位到这条泊位上。
    ///
    /// **交出本身是单向的一步**，故本函数不挂起。
    ///
    /// # Errors
    /// - [`Seat::NoName`] 名字非法 / 同名的那一条已经装上 /
    ///   [`Seat::NoHole`] 铸不出孔 / [`Seat::NoSeed`] 交不出去 /
    ///   [`Seat::NoRoom`] 账腾不出来
    pub fn seat(&mut self, name: Name) -> Result<&Pier, Seat> {
        if name.is_empty() {
            return Err(Seat::NoName);
        }
        // 同名挡在这里：**同一位、同一记号只可能有一枚**（归位那一侧的次序因此不再承担
        // 语义），也挡住"装上两条同名路"这种分不清谁是谁的账。
        if self.find(name).is_some() {
            return Err(Seat::NoName);
        }

        // 本端那一枚：先铸（**记号 = 这条泊位的名字**），再交给对端。
        let hole = call::mint(name).map_err(|()| Seat::NoHole)?;
        // **牌不写了**：交出去的副本与本体**共享同一个槽**（`HoleMeta.slot`，
        // `accord` 只克隆 `Arc`）——牌写进去，本端读就把对端那张一起吃掉，本端不读
        // 就占着单槽挡住对端推来的第一条消息。名字这一层信息改由**孔上的记号**承担
        // （见 [`Quay::scan`]）。
        if call::ship(hole, self.peer).is_err() {
            let _ = call::drop_local(hole);
            return Err(Seat::NoSeed);
        }
        // 本端 seat 的那一枚：写的那一半要等对端把它那一枚交进来（认领），故此刻
        // 还不算配齐（`at_peer` 是 `None`）。
        self.install(name, hole)?;
        self.find(name).ok_or(Seat::NoName)
    }

    /// 拆下一条泊位：告诉对方"这条别用了"，并放下本端那一枚孔。
    ///
    /// **拆不存在的泊位不是错误**：要的结果（它不在这儿）已经成立。
    pub fn unseat(&mut self, name: Name) {
        let Some(at) = self.piers.iter().position(|p| p.name == name) else {
            return;
        };
        let p = self.piers.remove(at);
        // 过线的那一句话。发不出去（它已经不在了 / 写端还没到）也算拆成功——它那边
        // 整张表随它消失。
        let _ = p.post(&call::UNSEAT);
        let _ = call::drop_local(p.hole);
    }

    /// 认领 **`of` 交给我的、刻着 `mark` 的那一枚**：扫表 → 认下 → 归位。
    ///
    /// 判据两格，**都读内核查得到的事实**：
    ///
    /// - `of` = **谁的孔**（`owner`：副本共享同一事实、转手不变）——它**不是**本端这条
    ///   泊位认的对端（[`Quay::peer`]）。两者只在"对端直接把孔交给我"时重合；装配者那
    ///   一侧的孔是**子方**交上来的（子方交给生我者），故那里 `of` = 子方、`peer` = 自己。
    ///   **一座码头只能认自己那一位的孔**：装配者给每个孩子各开一座码头，认错了会把别人
    ///   的孔配到这孩子头上（症状：两边都"配好对了"，可对方永远收不到话）。
    /// - `mark` = **孔上刻的记号**（`call::mint` 刻的那一格 = 铸者那张表里这条路的名字）。
    ///   有它才分得开"**同一位开的多枚孔**"——只按 `owner` 那一格是分不开的。
    ///
    /// **凑不齐就不返回**（不许半条会话）：等到期限还没齐就报 [`Claim`] 的错误码。
    /// 认领下来的泊位留在码头里（那是它的家），用 [`Quay::find`] 取。
    ///
    /// `ms` 属**上限族**（三态口径见 `env::fid` 文件头的定式）。
    pub fn claim(&mut self, of: TaskId, mark: Name, ms: usize) -> Result<(), Claim> {
        if self.piers.is_empty() {
            // 我一条都没 seat 出去 ⇒ 没有额度可认领（对方无从知道该给我几条）。
            return Err(Claim::Partial);
        }

        // **先扫再等**（这一序不能反）：信标是一次事件，先扫过一遍才不会漏掉
        // "等之前就已经落进来"的那一枚。
        let left = ms;
        let deadline =
            (left != usize::MAX).then(|| call::now_ns().saturating_add(left as u64 * 1_000_000));
        loop {
            self.scan(of, mark)?;
            // **到手了没有**：本端认下的写端里，有没有一枚的两格正是 `(of, mark)`——判据与
            // [`Quay::scan`] 的 pick **同源**（都读 `call::reserve` 那两格），故"到手"说的
            // 就是"这一枚"，不必再按名字找一条路：记号是**对端**那张表里这条路的名字，
            // 与本端这条泊位的名字**可以不同**（提示孔那一路就是：那枚刻的是 `tip`，本端
            // 这条泊位叫 `board-tip`）。
            let got = self
                .piers
                .iter()
                .filter_map(|p| p.at_peer)
                .any(|t| call::reserve(t) == (Some(of), mark));
            if got {
                return Ok(());
            }
            let remain = remain_ms(deadline);
            if left == 0 || remain == 0 {
                // 「一笔都没到」与「到了一些、不齐」是两种下一步（前面那种等于白等，
                // 后面那种要接着等剩下的）：两条计数只在这一刻用得上，故不再单独立函数
                // ——额度 = 我装上的条数（本端每一条路都在 `piers` 里），欠账 = 还没配齐
                // 的那几条。
                let quota = self.piers.len();
                let waiting = self.piers.iter().filter(|p| !p.paired()).count();
                return Err(if waiting == quota {
                    Claim::Timeout
                } else {
                    Claim::Partial
                });
            }
            // 有界等（`usize::MAX` = 永久）。返回**只是提示**（`wake` 的正文）：真醒还是
            // 期限到，由下一轮的扫表说了算——故这里不接它的值。
            let _ = call::fall(remain);
        }
    }

    /// 打烊：把这座码头上本端持有的一切放下。
    ///
    /// **半途失败也用它**——不必另有一个"清理"动作：能容忍"没开全"的关张就是清理。
    /// 放下的**都是本端那一枚孔**（seat 装上的、认领时也仍是本端表里的那一枚）；
    /// 对端表里那几枚副本不归我——它们的寿命随对端。对端那一侧的通知归 `unseat`，
    /// 故这里不发任何一句话。
    ///
    /// **本端表里"对端交进来的那一枚写端"（[`Pier::at_peer`]）也不归本函数**：它由
    /// **对端退场**一并带走——副本的 `sire` 指着对端表里那一枚（`gate::accord` 的派生边），
    /// 对端收尾时退出钩子沿 `sire` 反查全世界、把它摘掉（`messenger::reap` 里钩子在
    /// `Reaped` 之前跑）⇒ 本端**看到对端收尾的证据时，它已经不在本端表里了**。
    /// 故这里没有、也不需要"放对端那一枚"的动作（实测见 `rig.rs` 头注的照实记）。
    pub fn shut(&mut self) {
        for p in self.piers.drain(..) {
            let _ = call::drop_local(p.hole);
        }
    }

    /// 齐了没有：**每一条路的两头都齐了**。
    ///
    /// 一条会话通常是**两条**：我装一条（我读）、对方装一条（我写），两条都配齐才叫
    /// 通了——`claim` 会一直等到那一天。
    pub fn ready(&self) -> bool {
        !self.piers.is_empty() && self.piers.iter().all(Pier::paired)
    }

    // ── 结构（不碰内核）──────────────────────────────────────

    /// 登记一条泊位（**只有 `seat` 造它**：装上的那一条先只有本端那一枚）。
    fn install(&mut self, name: Name, hole: PieToken) -> Result<(), Seat> {
        if self.find(name).is_some() {
            return Err(Seat::NoName);
        }
        // 账腾不出来：本端那本账备不下这一格。**在改它之前**先备——失败就地退回。
        self.piers.try_reserve(1).map_err(|_| Seat::NoRoom)?;
        self.piers.push(Pier {
            name,
            hole,
            at_peer: None,
        });
        Ok(())
    }

    // ── 扫我的表 ────────────────────────────────────────────

    /// 扫一遍：把**符合那两格判据的那几枚孔**归位（一枚孔只配一条泊位）。
    ///
    /// 两格都由调用方给（[`Quay::claim`]）：`owner == of` **且** 孔上的记号 == `mark`。
    /// 它们都落在**内核在 `UnsealHole` 那一刻盖/刻的戳**上（开者与记号），副本共享同一
    /// 事实、转手不变，而名字、位置、内容、号码那四样都能被伪造或乱序。
    ///
    /// **归位贴到还没配齐的那几条路上（`Vec` 顺序）**：次序在这里不再承担语义——`seat`
    /// 挡同名 ⇒ **同一位、同一记号只可能有一枚**，认下它之后这条候选就没了。
    ///
    /// **一枚孔只配一条泊位**：本端有几条还没配齐的泊位，就按先后认领几枚对端来的孔；
    /// **已经用掉的那几枚不再认**——本端读的那一枚（[`Pier::hole`]）与我已认下的写端
    /// （[`Pier::at_peer`]）。不排除它们，第二次认领会把第一条泊位的写端再配给下一条
    /// （一座码头有两条泊位时必现：板那条路就长在 `records` 那条旁边）。
    ///
    /// **一枚都没等到就不动**：这一条由"贴到已有的路上"自己成立——枚举里没有候选，
    /// 就一次也不进归位那一步（早绑一次就钉在自己的孔上：那 160 字节再也到不了对端）。
    fn scan(&mut self, of: TaskId, mark: Name) -> Result<(), Claim> {
        let piers = &mut self.piers;
        call::each(|h| {
            // 两格判据 + 已经用掉的那几枚不再认。
            if h.owner != Some(of)
                || h.mark != mark
                || piers
                    .iter()
                    .any(|p| p.hole == h.token || p.at_peer == Some(h.token))
            {
                return Ok(());
            }
            if let Some(p) = piers.iter_mut().find(|p| !p.paired()) {
                p.at_peer = Some(h.token);
            }
            Ok(())
        })
    }
}

/// 死线还剩多少**毫秒**（`None` = 永久）。`fall` 收的是毫秒。
///
/// 向上取整：`1..1_000_000` 纳秒的零头算 1 毫秒（否则会提前判超时）；
/// 只有真到了死线才给 0——那一格是"期限到"的判据。
fn remain_ms(deadline: Option<u64>) -> usize {
    match deadline {
        Some(at) => at
            .saturating_sub(call::now_ns())
            .div_ceil(1_000_000)
            .min(usize::MAX as u64) as usize,
        None => usize::MAX,
    }
}
