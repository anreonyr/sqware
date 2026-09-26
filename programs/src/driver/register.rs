//! register — **登记那一条线**：从树上找到线路由者，把本域那一条线登记下来。
//!
//! **一份源码两处走**（`uart` / `rtc`）：两台驱动都有设备、都要占一条线，而"线 = 区的函数"
//! 那条权威在路由者那边解——客户**只报那一段区**（随配给记录发下来），从不报线号。
//! `router` 自己就是持有者，故它不走这一份（它的登记是门面上那一趟）。
//!
//! 会话是**各域那一条**（同一个域只开一条，见 `driver/uart/adapt/boot.rs` 头注）：本模块不自己开，
//! 只借用调用方那一条。

use env::Wait;
use env::{Name, PieToken};
use protocol::driver::line;
use protocol::session::Quay;
use protocol::system::operator as ocall;
use protocol::system::operator::client as operator;

/// 要找的那位服务（线路由者）在树上的名字。
const SERVICE: &str = "router";

/// 从树上找到线路由者（`/device/router`），把本域那一条线登记下来。
///
/// 坐标是**配给回给本域的那一段区**（本域不写死它）；入口经会话从树上授进来，
/// 泊位由 [`line`] 那一层装。名字先译成号（号才是树的直接坐标），此后按号。
///
/// 失败：`Err(())` = 找不到 / 授不进来 / 占不上——调用方按自己那格死法折
/// （今天两台都折 `Fail::Line`）。
pub fn occupy(
    link: &Quay,
    talk: PieToken,
    key: plan::Key,
    millis: Wait,
) -> Result<line::client::Line, ()> {
    let dir = Name::new(protocol::driver::DIR).map_err(|_| ())?;
    let want = Name::new(SERVICE).map_err(|_| ())?;
    let road = [dir, want];
    // **间接寻址那一手**：名字先译成号（号才是树的直接坐标），此后按号。
    let id = operator::seek(talk, link, &road, millis).map_err(|_| ())?;
    let entry = match operator::find(talk, link, id, millis) {
        // **查不到**与**授不出去**都落进这一格（`find` 的状态那一格说得出是哪一种）。
        Ok((ocall::OK, Some(entry))) => entry,
        _ => return Err(()),
    };
    line::client::Line::occupy(entry, key, millis).map_err(|_| ())
}
