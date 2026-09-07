#![no_std]
#![no_main]

use user::core::task::self_id;
use user::env::io::put;
use user::env::task::sire;

// sire_demo: 血缘溯源验证——作为 spawn_team 的子镜像运行，读自己的 self_id 与
// sire（生我者的 task id）。
//
//   self_id() → 本 task id（> 0，是 spawn_task 在子域里产出的线程）
//   sire()    → 生我者的 task id（= 执行 spawn_team 的父 task；> 0 表示血缘成立）
//
// 期望：sire() 返回的 id 与父（shell 里 spawn sire 的那个 task）一致；顶级域
// （直接 boot 装载）的 sire() 应返 0。

fn put_hex(v: usize) {
    let mut buf = [b'0'; 18];
    buf[0] = b'0';
    buf[1] = b'x';
    let mut i = 17;
    let mut v = v;
    while i > 1 {
        buf[i] = b"0123456789abcdef"[v & 0xF];
        v >>= 4;
        i -= 1;
    }
    let _ = put(core::str::from_utf8(&buf).expect("ascii"));
}

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("sire_demo: ");
    let _ = put("self=");
    put_hex(self_id().unwrap_or(0));
    let _ = put(" sire=");
    put_hex(sire().map(|t| t.get()).unwrap_or(0));
    let _ = put("\n");
    let _ = put("sire_demo: done\n");
}
