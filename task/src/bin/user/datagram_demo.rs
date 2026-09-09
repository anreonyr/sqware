#![no_std]
#![no_main]

extern crate alloc;

use task::core::datagram::MailDatagram;
use task::env::io::put;
use task::env::mail::{HOLE_MTU_MAX, HolePie};

// datagram_demo: 端口多路复用最小验证。
//
// 自发自收：unseal hole → MailDatagram(port=7) → send_to(11, "hello") →
// recv() → 验证 src=7 + payload 是 "hello"。再发再收验证编解码稳定。
// 然后发 100 字节载荷验 length 字段在大 payload 下正确。

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("datagram_demo\n");

    let hole = HolePie::unseal(HOLE_MTU_MAX).expect("unseal");
    let dg = MailDatagram::new(hole, 7);

    let mut ok = true;

    // V1：自发自收到 dst=11，payload = "hello"
    let p1: &[u8] = b"hello";
    if dg.send_to(11, p1).is_err() {
        ok = false;
    }
    let mut buf = [0u8; HOLE_MTU_MAX];
    match dg.recv(&mut buf) {
        Ok((src, n)) if src == 7 && n == p1.len() && &buf[..n] == p1 => {
            let _ = put("V1\n");
        }
        Ok(_) => {
            let _ = put("F1\n");
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

    // V3：100 字节 payload，验证 length 字段在大载荷下正确
    let p3: [u8; 100] = core::array::from_fn(|i| (i & 0xff) as u8);
    if dg.send_to(13, &p3).is_err() {
        ok = false;
    }
    match dg.recv(&mut buf) {
        Ok((src, n)) if src == 7 && n == p3.len() && &buf[..n] == p3 => {
            let _ = put("V3\n");
        }
        Ok(_) => {
            let _ = put("F3\n");
            ok = false;
        }
        Err(_) => {
            let _ = put("F3!\n");
            ok = false;
        }
    }

    let _ = dg.seal();
    let _ = put(if ok {
        "datagram_demo: ok\n"
    } else {
        "datagram_demo: fail\n"
    });
}
