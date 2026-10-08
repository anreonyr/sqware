//! Control registers approved paths over the private Operator channel; services request typed publication.

use alloc::vec::Vec;

use ::resource::port::{self, Access, Policy};
use env::wire::Field;
use env::{PieToken, TaskId, Wait};

use crate::support::timing::BOOT_MS;

use ::resource::raw::{Hole, alive, reserve};
use env::pie;
use ipc::hand::Sender;
use ipc::session::establish;
use system_api::operator::EntryId;
use system_api::operator::Path;
use system_api::operator::TIP_MARK;
use system_api::operator::Tip;

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

/// Deliver the host and the exact Control-side transport seed for this session.
pub(crate) fn tell(info: (TaskId, PieToken), into: PieToken, wait: Wait) -> Result<(), ()> {
    let mut record = [0u8; TaskId::WIDTH + PieToken::WIDTH];
    info.0.store(&mut record[..TaskId::WIDTH]);
    info.1.store(&mut record[TaskId::WIDTH..]);
    Hole::from_raw(into).push(&record, wait).map_err(|_| ())
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

    pub fn wire(&mut self, wiring: Wiring) -> Result<(), &'static str> {
        let Wiring {
            authority,
            faces: [resolve, matches, same],
        } = wiring;
        use system_api::identity::Grant;
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
            seeds[at] = port::ship(token, host, Access::FETCH | Access::STORE, Policy::VEST)
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

    pub(crate) fn mount(&mut self, placement: &super::Placement) -> Result<EntryId, &'static str> {
        let road = &placement.road;
        let leaf = (placement.tile.pie != PieToken::NONE).then_some(placement.tile.pie);
        let permit = placement.tile.permit;
        let owner = placement.tile.owner;
        let replace = placement.replace;

        let host = self.host.ok_or("no tree yet")?;
        let tip = self.tip.ok_or("no tip")?;
        let seed = match leaf {
            Some(entry) => port::ship(entry, host, Access::FETCH | Access::STORE, Policy::VEST)
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

fn current_request(client: TaskId, reply: PieToken) -> bool {
    alive(reply)
        && matches!(reserve(reply), Ok((vestor, owner, mark))
        if vestor == client && owner == client && mark == system_api::operator::LINK_MARK)
}

fn attach(
    request: (TaskId, PieToken),
    host: TaskId,
    tip: &mut Option<PieToken>,
) -> Result<establish::Held, &'static str> {
    let (client, reply) = request;
    if !current_request(client, reply) {
        return Err("operator:gone request");
    }
    host_of(host, Wait::POLL, tip)?;
    let budget = ipc::time::Deadline::new(Wait::AtMost(BOOT_MS));
    let link = establish::Held(establish::accept(reply).map_err(|_| "operator:seat")?);
    let delivered = hand(reply, host).map_err(|()| "operator:hand")?;
    let result = (|| {
        tell((host, link.seed()), reply, budget.remaining()).map_err(|()| "operator:who")?;
        let mut bytes = [0; PieToken::WIDTH];
        let (len, from) = Hole::from_raw(link.rx())
            .pull(&mut bytes, budget.remaining())
            .map_err(|_| "operator:request endpoint")?;
        if from != client || len != PieToken::WIDTH {
            return Err("operator:request source");
        }
        let ask = PieToken::fetch(&bytes).ok_or("operator:request encoding")?;
        if ask == PieToken::NONE {
            return Err("operator:request missing");
        }
        Sender::<Tip>::from_raw((*tip).ok_or("operator:tip")?)
            .send_within(
                Tip::Guest {
                    who: client,
                    reply: delivered,
                    ask,
                },
                budget.remaining(),
            )
            .map_err(|_| "operator:session handoff")
    })();
    if result.is_err() {
        let _ = pie::revoke(host, delivered);
    }
    result?;
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
    *tip = Some(
        establish::claim(host, TIP_MARK, millis)
            .map_err(|_| "operator:tip missing or ambiguous")?,
    );
    Ok(host)
}

/// 把**客人交出来的那一枚**转授给持树者
/// 转授的是"客人开的那扇门"（`owner` 是客人），持树者那侧认领时认的正是它
/// 子集只给 `R|W`，**不加 `VEST`**：持树者用这一枚写答话，不需要再授出——一分不多
pub(crate) fn hand(reply: PieToken, host: TaskId) -> Result<PieToken, ()> {
    let hole = Hole::from_raw(reply);
    port::ship(
        hole.token(),
        host,
        Access::FETCH | Access::STORE,
        Policy::NONE,
    )
    .map(|delivered| delivered.seed())
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
            .retain(|(_, link)| link.tx().is_some_and(alive));
        for client in connections.drain(..) {
            // Every live LINK is a separate bootstrap request, not a singleton role lookup.
            for candidate in ::resource::raw::pies() {
                let reply = candidate.token;
                if !current_request(client, reply)
                    || self
                        .clients
                        .iter()
                        .any(|(_, link)| link.tx() == Some(reply))
                {
                    continue;
                }
                self.clients
                    .try_reserve(1)
                    .map_err(|_| "operator:client capacity")?;
                match attach((client, reply), host, &mut self.tip) {
                    Ok(link) if current_request(client, reply) => self.clients.push((client, link)),
                    Ok(_) => {}
                    Err(_) => programs::debug::put("operator: rejected session bootstrap"),
                }
            }
        }
        Ok(())
    }
}
