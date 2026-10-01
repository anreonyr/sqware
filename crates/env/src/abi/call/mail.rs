//! call::mail — **Mail 域（class 5，数据轴：消息穿孔）**：调用表（[`MailCall`]）与失败词汇（[`MailFail`]）。

use super::HoleDir;
use crate::abi::wait::Wait;
use crate::wire::{PieToken, TaskId, VirtAddr};
use mold::{Envcall, Fail};

/// Mail 域（class 5：数据轴）的失败词汇。
#[derive(Fail)]
pub enum MailFail {
    /// token 不在表里 / 权不够 / 不是孔（递了铃、页、组）。
    Denied = -1,
    /// 那一枚已封印。
    Dead = -2,
    /// 条件未就绪（孔上已有手／正被取用／位已响／没有可取之事／铃已响）。
    #[busy]
    Busy = -3,
    /// 表项备不下。
    OoM = -4,
    /// 这一枚已交出去（交回即复原）。
    HandedOver = -5,
    /// 递出那只手所依的内存已经没了：发送方那段不可读，或它那个空间已回收。
    ///
    /// 与 `Denied` 分开：`Denied` 是"换够大的缓冲再来"，`Gone` 是"这条路废了"。
    Gone = -6,
}

/// `MailFail` 的结果别名。
pub type MailResult<T> = Result<T, erra::Error<MailFail>>;

