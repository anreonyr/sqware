//! session 的核心 —— **码头、泊位、失败域，与那六个动作**。
//!
//! 本文件**不碰内核**：判据只有一条可机械检查的纪律——
//!
//! > `core.rs` 里不出现 `runtime::`。
//!
//! 表、额度、齐没齐都在这里说清楚；"扫我的表""铸一枚孔""放下"那几手全在
//! [`call`]。这样会话的规矩喂一张假表就能推理，换载体不必重写。

use env::{Name, PieToken, TaskId};

use super::call;

// ── 结构 ────────────────────────────────────────────────────

/// 一条泊位：一个名字 + 本端那一枚孔 + 对端那个号。
///
/// 三者缺一就不能用：名字是双方的坐标，孔是本端**收话**用的那一枚，"种在对端表里的号"
/// 是本端**说话**用的那一枚（它只在**对端那张表**里有意义，故与对端身份绑在一起存）。
#[derive(Clone, Copy)]
pub struct Pier {
    name: Name,
    /// 本端那一枚孔：**我读**（对端往它推）。
    hole: PieToken,
    /// 对端放给我的那枚孔的**本地句柄**：**我写**（往它推，对端读）。
    ///
    /// 注意不是"种在对端表里的号"——那个号只在对端表里有意义，本端拿它推会被拒。
    /// 本端要写，就得有本端表里的一个句柄，而那一枚**由对端交过来**（它写名字牌时
    /// 把自己的号捎上，本端 `seat` 时把对方的孔交过去——两条路最后都落到本端这一格）。
    at_peer: PieToken,
    peer: TaskId,
    /// 这一条**是本端 seat 装上的**（false = 对方送来、本端认领的）。
    ///
    /// 它是"谁出的孔"——与 [`Pier::paired`] 是两件事：本端装上的那一条，
    /// 也要等对端把它那一枚交回来才算配齐。
    seated: bool,
    /// 两头都齐了（本端孔 + 对端号）。**认领的判据就是它**。
    paired: bool,
}

impl Pier {
    /// 这条泊位叫什么。
    pub fn name(&self) -> Name {
        self.name
    }

    /// 往这条泊位说一句话（对端会收到）。
    pub fn post(&self, msg: &[u8]) -> Result<(), ()> {
        call::post(self.at_peer, msg)
    }

    /// 本端读的那一枚（诊断用）。
    pub fn hole(&self) -> PieToken {
        self.hole
    }

    /// 本端写的那一枚（诊断用）。
    pub fn at_peer(&self) -> PieToken {
        self.at_peer
    }

    /// 诊断：本端孔号 / 对端号 / 谁送来的 / 本端 seat 的。
    pub fn probe(&self) -> (usize, usize, usize, bool, bool) {
        (
            self.hole.get(),
            self.at_peer.get(),
            self.peer.get(),
            self.seated,
            self.paired,
        )
    }

    /// 从这条泊位收一句话（有界等：`ms` 三态同全篇）。
    ///
    /// 与 [`Pier::post`] 成对：那一边说的是**对端的孔**，这一边收的是**本端的孔**。
    /// 收下来的字节数由返回值给出（`Err(())` = 期限内没等到）。
    pub fn pull(&self, buf: &mut [u8], ms: usize) -> Result<usize, ()> {
        call::pull_own(self.hole, buf, ms)
    }

    /// 对端是谁。
    pub fn peer(&self) -> TaskId {
        self.peer
    }

    /// 这条路能不能走了（两头都齐）。
    pub fn paired(&self) -> bool {
        self.paired
    }
}

/// 一条泊位装不上时，坏在哪一步。
///
/// 四个变体对应**四种不同的下一步**；与 [`Claim`] 是同一件事在两侧的镜像。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Seat {
    /// 名字非法（空 / 超 31 字节）——调用方写错了。
    NoName,
    /// 铸不出孔（资源）。
    NoHole,
    /// 这枚交不出去：没资格交 / 子集越界 / 对端已不在 / 还没定对端。
    NoSeed,
    /// 这座码头的额度用完了。
    Full,
}

