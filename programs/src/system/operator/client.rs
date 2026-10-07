//! Control registers approved paths over the private Operator channel; services request typed publication.

use alloc::vec::Vec;

use env::{Mark, Wait, HoleDir, PieToken, TaskId};
use env::wire::Field;
use ::resource::port::{self, Access, Policy};

use crate::system::common::timing::BOOT_MS;

use system_api::operator::Path;
use ipc::hand::Sender;
use ipc::session::establish;
use system_api::operator::{EntryId, Tip};
use env::pie;
use ::resource::raw::{Hole, pies, reserve};
pub use system_api::operator::{LINK, TIP_MARK};

/// **只走提示之路**：那条路上三形各带一格 `kind`（读者是持树者，它按首格认形状）
fn push(into: PieToken, tip: Tip) -> Result<(), ()> {
    Sender::<Tip>::from_raw(into)
        .send_within(tip, Wait::AtMost(BOOT_MS))
        .map_err(|_| ())
}

/// Consuming a bootstrap request does not prove its mutation succeeded.
fn request(into: PieToken, make: impl FnOnce(PieToken) -> Tip) -> Result<EntryId, &'static str> {
    let host = establish::opened_by(into).ok_or("operator:tip source")?;
    let (back, seed) = establish::lend_out(into, system_api::operator::TIP_BACK)
        .map_err(|_| "operator:tip reply")?;
    struct Back(PieToken);
    impl Drop for Back {
        fn drop(&mut self) {
            let _ = pie::seal(self.0);
            let _ = pie::release(self.0);
        }
    }
    let _back = Back(back);
    let result = (|| {
        let mut request = Sender::<Tip>::from_raw(into);
        request
            .send_within(make(seed), Wait::AtMost(BOOT_MS))
            .map_err(|_| "operator:tip send")?;
        let mut status = [0xff; 9];
        let (len, from) = Hole::from_raw(back)
            .pull(&mut status, Wait::AtMost(BOOT_MS))
            .map_err(|_| "operator:tip ack")?;
        if len != 9 || from != host || status[0] != system_api::operator::OK {
            return Err("operator:tip rejected");
        }
        Ok(EntryId::new(
            u64::from_le_bytes(status[1..].try_into().unwrap()) as usize,
        ))
    })();
    result
}

/// **把一个号推过去**（`TaskId`，8 字节小端）——**树路上那一格**：告客人"答话的是谁"
pub(crate) fn tell(who: TaskId, into: PieToken) -> Result<(), ()> {
    let mut rec = [0u8; TaskId::WIDTH];
    who.store(&mut rec);
    let road = Hole::from_raw(into);
    road.push(&rec, Wait::AtMost(BOOT_MS)).map_err(|_| ())?;
    road.wait(HoleDir::Push, Wait::Forever).map_err(|_| ())?;
    Ok(())
}

/// **持树者在装配者这一侧的状态**：持树者的号 ＋ 它那条提示之路
/// 它们问的是**树的语义**——客人怎么接、提示怎么认——故收进树这一间
#[derive(Default)]
pub struct Tree {
    host: Option<TaskId>,
    tip: Option<PieToken>,
    clients: Vec<(TaskId, establish::Held)>,
}

impl Tree {
    /// 持树者那一枚的号（`None` = 还没起）
    pub fn host(&self) -> Option<TaskId> {
        self.host
    }

    /// Only a live client-issued LINK requests a session; ordering is not a request.

