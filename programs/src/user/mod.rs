//! user — **U 态那一档**：不建域、不读设备、不碰 MMIO，也不转授权。
//! 判据是特权级（唯一声明处：`programs::unit::PROGRAMS` 里这一行的 `kind`）：今天这一档**只剩一台**
//! ——`canonical/`（入口 `main.rs` ＋ 装配声明 `program.rs`），**控制台那一台**：把控制台当 UNIX 那一套
//! stdin / stdout，并在本域里扮**终端那一侧的行规程**（canonical mode：ECHO / ICRNL / ERASE / KILL /
//! EOF），`exit` 或 `^D` 收场。它只走树上那一族客手与 `env` 的调试面，够不着建域那道 S 态门，
//! 故最小特权够用。

pub mod canonical;