/// 一批泊位认领不齐时，差在哪。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Claim {
    /// 有孔没带名字（对方违反了不变量）。
    Nameless,
    /// 期限内一笔都没到。
    Timeout,
    /// 到了一些，不齐。
    Partial,
    /// 认领途中对端没了。
    NoPeer,
}

/// 一座码头：对端 + 一批泊位。
///
/// **对端只存一份**：同一批泊位都交给同一个人，不必每条各存。
pub struct Quay {
    peer: Option<TaskId>,
    piers: [Option<Pier>; Quay::CAP],
}

impl Quay {
    /// 一座码头最多几条泊位。条数是策略，容器要有界——与 Service 表同款。
    pub const CAP: usize = 8;

    /// 空码头：还没定对端（用 [`Quay::open`] 定它）。
    pub const fn new() -> Quay {
        Quay {
            peer: None,
            piers: [const { None }; Quay::CAP],
        }
    }

    /// 起一座码头：对端定下，泊位一条都还没有。
    pub fn open(to: TaskId) -> Quay {
        let mut q = Quay::new();
        q.peer = Some(to);
        q
    }

    /// 对端是谁（还没定 = `None`）。
    pub fn peer(&self) -> Option<TaskId> {
        self.peer
    }

    /// 定对端。**同一个号重复定是幂等的**（第二次不算错）；**换一个号是错**——那会让
    /// 已有的泊位全部失效。
    ///
    /// 用它的场景：对端是**刚产出来的**那一枚线程（它会把孔交给"生我者"，故对端是它，
    /// 不是产它的那一枚）。故"先产线程、再定对端、然后装上泊位"是正常次序；而调用方
    /// 为了让 `seat` 能用，往往会先定一次——于是"定两次同号"是常态，不该报错。
    pub fn bind(&mut self, to: TaskId) -> Result<(), Seat> {
        match self.peer {
            Some(had) if had == to => Ok(()),
            Some(_) => Err(Seat::NoName),
            None => {
                self.peer = Some(to);
                Ok(())
            }
        }
    }

    /// 按名字找那一条泊位。
    pub fn find(&self, name: Name) -> Option<&Pier> {
        self.piers.iter().flatten().find(|p| p.name == name)
    }

    /// 按名字取一条**复制出来**的泊位。
    ///
    /// 与 [`Quay::find`] 的分工：那一条把借用拴在码头上，这一条把泊位解放出来——要在
    /// "用这条泊位"与"接着改这座码头"之间来回时用它（板服务读一条、答一条就是那样）。
    pub fn find_pier(&self, name: Name) -> Option<Pier> {
        self.find(name).copied()
    }

    /// 已经认领下来的泊位（对端坐标齐了的那些）。
    pub fn live(&self) -> impl Iterator<Item = &Pier> {
        self.piers.iter().flatten().filter(|p| p.at_peer.get() != 0)
    }

    /// 诊断：每条泊位的一句话（名字 / 是不是本端 seat 的 / 配齐没有 / 对端号）。
    ///
    /// 临时读数用；不进正文（会话的结论由 `ready` 说，不由打印说）。
    pub fn probe(&self) -> impl Iterator<Item = (&str, bool, bool, usize)> {
        self.piers
            .iter()
            .flatten()
            .map(|p| (p.name.as_str(), p.seated, p.paired, p.at_peer.get()))
    }

    /// 还空着几条。
    pub fn free(&self) -> usize {
        Quay::CAP - self.piers.iter().flatten().count()
    }

    // ── 六个动作 ────────────────────────────────────────────

