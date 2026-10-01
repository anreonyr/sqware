//! canonical::实现侧 — **控制台那一台**：把控制台当 UNIX 那一套 stdin / stdout，并在本域里扮
//! **终端那一侧的行规程**（**U 态**）。
//! ```text
//!   main.rs              入口（bin）：只剩流程
//!   core/discipline.rs   **纯功能**：行规程（canonical mode 的最小一份）
//!   adapt/               适配（bin）：找控制台 `/svc/drv/uart/{rx,tx}` ＋ 轮转那一圈
//! ```
//! **本域就是一台终端**：ECHO 负责把敲的字显出来、行规程负责行编辑，而**交付的行不再写出去**
//! ——没有下游程序，写一遍只会把每一行在屏幕上显示两次。故本域没有命令行、没有过滤器、没有管道；
//! 本质功能只有一件：**字节流 → 终端认的行**。
//! # 纯功能与适配的分界
//! 行规程的每一条规矩（ICRNL / ECHO / ECHOCTL / ERASE / KILL / EOF、行满截断、收场词）都在
//! `core/discipline.rs` 的 `Discipline::feed` 那一手；`adapt/terminal.rs` 只做"收、喂、写"的轮转
//! （碰孔、碰轮转那一圈），连"攒完清零"都不必记得（账在 `feed` 里清）。故那一圈的壳里**一条行
//! 语义也没有**。
