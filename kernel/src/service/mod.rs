// service — 服务分发（dispatcher）。
//
// 多服务架构：boot 期在 dispatcher 注册表登记服务的 req/rep hole；
// 用户态 Service::connect(ServiceId) 经 envcall Service::Connect
// 拿到 dispatcher 的 req/rep Pies，再向 dispatcher 发 lookup 请求拿
// 目标服务的 Pies。
//
// 模式：dispatcher 闭包独占持有注册表 Arc，生命周期 = dispatcher Task。

pub mod dispatch;
