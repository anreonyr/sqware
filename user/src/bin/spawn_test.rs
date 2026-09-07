#![no_std]
#![no_main]

extern crate alloc;

use core::time::Duration;

use ubi::Spawnee;
use user::env::io::put;
use user::env::room::sleep;
use user::env::task::{heir_at, heir_count, spawn_task, spawn_team};

// spawn_test: 运行期装载镜像成独立域 + 域内产线程验证（spawn Sire 镜像验溯源）。
//
//   spawn_team(Spawnee::Sire) → 建独立域（TeamId），子域记 sire（本 task）+ heir 予父域。
//   spawn_task(team, 0, 0)    → 域内产线程（entry=0 用域默认 = sire_demo `_start`）。
//   heir_count/heir_at        → 父枚举自己的 heir（应见 1 个子域，TeamId = tid）。
//   sleep(1s)                 → 给子线程跑完的时间窗口（父退出即 doom 级联，须在退出
//                               前让子线程执行完——否则子域还没跑就被 cull）。
//
// 验证两条血缘边：sire_demo 输出 `sire=…`（子→父溯源）；本 task 输出 heir 枚举
// （父→子清单）。两者同源同值。

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("spawn_test\n");

    match spawn_team(Spawnee::Sire) {
        Ok(tid) => {
            let _ = put("T\n"); // spawn_team 成功（TeamId 非 0）
            // 父枚举自己的 heir：应见 1 个子域，TeamId = tid。
            let n = heir_count().unwrap_or(0);
            let _ = put("heir_count=");
            put_dec(n);
            let _ = put("\n");
            for i in 0..n {
                let _ = put("heir[");
                put_dec(i);
                let _ = put("]=");
                put_dec(heir_at(i).map(|t| t.get()).unwrap_or(0));
                let _ = put("\n");
            }
            match spawn_task(tid, 0, 0) {
                Ok(task_id) => {
                    let _ = put("K\n"); // spawn_task 成功
                    // 给子线程时间窗口：跑 sire_demo 输出（父退出会 doom 级联子域）。
                    let _ = sleep(Duration::from_millis(1000));
                    let _ = put("spawn_test: done\n");
                    let _ = task_id;
                }
                Err(e) => {
                    let _ = put("F1\n"); // spawn_task 失败
                    let _ = put("spawn_test: done\n");
                    let _ = e;
                }
            }
        }
        Err(e) => {
            let _ = put("F0\n"); // spawn_team 失败
            let _ = put("spawn_test: done\n");
            let _ = e;
        }
    }
}

/// 无分配打印十进制 usize（panic 现场禁忌 format!）。
fn put_dec(v: usize) {
    let mut buf = [b'0'; 20];
    let mut i = 20;
    let mut v = v;
    if v == 0 {
        i -= 1;
        buf[i] = b'0';
    } else {
        while v > 0 && i > 0 {
            i -= 1;
            buf[i] = b'0' + (v % 10) as u8;
            v /= 10;
        }
    }
    let _ = put(core::str::from_utf8(&buf[i..]).expect("ascii"));
}