    /// 装上一条泊位，并把本端那一枚孔交给对端。
    ///
    /// 返**泊位**：以后从它说话。对端随后也会交给本端一枚——那一枚到的时候，
    /// [`Quay::claim`] 把它归位到同一条泊位上。
    ///
    /// `ms` 保留给"等对端回音"的调用模式（三态：`0` 只探、`usize::MAX` 无期限、
    /// 其余毫秒）；**交出本身是单向的一步**，故本函数不挂起。
    ///
    /// # Errors
    /// - [`Seat::NoName`] 名字非法 / [`Seat::NoHole`] 铸不出孔 /
    ///   [`Seat::NoSeed`] 交不出去（含"还没 `open`"）/ [`Seat::Full`] 额度用完
    pub fn seat(&mut self, name: Name, _ms: usize) -> Result<&Pier, Seat> {
        let Some(peer) = self.peer else {
            // 前置条件：seat 必须在 open 之后。没定对端时按"装不上"报，不 panic。
            return Err(Seat::NoSeed);
        };
        if self.seated(name) {
            return Err(Seat::NoName);
        }
        if self.free() == 0 {
            return Err(Seat::Full);
        }
        if name.is_empty() {
            return Err(Seat::NoName);
        }

        // 本端那一枚：先铸，再交给对端。
        let hole = call::mint().map_err(|()| Seat::NoHole)?;
        // **牌不写了**：交出去的副本与本体**共享同一个槽**（`HoleMeta.slot`，
        // `accord` 只克隆 `Arc`）——牌写进去，本端读就把对端那张一起吃掉，本端不读
        // 就占着单槽挡住对端推来的第一条消息。名字这件事改由对端**按"谁授的"归位**
        // （见 [`Quay::scan`]）。
        let peer_seed = match call::ship(hole, peer) {
            Ok(seed) => seed,
            Err(()) => {
                let _ = call::drop_local(hole);
                return Err(Seat::NoSeed);
            }
        };
        let at_peer = peer_seed;

        // 对方先送来过同名的一条 ⇒ 并成一条（跟认领那侧一样的两枚孔分工：
        // 本端读自己的、写对端放进来的那一枚）。
        if let Some(p) = self.pier_mut(name) {
            p.seated = true;
            p.at_peer = at_peer;
            // **`paired` 只由认领（`scan`）置真**：写的那一枚必须是对端送进来的
            // （本端表里的句柄），而 `at_peer` 此刻只是"种在对端表里的号"。
            p.paired = false;
            return self.find(name).ok_or(Seat::NoName);
        }
        // 本端 seat 的那一枚：写的那一半要等对端送进来（认领），故此刻还不算配齐。
        self.install(name, hole, at_peer, true)?;
        let _ = at_peer;
        if let Some(p) = self.pier_mut(name) {
            p.at_peer = PieToken::NONE;
            p.paired = false;
        }
        self.find(name).ok_or(Seat::NoName)
    }

    /// 拆下一条泊位：告诉对方"这条别用了"，并放下本端那一枚孔。
    ///
    /// **拆不存在的泊位不是错误**：要的结果（它不在这儿）已经成立。
    pub fn unseat(&mut self, name: Name) {
        let Some(slot) = self
            .piers
            .iter_mut()
            .find(|p| p.as_ref().is_some_and(|p| p.name == name))
        else {
            return;
        };
        let Some(p) = slot.take() else {
            return;
        };
        // 过线的那一句话。发不出去（它已经不在了）也算拆成功——它那边整张表随它消失。
        let _ = p.post(&call::UNSEAT);
        let _ = call::drop_local(p.hole);
    }

    /// 认领 **`from` 交给我的那一批**：扫表 → 按先后归位 → **凑齐才算**。
    ///
    /// `from` 是**谁的孔**（判据落在 `owner` 上：副本共享同一事实、转手不变）——它**不是**
    /// 本端这条泊位认的对端（[`Quay::peer`]）。两者只在"对端直接把孔交给我"时重合；装配者
    /// 那一侧的孔是**子方**交上来的（子方交给生我者），故那里 `from` = 子方、`peer` = 自己。
    /// **一座码头只能认自己那一位的孔**：装配者给每个孩子各开一座码头，认错了会把别人的
    /// 孔配到这孩子头上（症状：两边都"配好对了"，可对方永远收不到话）。
    ///
    /// **凑不齐就不返回**（不许半条会话）：等到期限还没齐就报 [`Claim`] 的错误码。
    /// 认领下来的泊位留在码头里（那是它的家），用 [`Quay::find`] 取。
    ///
    /// `ms` 三态：`0` 只探一次、`usize::MAX` 一直等、其余毫秒。
    pub fn claim(&mut self, from: TaskId, ms: usize) -> Result<(), Claim> {
        if self.quota() == 0 {
            // 我一条都没 seat 出去 ⇒ 没有额度可认领（对方无从知道该给我几条）。
            return Err(Claim::Partial);
        }

        let mut left = ms;
        loop {
            self.scan(|h| h.owner == Some(from))?;
            if self.ready() {
                return Ok(());
            }
            if left == 0 {
                return Err(if self.waiting() == self.quota() {
                    Claim::Timeout
                } else {
                    Claim::Partial
                });
            }
            call::nap(call::POLL_MS);
            left = left.saturating_sub(call::POLL_MS);
        }
    }

