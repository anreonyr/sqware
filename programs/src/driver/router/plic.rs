//! plic — PLIC（中断控制器）的**寄存器面**（设备侧）：只认识这台控制器的寄存器布局。
//!
//! 线数与 context **不是这里读出来的**——那是设备树给的事实（`core/sources.rs`，纯）；
//! 本文件只按它们算地址。它**不认识任何设备**：不知道 `serial@10000000` 后面是串口还是网卡；
//! 也不含服务循环、不含配给、不含投递（那些在 `adapt/` 与 `programs/src/driver/`）。
//!
//! 要哪几样、多少权、以什么形态出去，写在**本域自己开的**需求单里
//! （[`plan::assembly::ROUTER_WANTS`]）：本域收到记录后**按位次**认领自己那几格——**控制器
//! 按类**（`compatible`，编排域读树把类定成那一段区），**设备树本体与门铃按坐标本身**
//! （`Key::dtb()` / `Key::irq()`：它们不是树里的节点）。名字不在本文件里第二遍。

use crate::core::sources::Sources;
use runtime::core::dock::View;

/// 一条线的优先级：恒 1。**0 是"静音"**（见 [`Plic::disable`]），故本值不能是 0。
pub const LINE_PRIORITY: u32 = 1;

// PLIC 寄存器偏移（SiFive 布局；`reg` 给的是整块）。
const PRIORITY: usize = 0x0000_0000;
const ENABLE: usize = 0x0000_2000;
const ENABLE_STRIDE: usize = 0x80;
const CONTEXT: usize = 0x0020_0000;
const CONTEXT_STRIDE: usize = 0x1000;
const THRESHOLD: usize = 0x00;
const CLAIM: usize = 0x04;

/// PLIC 的寄存器视图 + 从树那侧拿来的两个数（线数 / context）。
pub struct Plic {
    view: View,
    /// 本控制器有多少条线（`riscv,ndev`）——按它拒绝越界的线号。
    device_count: u32,
    ctx: u32,
}

impl Plic {
    /// 按树给的事实建寄存器面：视图是本域借映进来的那一页，两个数从 [`Sources`] 来
    /// （**树是那两个数的唯一来路**，本层不自己读树）。
    pub fn new(view: View, facts: &Sources) -> Plic {
        Plic {
            view,
            device_count: facts.device_count(),
            ctx: facts.context(),
        }
    }

    /// 接上一条线：写优先级 + 本 context 的阈值与 enable。
    ///
    /// 阈值恒 0（不卡仲裁）。线上限由控制器自报的 `device_count` 把握——越界不是错误，是"这条线
    /// 不在这台控制器上"，接了也没用。
    pub fn enable(&self, line: u32, priority: u32) {
        if line < 1 || line > self.device_count {
            return;
        }
        self.write(PRIORITY + 4 * line as usize, priority);
        let at = CONTEXT + CONTEXT_STRIDE * self.ctx as usize;
        self.write(at + THRESHOLD, 0);
        let e = ENABLE + ENABLE_STRIDE * self.ctx as usize + 4 * (line / 32) as usize;
        let bits = self.read(e) | 1 << (line % 32);
        self.write(e, bits);
    }

    /// 静音一条线：**`priority = 0`**。
    ///
    /// 这是**可逆静音**，不是把线拆掉：`pending` 照旧置位，但 `claim` 恒 0 ⇒ 本域不再接它；
    /// 把优先级写回 [`LINE_PRIORITY`] 即复原。enable 位留着——线号与设备的绑定没变，
    /// 变的只是"现在有没有人接"。
    ///
    /// 它当刹车的作用是**不白叫醒客户**（实测：临时去掉这一手，短跑里就多出两次
    /// `uart: rang n=0`——设备那一格已经空了，客户被叫起来排到 0 字节）。**"空转成风暴"那
    /// 句话的来源是另一格**：设备里那一格没清（`rtc` 实测：把 `CLEAR_INTERRUPT` 那一手去掉，
    /// 同一段运行里投递从 5 次变 3093 次）。
    ///
    /// **为什么不是"压着不结"**（`deliver` 之后不 `complete`、押到客户排空）：它同样防得住
    /// 白叫醒，但**结是那一格的再武装**——见 [`Plic::complete`]。实测：故意不结 line 10 ⇒
    /// 只投递一次，之后第二次输入再也进不来（回显与停机都没了）。那一格裁在
    /// `protocol::driver::line` 的"静音还是压着不结"一节里。
    pub fn disable(&self, line: u32) {
        if line < 1 || line > self.device_count {
            return;
        }
        self.write(PRIORITY + 4 * line as usize, 0);
    }

    /// 拆线：**把这一条从本 context 摘出去**——优先级归零 + 清掉 enable 位（[`Plic::enable`]
    /// 的反面）。
    ///
    /// 与 [`Plic::disable`] 不是一回事：静音留着 enable 位（复原走 `enable`，那一格的账没动），
    /// 拆线是"这条线不归本域管了"——主人没了才做，要再接上只能重新登记一次（`occupy`）。
    pub fn unwire(&self, line: u32) {
        if line < 1 || line > self.device_count {
            return;
        }
        self.write(PRIORITY + 4 * line as usize, 0);
        let e = ENABLE + ENABLE_STRIDE * self.ctx as usize + 4 * (line / 32) as usize;
        let bits = self.read(e) & !(1 << (line % 32));
        self.write(e, bits);
    }

    /// 领一条线号；**0 = 没有可领的**（不是错误：别的 context 可能已经领走了）。
    pub fn claim(&self) -> u32 {
        self.read(CONTEXT + CONTEXT_STRIDE * self.ctx as usize + CLAIM)
    }

    /// 结一条线（把线号写回去）。
    ///
    /// **它是那一格的再武装，不是礼貌**（QEMU `hw/intc/sifive_plic.c`）：领走那一步（读
    /// `claim`）当场置 `claimed`，而仲裁只看 `pending & ~claimed & enable` ⇒ **结没写回去，
    /// 那一条线就再也不前送**。`claimed` 在那份实现里还是**整台控制器一份**（不像 `enable`
    /// 按 context 各一份）⇒ 一笔没付出去的结把这条线钉死到重启。
    ///
    /// 实测：故意不结 line 10 ⇒ 只投递一次，之后第二次输入再也进不来（回显与停机都没了）。
    ///
    /// 故本域**当场结清**（`disable` 挡在它之前——那一手是"这一条现在有没有人接"），
    /// 而不是把它押到客户排空那一刻。旧注写的是"`complete` 不会把中断带回来"——**照实记**：
    /// 那句话与 `disable` 那一注互相矛盾，且两句都只说了一半；今天两句都按读数重写了。
    pub fn complete(&self, line: u32) {
        self.write(CONTEXT + CONTEXT_STRIDE * self.ctx as usize + CLAIM, line);
    }

    fn read(&self, off: usize) -> u32 {
        // SAFETY: `view` 是 `Dock::open` 的产物——这段已借映进本域；偏移落在 `reg` 区间内。
        unsafe { core::ptr::read_volatile((self.view.base() + off) as *const u32) }
    }

    fn write(&self, off: usize, v: u32) {
        // SAFETY: 同上，只写控制器寄存器。
        unsafe { core::ptr::write_volatile((self.view.base() + off) as *mut u32, v) }
    }
}
