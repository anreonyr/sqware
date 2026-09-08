// service — 服务目录（dispatcher）。
//
// 目录是一个**普通 Service**：它只有一个 req hole，内核面没有它的入口调用
// （class 7 已删除）。到达目录的方式是能力模型里最普通的一条：父任务把入口
// 门闩 `Accord`/落表给子任务，子任务用 `MailCall::Collect` 找出来。
//
// 目录协议见 `docs/dispatch.md`：Register / Unregister / Replace / Resolve /
// Enumerate / Connect 全是 req hole 上的消息，底层只用 `Accord` + `Push/Pull`。
//
// 模式：dispatcher 闭包独占持有注册表 Arc，生命周期 = dispatcher Task。

pub mod dispatch;