    /// 一问一答的那一档：装上自己那一枚（**本端读**），等对端把本端**写**的那一枚交进来。
    ///
    /// 与 [`Quay::claim`] 对偶——那一档是**双向**（两侧各装一条，凑齐才算通），这一档只有
    /// 一侧在等：**问的那一侧**装一条、等答复，答的那一侧只管把孔交出来（它用
    /// [`Quay::seat`] + [`Quay::claim`] 那一对，不必再等什么）。用它的场景是"问一句话、
    /// 拿一句答"（[`board`](crate::board) 的 `Query` 就是），此时要的不是会话而是往返。
    ///
    /// **归位按先后，不按名字**：对端交进来的那一枚落在**本端还没配齐的那条泊位**上
    /// （见 [`Quay::scan`]）。故一座码头里"等答复的泊位"只该有一条——问一句装一条码头，
    /// 或者一次只让一条空着；不满足时本动作**超时**（不猜、不错配）。
    ///
    /// **判据是"我没开过的"**（见 [`Quay::scan`]）：它认不出"这一枚是谁给的"，故本端装这一档
    /// 时，表里**只该有对端那一枚外来孔**。别的来路（同域另一枚线程交回来的副本、别人借过来
    /// 的回信孔……）若已经躺在表里，本动作会按登记先后认错那一枚——**症状是两边都"配好对了"，
    /// 问出去的话石沉大海**（实测栽过：入口副本比板那一枚先到）。故这一档要用在**本端表还干净
    /// 的时候**；真要在一张满是外来孔的表上装路，得有比"谁开的"更强的判据（还没有）。
    ///
    /// 返那条泊位：答话从它的 [`Pier::pull`] 取。失败域借 [`Claim`]——本动作与认领判的是
    /// 同一件事（对方那一枚到没到），故不另立一张码表。
    ///
    /// `ms` 三态与全篇一致：`0` 只探一次、`usize::MAX` 一直等、其余毫秒。
    ///
    /// # Errors
    /// - [`Claim::Nameless`] 名字非法 / 铸不出孔
    /// - [`Claim::NoPeer`] 还没定对端 / 交不出去（对端已不在）
    /// - [`Claim::Partial`] 额度用完
    /// - [`Claim::Timeout`] 期限内对方那一枚没来
    pub fn pair(&mut self, want: Name, ms: usize) -> Result<&Pier, Claim> {
        // 装上自己那一枚（**本端读**），它就落在对端表里。
        self.seat(want, 0).map_err(map_seat)?;
        // **不写字条**：曾经往这一枚孔里推一张"名字 + 号"的牌想让对端按名字归位，但
        // 交出去的副本与本体**共享同一个槽**（`HoleMeta.slot`，`accord` 只克隆 `Arc`），
        // 于是本端读就把对端那张一起吃掉、本端不读就占着单槽把对端推来的第一条消息
        // 堵在槽外——两条路都不通。名字这一层信息由**对端按 `owner` 认领**替代。
        // 没定对端就无从等起（`seat` 那一半已经报过一次，这里报第二次是给"光等不问"的用法）。
        if self.peer.is_none() {
            return Err(Claim::NoPeer);
        }
        let me = call::me();
        let mut left = ms;
        loop {
            // 判据是"**不是本端开的**"，不是"是谁开的"：这一档的对端**可能是第三方**
            // ——板那条路就是（客人的孔由装配者转授给板线程，把写端交回来的是板线程，
            // 客人手里根本没有它的号）。故只能认"我没开过的那一枚"。
            self.scan(|h| h.owner.is_some() && h.owner != me)?;
            // 交进来的那一枚归到哪一条由 [`Quay::scan`] 的先后说了算，故这里只认
            // "本端这一条配齐了没有"。
            if self.find(want).is_some_and(Pier::paired) {
                return self.find(want).ok_or(Claim::Partial);
            }
            if left == 0 {
                return Err(Claim::Timeout);
            }
            call::nap(call::POLL_MS);
            left = left.saturating_sub(call::POLL_MS);
        }
    }