    pub fn wire(&mut self, wiring: Wiring) -> Result<(), &'static str> {
        let Wiring {
            authority,
            faces: [resolve, matches, same],
        } = wiring;
        use protocol::system::identity::Grant;
        let host = self.host.ok_or("no tree yet")?;
        let Some(tip) = self.tip else {
            return Err("no tip");
        };
        let faces = [
            (resolve, Grant::Resolve),
            (matches, Grant::Matches),
            (same, Grant::Same),
        ];
        // 先验整束，再转授。只认装配者指定的来源，不认同 mark 的伪入口。
        for (token, grant) in faces {
            let (_, owner, mark) = reserve(token).map_err(|_| "operator:wire source")?;
            if owner != authority || mark != grant.mark() {
                return Err("operator:wire source");
            }
        }
        let mut seeds = [PieToken::NONE; 3];
        for (at, (token, _)) in faces.into_iter().enumerate() {
            seeds[at] = port::ship(
                token,
                host,
                Access::FETCH | Access::STORE,
                Policy::VEST,
            )
            .map_err(|_| "operator:wire ship")?
            .seed();
        }
        request(tip, |back| Tip::Wired {
            authority,
            resolve: seeds[0],
            matches: seeds[1],
            same: seeds[2],
            back,
        })
        .map(|_| ())
    }

    /// **它就是持树者本身**：认下它那条提示之路，此后客人上树才有路可走
    pub fn adopt(&mut self, host: TaskId, millis: Wait) -> Result<(), &'static str> {
        self.clients.clear();
        self.host = Some(host);
        self.tip = None;
        host_of(host, millis, &mut self.tip)?;
        Ok(())
    }

    /// **这一台是不是持树者**——判据是**它自己交出来的那一件东西**：提示之路上那枚挂在它名下的
    /// `TIP_MARK` 孔（establish::find **只看**，不另铸一枚新的）
    pub fn holds(&self, host: TaskId) -> bool {
        establish::find(host, TIP_MARK).is_some()
    }

    pub(crate) fn mount(
        &mut self,
        placement: &super::Placement,
    ) -> Result<EntryId, &'static str> {
        let road = &placement.road;
        let leaf = (placement.tile.pie != PieToken::NONE).then_some(placement.tile.pie);
        let permit = placement.tile.permit;
        let owner = placement.tile.owner;
        let replace = placement.replace;

        let host = self.host.ok_or("no tree yet")?;
        let tip = self.tip.ok_or("no tip")?;
        let seed = match leaf {
            Some(entry) => port::ship(
                entry,
                host,
                Access::FETCH | Access::STORE,
                Policy::VEST,
            )
            .map_err(|_| "operator:mount ship")?
            .seed(),
            None => PieToken::NONE,
        };
        let result = request(tip, |back| Tip::Plate {
            road: road.to_path_buf(),
            leaf: seed,
            permit,
            owner: owner.unwrap_or(TaskId::new(0)),
            replace,
            back,
        });
        if result.is_err() && seed != PieToken::NONE {
            let _ = request(tip, |back| Tip::Abort {
                road: road.to_path_buf(),
                leaf: seed,
                back,
            });
            let _ = pie::revoke(host, seed);
        }
        result
    }

    pub(crate) fn remove_empty(&mut self, road: &Path) -> Result<(), &'static str> {
        request(self.tip.ok_or("no tip")?, |back| Tip::Empty {
            road: road.to_path_buf(),
            back,
        })
        .map(|_| ())
    }
    pub(crate) fn unmount(&mut self, id: EntryId) -> Result<(), &'static str> {
        request(self.tip.ok_or("no tip")?, |back| Tip::Unplate { id, back }).map(|_| ())
    }
}

/// 把持树者接上一位客人（装配者调用）：**三步**
/// `host` = 持树者的号（service::spawn 交回来的那个，装配者本来就知道它）
/// `tip` = 提示之路在**本task表里**的那一枚（第一次用时认下来，此后逐条传下去）
/// 返 `Err(哪一步)`：名字非法 / 席位满 / 等不到客人那一枚 / 提示孔认不到……对调用方是
/// 同一件事——**这条服务没接上树**——但"死在哪一步"正是装配诊断要的那一格
fn current_request(client: TaskId, reply: PieToken) -> bool {
    establish::find(client, Mark::of(LINK)) == Some(reply)
        && matches!(reserve(reply), Ok((_, owner, mark))
            if owner == client && mark == Mark::of(LINK))
}