/// 通信调用（class 5，mail）—— **数据轴**：消息穿孔 + 门铃。
///
/// 作用在一枚 Hole（孔）上：`Push` 递出一只手、`Pull` 取走一只手、`Peek` 只看一眼、
/// `Ring`／`Hush` 置／清孔上那一位、`Wait` 等方向就绪；
/// 后两个动词也作用在一枚 Nole（门铃）上：`Ring` 响铃、`Hush` 应铃。
/// 权柄的生死与流动不在此类，见 [`PieCall`]（class 7）。
///
/// **wait 的分界**：事件键等待留 Room（`RoomCall::Wait/Wake` 的键是调用方命名空间
/// 里的裸整数，内核不解释）；**资源就绪**等待归本类——`Wait` 收 `token`，由内核
/// 解引用出资源自己的等待键，键不出内核。
///
/// **孔不预设长度**：孔**不持有荷载**——`Push { len }` 递出的是**发送方那段内存的一只手**，
/// `Pull { max }` 取走时把它复制**一次**进收方那段。长度只在取的那一侧被 `max` 判：
/// `len > max` 答 `Denied`，**手原样留在孔上**（不替调用方丢东西，可以换够大的缓冲再取）。
/// 内核每条消息至多搬一遍字节，**没有"一条消息 ≤ 一页"那条界**——比一页大的东西走
/// [`Pole`]（页级共享内存那一轴）仍是**推荐**，不是必需的。
///
/// **两种待取之事**：一只手（`Push`／`Pull`／`Peek`）与一个位（`Ring`／`Hush`）。
/// 位不占字节，故那一路零复制、零分配、也不阻塞发送方。两者写在孔上的**同一格**里，互斥。
/// `Wait` 复用在这三种资源上，靠 `dir` 分：Hole 两个方向，**Nole 只认 `Pull`**。
#[derive(Envcall)]
#[call(class = 5, fail = MailFail)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MailCall {
    /// 递出一只手：token + 发送方那段 VA + 长度（`len ≥ 1`）。
    ///
    /// **不搬字节**：内核在孔上登记 `(发送者, 那段, 长度)`，复制由**取**的一方做（一处）。
    /// `Ok` 只说明**内核收下了这只手**——送达要等 `Wait { dir: Push }` 报"手下线了"。
    ///
    /// **这一格是"一次尝试"**：孔上有手就答 `Busy`，不睡。**"等轮到自己"不在这一格里**——它是
    /// 调用点写出来的（`HolePie::push(msg, within)` 拿 `Wait` 拼一个 `Wait{dir: Push}` 的循环），
    /// 而"等自己那只手被取走"是**再写一次 `Wait`**（写端那一格 `Sender::reclaim`）。两件事在
    /// 内核是同一个条件（孔空），故本类不多开动词。
    #[ret(())]
    Push {
        token: PieToken,
        msg: VirtAddr,
        len: usize,
    },
    /// 取走一只手：token + 收方缓冲 VA + 容量。返 `(实际长度, 发送者 TaskId)`。
    ///
    /// 发送者由**内核在 `Push` 时盖章**（syscall 上下文，不可伪造）——身份不必从报文里猜。
    ///
    /// `max` = **收方自己给的那段区间有多长**（不是申请，是申报）。四条不过的路
    /// **都不消费那只手**：`max` 装不下 `len`／收方那段不可写 ⇒ `-1 Denied`；
    /// 发送方那段已不可读／它那个空间已回收 ⇒ `-6 Gone`。全过了才复制**一次**、清孔、
    /// 唤醒等递的一方。手上没有可取之事 ⇒ `-3 Busy`；孔已封印 ⇒ `-2 Dead`（先过存活闸）。
    /// 这一格正是 [`MailCall::Wait`] 那句"绝不返 `Busy`"的对照面：同一个"未就绪"，
    /// 非阻塞的 `Pull` 用 `Busy` 答、阻塞的 `Wait` 用 `false` 答。
    #[ret((usize, TaskId))]
    Pull {
        token: PieToken,
        buf: VirtAddr,
        max: usize,
    },
    /// 等某方向就绪：`millis`——**上限族**（定式见文件头）。
    ///
    /// 返回 `true` = 本次调用**当场就绪**（未挂起）；`false` = 未就绪（探测失败，
    /// 或挂起过——被唤醒与超时不分）。**绝不返 `-3 Busy`**：未就绪的答案就是 `false`。
    /// 权利：`Pull` 需 R、`Push` 需 W。
    ///
    /// 作用在 Nole（门铃）上时：**`dir` 必须是 `Pull`**——铃只有"响了"这一条方向，
    /// 别的值返 `-1 Denied`（不静默忽略：ABI 不留一个白填的字段）。权利仍按 `dir` 判。
    #[ret(bool)]
    Wait {
        token: PieToken,
        dir: HoleDir,
        millis: Wait,
    },
    /// 清"有待取之事"：门铃上应铃，孔上**清那一位**。权利：R——听与应都在"取"这一侧。
    ///
    /// 未响 ⇒ `-3 Busy`（没有可取之事）。**不唤醒任何人**：没人等"铃不响"。
    /// 门铃那一支顺带重开本 hart 的中断闸门；**孔上那一位不碰闸门**（它不是中断响的）。
    #[ret(())]
    Hush { token: PieToken },
    /// 置"有待取之事"并唤醒听者：门铃上响铃，孔上**置那一位**。权利：W。
    ///
    /// 已响 ⇒ `-3 Busy`——多 hart 同时响合成一位，第二次起不改变状态。
    /// 这个动词是给**自检**与"自己叫自己"的；孔上那一条（板推死亡道那一类）也走它：
    /// **发送方不睡**，故"通知"这一类不会把两台机器的进度互锁。
    /// 内核响中断那道门铃不走这里（它持着源实体，见 `devices.rs`）。
    #[ret(())]
    Ring { token: PieToken },
    /// 只看那只手：`(长度, 发送者, 队里排着几只)`——**一个字节都不取**。
    ///
    /// 不动孔的状态（**取用中的那只也照报**），也不唤醒任何人。手上没有东西 ⇒ `-3 Busy`。
    /// **不是取消息的前一步**：取走就是一次 `Pull`，够不够由 `max` 判。
    /// 它的读者是"等之前先看一眼"那一格（`harness` 的 waiter）。
    ///
    /// **第三格是队列深度**（a2；这一格此前空着 ⇒ 加一格，a0／a1 的含义一字不动）：一只孔上
    /// 可以排着至多 `QUEUE_CAP` 只手，写者据此知道"我还排着几手"（`hand::Sender` 靠它把缓冲收回来）。
    /// 三件事两格装不下 ⇒ 走**宽返回**那一档（`#[ret3]`，与 `PieCall::Collect` 同一条路）。
    #[ret3((usize, TaskId, usize))]
    Peek { token: PieToken },
    //     #[ret(())] Withdraw { token: PieToken },
}