    /// 收回认领下来的那几条（本地放下，不必告诉对方——通知那一侧由 `unseat` 发）。
    pub fn reclaim(&mut self) {
        for p in self.piers.iter_mut() {
            if let Some(p) = p.take() {
                // 两条来路放的**都是本端那一枚孔**（seat 装上的、认领来的，本端孔都在手里）；
                // 对端表里那几枚副本不归我——它们的寿命随对端。
                let _ = call::drop_local(p.hole);
            }
        }
    }

    /// 打烊：把这座码头上本端持有的一切放下。
    ///
    /// **半途失败也用它**——不必另有一个"清理"动作：能容忍"没开全"的关张就是清理。
    pub fn shut(&mut self) {
        self.reclaim();
    }

    // ── 结构（不碰内核）──────────────────────────────────────

    /// 登记一条泊位（[`Pier::seated`] 说出它是本端装上的、还是认领来的）。
    fn install(
        &mut self,
        name: Name,
        hole: PieToken,
        at_peer: PieToken,
        seated: bool,
    ) -> Result<(), Seat> {
        if self.find(name).is_some() {
            return Err(Seat::NoName);
        }
        let Some(slot) = self.piers.iter_mut().find(|p| p.is_none()) else {
            return Err(Seat::Full);
        };
        let _ = &mut *slot;
        *slot = Some(Pier {
            name,
            hole,
            at_peer,
            peer: self.peer.unwrap_or(TaskId::new(0)),
            seated,
            paired: at_peer.get() != 0,
        });
        Ok(())
    }

    /// 按名字取一条可改的（认领归位时用：本端 seat 过的、和认领来的都要能改）。
    fn pier_mut(&mut self, name: Name) -> Option<&mut Pier> {
        self.piers.iter_mut().flatten().find(|p| p.name == name)
    }

    /// 这个名字我已经 `seat` 过了。
    fn seated(&self, name: Name) -> bool {
        self.piers
            .iter()
            .flatten()
            .any(|p| p.name == name && p.seated)
    }

    /// 我的额度：我 `seat` 出去几条，就等对方几条送来。
    fn quota(&self) -> usize {
        self.piers.iter().flatten().filter(|p| p.seated).count()
    }

    /// 齐了没有：**我装上的每一条都配齐了**。
    ///
    /// 只数自己装的那几条——对端多送来的（本端没 seat 过的名字）不算欠账：那是
    /// 对端 `seat` 的，它自己会等到本端那一枚。两侧的额度因此各算各的，不必知道对方
    /// 装了几条。
    ///
    /// 一条会话通常是**两条**：我装一条（我读）、对方装一条（我写），两条都配齐才叫
    /// 通了——`claim` 会一直等到那一天。
    pub fn ready(&self) -> bool {
        self.quota() > 0
            && self
                .piers
                .iter()
                .flatten()
                .filter(|p| p.seated)
                .all(|p| p.paired)
    }

    /// 我装了还没配齐的条数（判"一笔都没到"与"到了一些"用）。
    fn waiting(&self) -> usize {
        self.piers
            .iter()
            .flatten()
            .filter(|p| p.seated && !p.paired)
            .count()
    }

    // ── 扫我的表 ────────────────────────────────────────────

