//! 寄存器面（设备侧）：只认识这台控制器的寄存器布局。
//! 也不含服务循环、不含配给、不含投递（那些在 `adapt/` 与 `programs/src/driver/`）。
//! ——**控制器按类**（`compatible`：设备账读树把类定成那一段区），**设备树本体与门铃按名字点名**

use crate::core::sources::Sources;
use runtime::core::res::dock::View;

/// 一条线的优先级：恒 1。**0 是"静音"**（见 Plic::disable），故本值不能是 0
pub const LINE_PRIORITY: u32 = 1;

// PLIC 寄存器偏移（SiFive 布局；`reg` 给的是整块）。
const PRIORITY: usize = 0x0000_0000;
const ENABLE: usize = 0x0000_2000;
const ENABLE_STRIDE: usize = 0x80;
const CONTEXT: usize = 0x0020_0000;
const CONTEXT_STRIDE: usize = 0x1000;
const THRESHOLD: usize = 0x00;
const CLAIM: usize = 0x04;

/// PLIC 的寄存器视图 + 从树那侧拿来的两个数（线数 / context）
pub struct Plic {
    view: View,
    /// 本控制器有多少条线（`riscv,ndev`）——按它拒绝越界的线号
    device_count: u32,
    ctx: u32,
}

impl Plic {
    /// （**树是那两个数的唯一来路**，本层不自己读树）
    pub fn new(view: View, facts: &Sources) -> Plic {
        Plic {
            view,
            device_count: facts.device_count(),
            ctx: facts.context(),
        }
    }

    /// 接上一条线：写优先级 + 本 context 的阈值与 enable
    /// 阈值恒 0（不卡仲裁）。线上限由控制器自报的 `device_count` 把握——越界不是错误，是"这条线
    /// 不在这台控制器上"，接了也没用
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

    /// 静音一条线：**`priority = 0`**
    /// 把优先级写回 LINE_PRIORITY 即复原。enable 位留着——线号与设备的绑定没变
    /// 变的只是"现在有没有人接"
    /// `uart: rang n=0`——设备那一格已经空了，客户被叫起来排到 0 字节）。**"空转成风暴"那
    /// 句话的来源是另一格**：设备里那一格没清（`rtc` 实测：把 `CLEAR_INTERRUPT` 那一手去掉
    /// 同一段运行里投递从 5 次变 3093 次）
    /// **为什么不是"压着不结"**（`deliver` 之后不 `complete`、押到客户排空）：它同样防得住
    /// 白叫醒，但**结是那一格的再武装**——见 Plic::complete。实测：故意不结 line 10 ⇒
    /// 只投递一次，之后第二次输入再也进不来（回显与停机都没了）。那一格裁在
    pub fn disable(&self, line: u32) {
        if line < 1 || line > self.device_count {
            return;
        }
        self.write(PRIORITY + 4 * line as usize, 0);
    }

    /// 拆线：**把这一条从本 context 摘出去**——优先级归零 + 清掉 enable 位（Plic::enable
    /// 的反面）
    /// 与 Plic::disable 不是一回事：静音留着 enable 位（复原走 `enable`，那一格的账没动）
    pub fn unwire(&self, line: u32) {
        if line < 1 || line > self.device_count {
            return;
        }
        self.write(PRIORITY + 4 * line as usize, 0);
        let e = ENABLE + ENABLE_STRIDE * self.ctx as usize + 4 * (line / 32) as usize;
        let bits = self.read(e) & !(1 << (line % 32));
        self.write(e, bits);
    }

    /// 领一条线号；**0 = 没有可领的**（不是错误：别的 context 可能已经领走了）
    pub fn claim(&self) -> u32 {
        self.read(CONTEXT + CONTEXT_STRIDE * self.ctx as usize + CLAIM)
    }

    /// 结一条线（把线号写回去）
    /// **它是那一格的再武装，不是礼貌**（QEMU `hw/intc/sifive_plic.c`）：领走那一步（读
    /// `claim`）当场置 `claimed`，而仲裁只看 `pending & ~claimed & enable` ⇒ **结没写回去
    /// 那一条线就再也不前送**。`claimed` 在那份实现里还是**整台控制器一份**（不像 `enable`
    /// 按 context 各一份）⇒ 一笔没付出去的结把这条线钉死到重启
    /// 实测：故意不结 line 10 ⇒ 只投递一次，之后第二次输入再也进不来（回显与停机都没了）
    pub fn complete(&self, line: u32) {
        self.write(CONTEXT + CONTEXT_STRIDE * self.ctx as usize + CLAIM, line);
    }

    fn read(&self, off: usize) -> u32 {
        unsafe { core::ptr::read_volatile((self.view.base() + off) as *const u32) }
    }

    fn write(&self, off: usize, v: u32) {
        // SAFETY: 同上，只写控制器寄存器。
        unsafe { core::ptr::write_volatile((self.view.base() + off) as *mut u32, v) }
    }
}
