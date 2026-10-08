//! Control registers approved paths over the private Operator channel; services request typed publication.

use ::resource::port::{self, Access, Policy};
use env::{PieToken, TaskId, Wait};

use crate::support::timing::BOOT_MS;

use ::resource::raw::{Hole, reserve};
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

/// **持树者在装配者这一侧的状态**：持树者的号 ＋ 它那条提示之路
/// 它们问的是**树的语义**——客人怎么接、提示怎么认——故收进树这一间
#[derive(Default)]
pub struct Tree {
    host: Option<TaskId>,
    tip: Option<PieToken>,
    connections: super::connection::Connections,
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
        self.connections = super::connection::Connections::default();
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

pub struct Wiring {
    pub authority: TaskId,
    pub faces: [PieToken; 3],
}

impl Tree {
    pub(crate) fn request_connection(&mut self, caller: TaskId) -> Result<(), &'static str> {
        self.connections.request(caller)
    }
    pub(crate) fn connect(&mut self) -> Result<(), &'static str> {
        if let (Some(host), Some(tip)) = (self.host, self.tip) {
            self.connections.maintain((host, tip))?;
        }
        Ok(())
    }
    pub(crate) fn connection_entries(&self) -> impl Iterator<Item = PieToken> + '_ {
        self.connections.entries()
    }
    pub(crate) fn connection_budget(&self) -> Wait {
        self.connections.remaining()
    }
}
