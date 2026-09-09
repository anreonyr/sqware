#![no_std]
#![no_main]

extern crate alloc;

use task::core::datagram::MailDatagram;
use task::env::io::put;
use task::env::mail::{HolePie, HOLE_MTU_MAX};

// datagram_demo: 端口多路复用最小验证。
//
// 自发自收：unseal hole → MailDatagram(port=7) → send_to(11, "hello") →
// recv() → 验证 src=7 + payload 是 "hello"。再发再收验证编解码稳定。
// 然后故意把 dst 端口搞错，确认收端看到不同 src/dst 但仍能解码。
//
// 跨任务场景需要 spawn 子任务 + 两端 Hole 对，本 demo 只验核心编解码 +
// 同 Hole 收发（单 slot 必须交替 push/pull，故串行收发）。

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("datagram_demo\n");

    let hole = HolePie::unseal(HOLE_MTU_MAX).expect("unseal");
    let dg = MailDatagram::new(hole, 7);

    let mut ok = true;

    // V1：自发自收到 dst=11
    let p1: &[u8] = b"hello";
    if dg.send_to(11, p1).is_err() {
        let _ = put("S1!\n");
        ok = false;
    }
    let mut buf = [0u8; HOLE_MTU_MAX];
    match dg.recv(&mut buf) {
        Ok((src, n)) if src == 7 && n == p1.len() && &buf[..n] == p1 => {
            let _ = put("V1\n");
        }
        Ok((src, n)) => {
            let _ = put("F1\n");
            let _ = (src, n); // src/n 触发未使用变量警告
            ok = false;
        }
        Err(_) => {
            let _ = put("F1!\n");
            ok = false;
        }
    }

    // V2：再发再收到 dst=13（不同 dst），验证编解码不串扰
    let p2: &[u8] = b"world";
    if dg.send_to(13, p2).is_err() {
        let _ = put("S2!\n");
        ok = false;
    }
    match dg.recv(&mut buf) {
        Ok((src, n)) if src == 7 && n == p2.len() && &buf[..n] == p2 => {
            let _ = put("V2\n");
        }
        Ok(_) => {
            let _ = put("F2\n");
            ok = false;
        }
        Err(_) => {
            let _ = put("F2!\n");
            ok = false;
        }
    }

    // V3：checksum 破坏——手工发一条坏头，期望 decode 返 BadChecksum
    // 用裸 HolePie 推一条 8 字节头 + 4 字节载荷，但 checksum 故意算错
    let mut bad = [0u8; 12];
    bad[0..2].copy_from_slice(&7u16.to_le_bytes());   // src
    bad[2..4].copy_from_slice(&15u16.to_le_bytes());  // dst
    bad[4..6].copy_from_slice(&12u16.to_le_bytes());  // length
    bad[6..8].copy_from_slice(&0xDEADu16.to_le_bytes()); // 假 checksum
    bad[8..12].copy_from_slice(b"bad!");
    dg.hole().push(&bad).expect("push bad");
    // recv 期望失败
    match dg.recv(&mut buf) {
        Err(_) => {
            let _ = put("V3\n"); // 预期：denied（checksum 不匹配）
        }
        Ok(_) => {
            let _ = put("F3\n"); // 不应通过
            ok = false;
        }
    }

    let _ = dg.seal();
    let _ = put(if ok { "datagram_demo: ok\n" } else { "datagram_demo: fail\n" });
}