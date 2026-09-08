//! Service 域：用户态与内核驻留服务交互（`Service::connect` + `echo`）。
//!
//! 路径 B（dispatcher）：
//! 1. `Service::connect(sid)` 经 `ServiceCall::Connect` 拿到 dispatcher 的 req/rep Pies
//! 2. push(sender_id + service_name) 到 dispatcher req
//! 3. pull dispatcher rep 拿到目标服务的 Pies
//! 4. 用目标服务 Pies 构造 Service（持 req + rep HolePie）
//! 5. `Service::echo` push req → pull rep（与 §12 同）
//!
//! 与 `ipc` 无涉——用 sqware 词族 `Service`。

use ubi::{EnvResult, ServiceCall, ServiceCallRet, ServiceId, TaskId, make_err};

use super::mail::HolePie;
use super::task;

/// 用户态服务通道：两个 HolePie（req + rep）。
pub struct Service {
    req: HolePie,
    rep: HolePie,
}

impl Service {
    /// 连接到指定服务：先拿 dispatcher 的 Pies，再 lookup。
    pub fn connect(sid: ServiceId) -> EnvResult<Service> {
        // 1. envcall 拿 dispatcher Pies
        let (dreq_tk, drep_tk) = match (ServiceCall::Connect { service: sid }).call()? {
            ServiceCallRet::Connect((a, b)) => (a.get(), b.get()),
        };
        let dreq = HolePie::from_token(dreq_tk);
        let drep = HolePie::from_token(drep_tk);

        // 2. 构造 lookup 请求
        let my_id: TaskId = task::self_id()?;
        let mut req_msg = [0u8; 64];
        req_msg[0..8].copy_from_slice(&(my_id.get() as u64).to_le_bytes());
        let name = sid.name_bytes();
        let n = name.len().min(55);
        req_msg[8..8 + n].copy_from_slice(&name[..n]);

        // 3. push dispatcher 请求（短 spin 等 wake）
        dreq.push(&req_msg)?;

        // 4. pull dispatcher 回复
        let mut reply = [0u8; 64];
        drep.pull(&mut reply)?;
        // 5. 解析服务 Pies（0 = 未找到）
        let req_tk = u64::from_le_bytes(reply[0..8].try_into().unwrap_or([0u8; 8]));
        let rep_tk = u64::from_le_bytes(reply[8..16].try_into().unwrap_or([0u8; 8]));
        if req_tk == 0 || rep_tk == 0 {
            return Err(make_err(ubi::EnvError::from_raw(-2))); // not found
        }
        Ok(Service {
            req: HolePie::from_token(req_tk),
            rep: HolePie::from_token(rep_tk),
        })
    }

    /// 一次 echo：caller push(req) → 等 echo 处理 → echo push(rep) → caller pull(rep)。
    /// 阻塞语义由内核 push/pull 实现（park + wake）。
    pub fn echo(&self, req: &[u8; 64]) -> EnvResult<[u8; 64]> {
        self.req.push(req)?;
        let mut reply = [0u8; 64];
        self.rep.pull(&mut reply)?;
        Ok(reply)
    }
}
