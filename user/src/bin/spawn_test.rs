#![no_std]
#![no_main]

extern crate alloc;

use ubi::Spawnee;
use user::env::io::put;
use user::env::task::{spawn_task, spawn_team};

// spawn_test: 运行期装载镜像成独立域 + 域内产线程验证。
//
//   spawn_team(Spawnee::Back) → 建独立域（TeamId），子域记 sire（本 task）+ heir 予父域.
//   spawn_task(team, 0, 0)    → 域内产线程（entry=0 用域默认 = back `_start`）。
//   Back 是快速跑完退出的 demo（非交互），验证 spawn 链路 + 域默认 entry 让新 task 跑起来。
//
// 血缘（sire/heir 数据）不在此输出（收敛到内核机制）；此处验 run-time spawn 链路。

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("spawn_test\n");

    match spawn_team(Spawnee::Back) {
        Ok(tid) => {
            let _ = put("T\n"); // spawn_team 成功（TeamId 非 0）
            match spawn_task(tid, 0, 0) {
                Ok(task_id) => {
                    let _ = put("K\n"); // spawn_task 成功
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
