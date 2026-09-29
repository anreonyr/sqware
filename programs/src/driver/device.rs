//! driver::device — **一台设备**：走一趟设备账（认领）→ 开图 → 交出视图。
//!
//! ```text
//!   Ask      本域要认的那一台：类 ＋（可选）点名 ＋ 要什么权
//!   Hub      设备账那条路：hub 的两枚面（报名 / 列册）＋ 本域那枚报活孔
//!   Device   一台设备：契里那一枚门闩 → 一页映射
//! ```
//!
//! **照实记（这一份从前是什么样）**：它从前是"装配期那一半"的客侧——按本域那张需求单的
//! **长度**收配给（`assemble::take::<N>`）、把一条 `Pair` 开成一页（`Dock::open`）、再把坐标
//! 交出来（`Device::key`）。收配给那条路**整个退了**（见 [`protocol::driver::hub`] 的头注）：
//! "我要驱这一类"今天由本域**自己跑一趟**（报名 → 列册 → 找门 → 认领），于是本文件从
//! "收记录"变成"走那一趟"，而 `Key`（坐标）不再是本域要的东西——**线号**才是（它从契里来，
//! 路由者按它接线）。
//!
//! **它不含设备语义**：读哪个寄存器、FIFO 怎么排、闸门开哪一位——住各域自己的设备模块
//! （`uart.rs` / `rtc.rs` / `plic.rs`）。本文件只办"从设备账把那一页接过来"这一件事。
//!
//! **`Nole`（门铃）不走 [`Device`]**：它没有寄存器页，只有一枚号——要它的那一位直接把契里
//! 那一枚包成 [`NolePie`](runtime::env::mail::NolePie)（见 `driver/router/adapt/boot.rs`）。

use env::{Access, Policy};
use env::{Kind, PieToken, Tag, Wait};
use protocol::driver::hub;
use protocol::driver::hub::Deed;
use protocol::system::operator::client::Face as TreeFace;
use protocol::system::operator::path::Path;
use runtime::core::dock::{Dock, View};
use runtime::env::mail::{self, PolePie};

use crate::program::Died;

use crate::driver::fail::Fail;

/// **本域要认的那一台**：哪一类（树里认的 `compatible`）、要什么权、（可选）**点名**那一台。
///
/// 它是**声明**（`const` 可造：三格全是字面量 / 枚举），各驱动写在**自己那一域**里——装配表
/// 不再读它（装配者今天不替谁认设备），故它不必再住 `program.rs`（那是**装配**声明那一层）。
#[derive(Clone, Copy)]
pub struct Ask {
    /// 树里认的类：hub 按它把设备归到 `/dev/<类>` 那一块窗格底下，也按它立那一枚**盟**。
    pub class: &'static str,
    /// **点名要哪一台**；`None` = 这一类里**头一台**（区首址最小那台，见 [`Hub::claim`]）。
    ///
    /// 点名那一档今天只有两处用：`/dev/boot/{dtb,irq}`（那两件的名字是**常量**，不是树给的）。
    pub name: Option<&'static str>,
    /// 什么种类（`Pole` = 一段内存 / `Nole` = 空载荷的信号）。
    pub kind: Kind,
    /// 要多少权（`FETCH` 读、`FETCH_STORE` 读写——设备寄存器面要读写）。
    pub access: Access,
    /// **形态**（`ONLY` = 独占：设备 `reg` 段那一枚内核就是那么发的，一枚门闩只许一个使用者）。
    pub policy: Policy,
}

/// **设备账那条路**：hub 的两枚面（报名 / 列册）＋ 本域那枚**报活孔**。
///
/// **报活孔是这一族自己铸的**（记号 [`hub::ALIVE_MARK`]）：hub 要判"这一台的主人还在不在"
/// 只能问主人自己交来的那一枚（内核那一问 `Join` 只许同队或父域，而 hub 与驱动是兄弟）。
///
/// **它不持树那条会话**：树上那几手每次按调用方给的 [`TreeFace`] 走（会话归 [`Context`]，
/// 四个客人各持各的）。
///
/// [`Context`]: crate::driver::context::Context
pub struct Hub {
    bond: hub::Face,
    list: hub::Face,
    sensor: PieToken,
}

