use alloc::string::{String, ToString};
use env::{Mark, PieToken};

/// **铸某一面的待客入口**，并交出它**自己那一段名字**（`/svc/{族}/{面名}` 的末段）
/// 由记号创建 Hole，由面名建立字符串。
/// 返 `Err(哪一步)`：`"grant"`（记号铸不出）或 `"name"`（面名非法）。对调用方是同一件事
/// （这一面没挂上），但"死在哪一步"正是诊断要的那一格
/// **每一面只铸一枚**：这一枚此后就是 `/svc/{族}/{面名}` 那一格背后那一枚；铸第二枚就会有一枚
pub fn entry(mark: Mark, name: &'static str) -> Result<(PieToken, String), &'static str> {
    let entry = env::hole::unseal(mark, env::HoleLimits::default()).map_err(|_| "grant")?;
    let name = name.to_string();
    Ok((entry, name))
}