    /// 扫一遍：把**符合 `pick` 的那几枚孔**按先后归位（一枚孔只配一条泊位）。
    ///
    /// 判据都由调用方给，因为两个动作认的不是同一件事：
    ///
    /// - [`Quay::claim`]：`owner == from`——认**我认得的那一位**交给我的那一批（"谁的孔"，
    ///   装配者认的是子方、板线程认的是客人）；
    /// - [`Quay::pair`]：`owner` 存在且**不是本端**——一问一答那一档的对端可能是**第三方**
    ///   （板线程替装配者把写端交进来），本端手里没有它的号，故只认"我没开过的"。
    ///
    /// 两处判据都落在 `owner`（内核在 `UnsealHole` 那一刻盖的戳）上：副本共享同一事实、
    /// 转手不变，而名字、位置、内容、号码那四样都能被伪造或乱序。
    ///
    /// 从前这两处是同一个判据（"跳过 `owner == from`"）：父域那一侧把 `from` 传成**自己**，
    /// 于是它碰巧等于"不是我开的"；真有一个不同的对端时（板线程对客人），它会把**正好要认
    /// 的那一枚**跳过去。反过来，只按"不是我开的"认，则装配者给每个孩子各开的那几座码头
    /// 会互相抢孔（`claim` 的正文）——两处因此分开写。
    ///
    /// 三条走不通的判据都实测过，记在这里免得再走回去：
    /// 1. `h.grantor == from`（对端授的）⇒ 一枚都匹配不上：`accord` 副本的 `vestor`
    ///    是**授出方**（本端自己）。
    /// 2. `grantor == 本端`（本端授的）⇒ 选中本端自己 `seat` 铸的那一枚（它正是本端
    ///    授出的副本）⇒ 写端指回自己的槽，对端永远收不到。
    /// 3. 号码最大的一枚 ⇒ 令牌号是**全局**分配的，与"表尾"无关，选中过本端自己那份。
    ///
    /// **一枚孔只配一条泊位**：本端有几条 `seat` 过还没配齐的泊位，就按先后认领几枚对端
    /// 来的孔——**已经用掉的那几枚不再认**（[`Quay::used_holes`]）。不排除它们，第二次
    /// 认领会把第一条泊位的写端再配给下一条（一座码头有两条泊位时必现：板那条路就长在
    /// `records` 那条旁边）。
    fn scan(&mut self, pick: impl Fn(&call::Hole) -> bool) -> Result<(), Claim> {
        let used = self.used_holes();
        let mut free: [Option<PieToken>; Quay::CAP] = [const { None }; Quay::CAP];
        let mut n = 0usize;
        call::each(|h| {
            if !pick(&h) || used.contains(&h.token) {
                return Ok(());
            }
            if let Some(slot) = free.get_mut(n) {
                *slot = Some(h.token);
                n += 1;
            }
            Ok(())
        })?;
        // **一枚都没等到就不动**：`claim` 是循环（`POLL_MS` 一转），下一转再扫。早绑一次
        // 就钉在自己的孔上——那 160 字节再也到不了对端（实测症状：本端 `post` 返回成功，
        // 对端 `pull` 超时）。
        if n == 0 {
            return Ok(());
        }
        let mut k = 0usize;
        for p in self.piers.iter_mut().flatten() {
            if !p.seated || p.paired {
                continue;
            }
            let Some(write_end) = free.get(k).copied().flatten() else {
                break;
            };
            p.at_peer = write_end;
            p.paired = true;
            k += 1;
        }
        Ok(())
    }

    /// 本端**已经用掉**的那几枚孔：每条泊位我读的那一枚（[`Pier::hole`]）+ 我已认下的
    /// 写端（[`Pier::at_peer`]）。判据要排除它们（见 [`Quay::scan`]）。
    fn used_holes(&self) -> [PieToken; Quay::CAP * 2] {
        let mut out = [PieToken::NONE; Quay::CAP * 2];
        let mut i = 0usize;
        for p in self.piers.iter().flatten() {
            if let Some(slot) = out.get_mut(i) {
                *slot = p.hole;
            }
            if let Some(slot) = out.get_mut(i + 1) {
                *slot = p.at_peer;
            }
            i += 2;
        }
        out
    }
}

/// [`Quay::pair`] 的失败域翻译/// [`Quay::pair`] 的失败域翻译/// [`Quay::pair`] 的失败域翻译/// [`Quay::pair`] 的失败域翻译：把"装不上"的四种因翻成"那一条没成"的四种因。
fn map_seat(seat: Seat) -> Claim {
    match seat {
        Seat::NoName => Claim::Nameless,
        Seat::NoHole => Claim::Nameless,
        Seat::NoSeed => Claim::NoPeer,
        Seat::Full => Claim::Partial,
    }
}
