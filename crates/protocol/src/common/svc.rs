//! 命名树那几段的共同前缀（一条路：/svc）。

use super::path::Path;

/// **服务那一层那一段路**（一条路：`/svc`）——挂在树上的服务都从它起
pub const SVC: &Path = Path::new("svc");

/// **平台自己那几枚在容器底下那一段**（`sys`）：持树者（`operator`）与三枚内件
/// （名册 / 盟册 / 控制面）都从它起 —— `/svc/sys/{operator,principal,coalition,control}`
pub const SYS: &str = "sys";

/// **那四族共用那段前缀**（`/svc/sys`）：`operator` / `principal` / `coalition` / `control`
/// 各自那一段路（各族自己的 `DIR`）都从它起 —— **只此一处**
pub const DIR: &Path = Path::new("svc/sys");
