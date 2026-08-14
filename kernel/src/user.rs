// 用户态引导 — 首用户空间构建与首次进入用户态（阶段 B：合成 blob 自检）
//
// 阶段 B 不做 ELF 加载：手写 3 指令机器码循环（addi/ecall/j，llvm-mc 核对过
// 字节）映射在用户 VA 0x10000（与 build.rs 注释的"用户 0x10000"规划一致）。
// boot() 构建用户空间、填好 trap-context 帧（sepc/gpr/sstatus）后经 __restore
// 首次进入用户态，永不返回；此后由 ecall/中断驱动 trap 进出链路（trap_handler
// 分发，见 runtime/trap.rs）。
//
// 本模块持唯一用户空间（OnceLock）；阶段 C 演化出 task/scheduler 后，这里的
// 构建逻辑拆给任务创建，USER_SPACE 换成 per-task 持有。

use alloc::boxed::Box;

use crate::lock::OnceLock;
use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame::allocator;
use crate::memory::manager::addr::{PhysAddr, VirtAddr};
use crate::memory::manager::entry::PteFlags;
use crate::memory::manager::space::{Space, SpaceBuilder, TRAP_CONTEXT};
use crate::putln;
use crate::runtime::context::TrapContext;
use crate::runtime::trampoline::restore;

/// 用户程序加载基址（与 build.rs 注释的"用户 0x10000"一致；阶段 C ELF 加载沿用）。
const USER_TEXT_BASE: VirtAddr = VirtAddr::from_raw(0x1_0000);

/// 阶段 B 合成用户程序（llvm-mc 核对）：
///   addi a0, a0, 1   → a0 递增（a0 可作 ecall 参数）
///   ecall            → 触发 scause=8，handler 里 sepc += 4 后回用户
///   j -8             → 跳回 addi，形成无限 ecall 循环
const USER_BLOB: [u8; 12] = [
    0x13, 0x05, 0x15, 0x00, // addi a0, a0, 1
    0x73, 0x00, 0x00, 0x00, // ecall
    0x6f, 0xf0, 0x9f, 0xff, // j -8
];

/// 临时用户栈大小（16 KiB，经堆窗口分配；阶段 C 换 stack_alloc 窗口）。
const USER_STACK_SIZE: usize = 0x4000;

/// 唯一用户空间（阶段 B 单例；缺页解析与阶段 C 前的读取）。
static USER_SPACE: OnceLock<Space> = OnceLock::new();

/// 当前用户空间引用（trap 缺页解析用）。
pub fn user_space() -> &'static Space {
    USER_SPACE.get().expect("user space not booted")
}

/// 构建首用户空间并首次进入用户态（永不返回）。
///
/// 顺序：build 用户空间（seed_user 已写好 user_satp/self_pa）→ 映射文本 → 分配
/// 栈 → 填用户帧（sepc/gpr/sstatus）→ 发布 USER_SPACE → __restore 切进用户态。
pub fn boot() -> ! {
    let space = SpaceBuilder::user()
        .build()
        .unwrap_or_else(|e| panic!("user space build failed: {e}"));

    // 1. 用户程序：文本帧拷贝 blob，映射到 USER_TEXT_BASE（R|X|U）
    let mut text = Box::try_new_in([0u8; PAGE_SIZE], allocator())
        .unwrap_or_else(|_| panic!("user text frame allocation failed"));
    text[..USER_BLOB.len()].copy_from_slice(&USER_BLOB);
    let text_pa = PhysAddr::from_raw(text.as_ptr() as usize);
    space
        .map(
            USER_TEXT_BASE,
            text_pa,
            PAGE_SIZE,
            PteFlags::V | PteFlags::R | PteFlags::X | PteFlags::U | PteFlags::A | PteFlags::D,
        )
        .unwrap_or_else(|e| panic!("user text map failed: {e}"));
    space.track_frame(text);

    // 2. 用户栈：堆窗口立即映射 16 KiB，sp = 栈顶
    let stack_base = space
        .heap_allocate(USER_STACK_SIZE)
        .unwrap_or_else(|e| panic!("user stack alloc failed: {e}"));
    let stack_top = stack_base + USER_STACK_SIZE;

    // 3. 用户 trap-context 帧（seed_user 已设 user_satp/self_pa；补用户上下文）
    let (trap_ctx_pa, _) = space
        .translate(TRAP_CONTEXT)
        .expect("user trap-context frame not mapped");
    let frame = unsafe { &mut *(trap_ctx_pa.as_usize() as *mut TrapContext) };
    let s = riscv::register::sstatus::read().bits();
    frame.sepc = USER_TEXT_BASE.as_usize();
    frame.gpr[2] = stack_top.as_usize();
    frame.gpr[10] = 0; // a0 起始值（blob 从 0 递增）
    // sstatus：SPP=0（回 U 态）| SPIE=1（sret 后开中断）| SIE=0（__restore 到 sret 之间关中断）
    frame.sstatus = (s & !(1 << 1) & !(1 << 8)) | (1 << 5);

    // 4. 发布用户空间（缺页解析等读取），然后进入用户态
    USER_SPACE
        .set(space)
        .unwrap_or_else(|e| panic!("user space already initialized\n{e:#?}"));

    putln!(
        "user: entering user mode @ {:#x}, sp {:#x}",
        frame.sepc,
        stack_top.as_usize()
    );
    restore(frame.self_pa.as_usize())
}