impl Hub {
    /// **找到设备账那两枚面**（`/svc/hub/{bond,list}`）＋ 铸本域那枚报活孔。
    ///
    /// 两枚面各找一趟（译号带重试 ＋ 取那一枚，落在 [`TreeFace::tile`] 上）：hub 可能落得比
    /// 本域晚（它排在驱动之前，但"先起"与"上树"不是同一步）。
    ///
    /// 失败读数说步名（`"hub"`），号由调用方带（[`Died`]）。
    pub fn find(tree: &TreeFace, died: Died, ms: Wait) -> Result<Hub, Fail> {
        // **路是 `/svc/hub/<面>`**（容器那一段接 `hub` 那一段，末段是那一枚面）——**不是
        // `/svc/drv/...`**：hub 是**服务那一层**里的一位（与驱动平级），故头一段是 `SVC`
        // 而不是驱动那一家两段。
        let hub_road = protocol::system::SVC
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
        let sensor = mail::unseal_hole(hub::ALIVE_MARK).map_err(|_| Fail::at(died, "hub"))?;
        Ok(Hub { bond, list, sensor })
    }

    /// **认领一台**：报名（[`Hub::bond`]，幂等）→ 列册（取名字）→ 树上找那一格 → 认领 ⇒ 一张契。
    ///
    /// **"哪一台"由"你找的是哪一格"定**（见 [`protocol::driver::hub`]）：`ask.name = None`
    /// 时取的是**列册的头一条**——按区升序 ⇒ 正是旧 `site_of` 那条"这一类里 `reg` 首址最小的
    /// 一台"。
    ///
    /// **报名为什么必须在找门之前**：那一格 `/dev/<类>/<名>` 上的许可写着 `Among(c_类)`
    /// （"许驱这一类的那枚盟里的人才找得到"），而把人放进那枚盟的正是报名那一手
    /// （hub 代本域入盟）。次序倒过来 ⇒ 树那一趟答 `DENIED`。
    ///
    /// 失败读数说步名（`"bond"` / `"list"` / `"tree"` / `"claim"`）。
    pub fn claim(&self, tree: &TreeFace, ask: &Ask, died: Died, ms: Wait) -> Result<Deed, Fail> {
        let class = Tag::new(ask.class).ok_or(Fail::at(died, "class"))?;
        self.bond
            .bond(class, ms)
            .map_err(|_| Fail::at(died, "bond"))?;
        let name = match ask.name {
            Some(want) => Tag::new(want).ok_or(Fail::at(died, "class"))?,
            None => {
                let window = self
                    .list
                    .list(class, 0, ms)
                    .map_err(|_| Fail::at(died, "list"))?;
                // **这一类里一台都没有** ⇒ 这一步与"树里没这台"是同一句话的两半（本域拿不到
                // 那一台，下一步相同）。
                *window.name(0).ok_or(Fail::at(died, "list"))?
            }
        };
        // 设备那一轴是 `/dev/<类>/<名>`（**顶层那一层**——与 `/svc` 平级，见 `hub::DEV`）。
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
/// 的那一趟：`tile`（带重试）＋ `token`。
///
/// **它不吞错**：失败一律折 `Fail::at(died, "hub")`（"设备账那两枚面没找着"），由调用方给号。
fn face_of(tree: &TreeFace, road: &Path, died: Died, ms: Wait) -> Result<hub::Face, Fail> {
    let door = tree
        .tile(road, ms)
        .and_then(|entry| entry.token(ms))
        .map_err(|_| Fail::at(died, "hub"))?;
    hub::Face::of(door).map_err(|_| Fail::at(died, "hub"))
}

/// **一台设备的持有者手里那一样**：那一页的映射（域活多久它活多久）。
///
/// **`Dock` 自己带着门闩**：它落出作用域就撤图（[`Dock`] 的 `Drop`）——本域持到退场。
pub struct Device {
    dock: Dock,
}

impl Device {
    /// 契里那一枚门闩 → 一页映射。
    ///
    /// **失败那一格由调用方命名**（`"device open failed"` / `"docks"`——**步名**，
    /// 见 [`crate::driver::fail`] 那一格裁）——本文件不认识域名，也不该认识。
    pub fn open(page: PieToken) -> Result<Device, ()> {
        let dock = Dock::open(PolePie::from_token(page)).map_err(|_| ())?;
        Ok(Device { dock })
    }

    /// 那一页的视图（[`View`] 是 `Copy`：常驻那一圈每醒一次取一份）。
    pub fn view(&self) -> View {
        self.dock.view()
    }
}