fn attach(
    request: (TaskId, PieToken),
    host: TaskId,
    tip: &mut Option<PieToken>,
) -> Result<establish::Held, &'static str> {
    let (client, reply) = request;
    //      答话从客人自己那枚走）＋ 认领**这位客人**交出来的那一枚（记号 = 这条路的名字）。
    //      判据两格（`owner == client` ＋ 记号）与原 `seat` ＋ `claim` 逐字同源。
    if !current_request(client, reply) {
        return Err("operator:gone request");
    }
    let link = establish::Held(establish::accept(reply).map_err(|_| "operator:seat")?);
    // 3. 提示孔（只认一次）→ 把客人那一枚转授给持树者 → 告两边。
    //    要推的正是它（**不是** `host`：那是持树者的号，推不动）。
    let _ = host_of(host, Wait::POLL, tip)?;
    hand(reply, host).map_err(|()| "operator:hand")?;
    // 客人那一侧的一格：**答话的是谁**（持树者的号，8 字节**裸号**——那条路的读者是
    // ipc::session::hear，见 tell）。
    tell(host, reply).map_err(|()| "operator:who")?;
    // 提示在**转授之后**：持树者据此可以按"提示一到，答话路必已在本表里"办事。
    // The counterpart remains owned until the client closes or replaces this LINK.
    push((*tip).ok_or("operator:tip")?, Tip::Guest(client)).map_err(|()| "operator:tell")?;
    Ok(link)
}

/// 认下持树者交回来的那一枚提示孔（**只认一次**）：判据两格——`owner == 持树者`
/// （那一枚是它铸的）**且** 记号 = TIP_MARK。认下来之后本task拿着的就是
/// "往提示之路推客人号 / 协调帧 / 一条路"那一枚
pub fn host_of(
    host: TaskId,
    millis: Wait,
    tip: &mut Option<PieToken>,
) -> Result<TaskId, &'static str> {
    if tip.is_some() {
        return Ok(host);
    }
    // 交给调用方拿着：同一条路上以后每次都往里推客人号 / 协调帧 / 一条路（**同一枚task**用）。
    *tip = establish::claim(host, TIP_MARK, millis);
    if tip.is_none() {
        return Err("operator:tip");
    }
    Ok(host)
}

/// 把**客人交出来的那一枚**转授给持树者
/// 转授的是"客人开的那扇门"（`owner` 是客人），持树者那侧认领时认的正是它
/// 子集只给 `R|W`，**不加 `VEST`**：持树者用这一枚写答话，不需要再授出——一分不多
pub(crate) fn hand(reply: PieToken, host: TaskId) -> Result<(), ()> {
    let hole = Hole::from_raw(reply);
    port::ship(hole.token(), host, Access::FETCH | Access::STORE, Policy::NONE)
        .map(|_| ())
        .map_err(|_| ())
}

pub struct Wiring {
    pub authority: TaskId,
    pub faces: [PieToken; 3],
}

impl Tree {
    pub fn connect(&mut self, connections: &mut Vec<TaskId>) -> Result<(), &'static str> {
        let Some(host) = self.host else {
            return Ok(());
        };
        self.clients
            .retain(|(_, link)| link.tx().is_some_and(|token| reserve(token).is_ok()));
        let mut requests = Vec::new();
        for client in connections.drain(..) {
            requests
                .try_reserve(1)
                .map_err(|_| "operator:request capacity")?;
            requests.push((client, None));
        }
        if requests.is_empty() {
            return Ok(());
        }
        // Enumerate once, retaining the last matching LINK for each client.
        for pie in pies() {
            if pie.mark != Mark::of(LINK) {
                continue;
            }
            for (client, reply) in &mut requests {
                if pie.owner == *client {
                    *reply = Some(pie.token);
                }
            }
        }
        for (client, reply) in requests {
            let Some(reply) = reply else {
                continue;
            };
            let old = self.clients.iter().position(|(task, _)| *task == client);
            if old.is_some_and(|at| self.clients[at].1.tx() == Some(reply)) {
                continue;
            }
            if old.is_none() {
                self.clients
                    .try_reserve(1)
                    .map_err(|_| "operator:client capacity")?;
            }
            let link = match attach((client, reply), host, &mut self.tip) {
                Ok(link) if current_request(client, reply) => link,
                Ok(_) => continue,
                Err(_)
                    if !current_request(client, reply)
                        || env::unit::join(client, Wait::POLL).unwrap_or(true) =>
                {
                    continue;
                }
                Err(why) => return Err(why),
            };
            if let Some(at) = old {
                self.clients[at].1 = link;
            } else {
                self.clients.push((client, link));
            }
        }
        Ok(())
    }
}
