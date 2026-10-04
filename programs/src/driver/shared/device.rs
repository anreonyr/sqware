//! 走一趟设备账（认领）→ 开图 → 交出视图。

use alloc::string::ToString;

use env::{Access, Policy, PieKind, PieToken, Wait};
use protocol::common::path::Path;
use protocol::service::hub;
use protocol::service::hub::Deed;
use protocol::system::operator::Face;
use runtime::core::res::dock::{Dock, View};

use crate::unit::Died;

use crate::driver::shared::fail::Fail;
use env::pie;
use runtime::core::res::pie::{PolePie};

/// 它是**声明**（`const` 可造：三格全是字面量 / 枚举），各驱动写在**自己那一域**里——装配表
#[derive(Clone, Copy)]
pub struct Ask {
    /// 树里认的类：hub 按它把设备归到 `/dev/<类>` 那一块窗格底下，也按它立那一枚**盟**
    pub class: &'static str,
    /// **点名要哪一台**；`None` = 这一类里**头一台**（区首址最小那台，见 Hub::claim）
    pub name: Option<&'static str>,
    /// 什么种类（`Pole` = 一段内存 / `Nole` = 空载荷的信号）
    pub kind: PieKind,
    /// 要多少权（`FETCH` 读、`FETCH_STORE` 读写——设备寄存器面要读写）
    pub access: Access,
    /// **形态**（`ONLY` = 独占：设备 `reg` 段那一枚内核就是那么发的，一枚门闩只许一个使用者）
    pub policy: Policy,
}

/// 只能问主人自己交来的那一枚（内核那一问 `Join` 只许同队或父域，而 hub 与驱动是兄弟）
/// **它不持树那条会话**：树上那几手每次按调用方给的 Face 走（会话归 Context
/// 四个客人各持各的）
/// crate::driver::shared::context::Context
pub struct Hub {
    bond: hub::Face,
    list: hub::Face,
    sensor: PieToken,
}

impl Hub {
    /// 两枚面各找一趟（译号带重试 ＋ 取那一枚，落在 Face::tile 上）：hub 可能落得比
    pub fn find(tree: &Face, died: Died, ms: Wait) -> Result<Hub, Fail> {
        // **路是 `/svc/hub/<面>`**（容器那一段接 `hub` 那一段，末段是那一枚面）——**不是
        // `/svc/drv/...`**：hub 是**服务那一层**里的一位（与驱动平级），故头一段是 `SVC`
        // 而不是驱动那一家两段。
        let hub_road = protocol::common::svc::SVC
            .try_join(hub::NAME)
            .ok_or(Fail::at(died, "name"))?;
        let bond = face_of(
            tree,
            &hub_road
                .try_join(hub::Grant::Bond.name())
                .ok_or(Fail::at(died, "name"))?,
            died,
            ms,
        )?;
        let list = face_of(
            tree,
            &hub_road
                .try_join(hub::Grant::List.name())
                .ok_or(Fail::at(died, "name"))?,
            died,
            ms,
        )?;
        let sensor = pie::unseal_hole(hub::ALIVE_MARK).map_err(|_| Fail::at(died, "hub"))?;
        Ok(Hub { bond, list, sensor })
    }

    /// **认领一台**：报名（Hub::bond，幂等）→ 列册（取名字）→ 树上找那一格 → 认领 ⇒ 一张契
    /// **"哪一台"由"你找的是哪一格"定**（见 protocol::service::hub）：`ask.name = None`
    pub fn claim(&self, tree: &Face, ask: &Ask, died: Died, ms: Wait) -> Result<Deed, Fail> {
        let class = ask.class.to_string();
        self.bond
            .bond(class.clone(), ms)
            .map_err(|_| Fail::at(died, "bond"))?;
        let name = match ask.name {
            Some(want) => want.to_string(),
            None => {
                let window = self
                    .list
                    .list(class.clone(), 0, ms)
                    .map_err(|_| Fail::at(died, "list"))?;
                // 那一台，下一步相同）。
                window.name(0).ok_or(Fail::at(died, "list"))?.clone()
            }
        };
        // 设备那一轴是 `/dev/<类>/<名>`（**顶层那一层**——与 `/svc` 平级，见 hub::DEV）。
        // 轴那一段是常量（`DEV_ROAD`），类与名来自报文 ⇒ 走运行期那一手（答 `None` 就报步名）。
        let road = hub::DEV_ROAD
            .try_join(class.as_str())
            .and_then(|road| road.try_join(name.as_str()))
            .ok_or(Fail::at(died, "name"))?;
        let door = tree
            .tile(&road, ms)
            .and_then(|tile| tile.token(ms))
            .map_err(|_| Fail::at(died, "tree"))?;
        // **认领打的是那一格上挂着的那一枚**（不是 hub 的门面）：hub 据"哪一枚孔响了"认台。
        let Ok(face) = hub::Face::of(door) else {
            return Err(Fail::at(died, "claim"));
        };
        face.claim(ask.kind, ask.access, ask.policy, self.sensor, ms)
            .map_err(|_| Fail::at(died, "claim"))
    }
}

/// 树上找一枚门牌（**沿一条路，答那一枚**）——三个客人（本手两处 ＋ 各域自己那几处）共用
/// **它不吞错**：失败一律折 `Fail::at(died, "hub")`（"设备账那两枚面没找着"），由调用方给号
fn face_of(tree: &Face, road: &Path, died: Died, ms: Wait) -> Result<hub::Face, Fail> {
    let door = tree
        .tile(road, ms)
        .and_then(|entry| entry.token(ms))
        .map_err(|_| Fail::at(died, "hub"))?;
    hub::Face::of(door).map_err(|_| Fail::at(died, "hub"))
}

/// **一台设备的持有者手里那一样**：那一页的映射（域活多久它活多久）
pub struct Device {
    dock: Dock,
}

impl Device {
    /// 契里那一枚门闩 → 一页映射
    /// **失败那一格由调用方命名**（`"device open failed"` / `"docks"`——**步名**
    pub fn open(page: PieToken) -> Result<Device, ()> {
        let dock = Dock::open(PolePie::from_token(page)).map_err(|_| ())?;
        Ok(Device { dock })
    }

    /// 那一页的视图（View 是 `Copy`：常驻那一圈每醒一次取一份）
    pub fn view(&self) -> View {
        self.dock.view()
    }
}
