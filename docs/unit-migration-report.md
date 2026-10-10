# Unit 域迁移记录

状态：实现与验证完成；迁移已接到 PR #5 的真实 head，作为后续提交推送。

实施日期：2026-10-10。工作树：`/workspace/sqware-unit-impl`，分支 `unit-domain-redesign`。设计依据为 [Unit 域统一设计](unit-domain-redesign.md)。主智能体已接手原 Luna 迁移工作。

## 基线与范围

初期实施树在 base `4d664c99c7b068b3e005ffa1f54e4cb82d64c0ce` 上还原 PR diff，使用 `/workspace/pr5-review` 作对照。推送前已获取并核验真实 PR head `35b3d73e13f695c935665efa6d79b12265af98e7`，在 `/workspace/sqware-unit-push` 创建后续提交，迁移补丁通过 `git apply --check --whitespace=error` 后应用。重建快照和真实 head 存在 Schedule 链式接口、错误映射及少量调试条件差异；推送树保留真实 head 的这些实现，不带入无关 base 改动。

提交目标为原 PR 分支 `codex/lisp-shell`，保留原提交历史。下表整机验证在初期实施树完成；真实 head 上应用迁移后另行通过 kernel/programs 全目标编译、gate 30 项、Control instance 10 项和 execution 回收 harness 1 项测试。真实 head 上未重跑整机场景。

## 落地的契约

| 范围 | 实现 |
| --- | --- |
| 状态 | TaskState 唯一保存任务状态、运行 hart 和退出 cause/reason；TeamState 保存构造、暂停和终止阶段。删除独立 tag、completion、boarding、gate.closed 等状态镜像。 |
| 成员与结果 | Team.tasks 保存共享成员记录，引用唯一 TaskState；退出记录不强持有 Task、Team 或 Space。待领取 owner 集表示接收进度，不复制退出事实。 |
| 多层管理 | 同一 owner 按 TaskId 去重，记录其授权 root 来源；Oust 精确撤销该根来源。多个祖先仍可独立领取同一退出事实。稳定父子元数据支持执行资源先回收、祖先后领取。 |
| Join | Hear 并入 Join，Task/Team 目标共用 Wait 和 receive 位。观察不消费，领取删除当前 owner 的待领取项。没有单独放弃结果调用或 Oust(Task)。 |
| Scan | 按稳定 TeamId 游标分页枚举直接 owned Team，单页 1–64 项；删除公共 Heir/HeirCount。 |
| 暂停与终止 | Team Debark 等待各运行任务离槽；Embark 不放行单独 Held/暂停的任务。Task Slay 终止该 Task 及其 heir，Team Slay 终止整个子树，后者保留 Doom 权限要求。 |
| Oust | 仅直接 owner 可 Oust Team；要求整个子树 Reaped 且没有创建发布并发。Busy 不提前关闭 Team；Oust 不要求 Doom 权限。 |
| ABI | Unit class 1 使用显式 slots 32–42；旧 slots 拒绝。Join 以三个返回寄存器交付 cause、TaskId、reason。内核、env、运行时及程序镜像同步迁移。 |
| Rust Join<T> | T 在共享用户空间传递，与 native TaskExit 分开。修正闭包 trampoline 的 FnOnce 类型及启动 Box 所有权；提前 Drop 的槽由运行时登记并在 native 完成后清理。 |
| Control | 分别接收成员退出，由 Control 判断主任务/辅助任务策略。取消、停止和已知 Team 的失败回滚终止整个 Team，防止同 Team 辅助线程阻塞 Oust。主任务非零 reason 优先，主任务零 reason 不盖掉辅助失败；分轮领取耗尽 64 项预算时暂缓 Oust，避免丢掉后面的失败。 |

成员和直接子 Team 各限制为 1024，域深度限制为 32。发布前准备容量；额度或分配失败返回错误，不覆盖现有记录。

## 同步与回收

Unit 结构提交锁串行化发布、暂停、终止、领取和 Oust；共享状态另有同步保护。能力操作先按 TaskId 排序锁 gate，再获取 Unit 锁。等待容器移除与任务脱链遵守同一提交边界，避免终止与唤醒争夺任务链。

退出先执行能力/等待清理 hooks，再发布 Reaped，随后通知 Task 与祖先 Team 等待键。回收躯壳使用独立侵入链，不把工作链接保留在 Reaped 中。

整机验证发现 Starve 和等待分支曾持 Unit 锁进入 idle fetch；已改为先释放锁，并增加进入 fetch 时的 debug 检查。Build 的 heir 容量在进入提交锁前预留，提交时复核容量。另发现 Control 取消实例只 Doom 主 Task，暂停的同 Team 辅助 Task 仍存活；已将已知 Team 的实例/服务终止改为 Team Slay。

## 验证

工具链为 `nightly-2026-10-09`，目标 `riscv64gc-unknown-none-elf`。主构建目录 `/tmp/unit-migration-review-target`，host 测试使用独立目录。日志已归档至 `/workspace/.sqware-tools/unit-migration-verification/`；临时原件位于 `/tmp/unit-*.log`。

| 检查 | 结果 |
| --- | --- |
| `cargo check -p kernel -p programs --all-targets` | 通过（包含最后的 Control Team 终止改动）。 |
| `cargo check -p execution --lib` | 通过。 |
| env host ABI 测试 | 5 项通过，覆盖新 Join 编码及旧 slots/非法参数拒绝。 |
| kernel gate host 测试 | 30 项通过。 |
| execution 独立 host 测试 | 原有 2 项通过；生产 task.rs 回收 harness 1 项通过，覆盖 Spawn/Embark 失败、终止记录已 prune 的失败清理、提前 Drop、正常 Join 和异常完成。 |
| Control host 测试 | instance 10、lifetime 9、query 2、rpc 5、api 6 项通过；instance 检查 Ruin 实际终止 Team、主任务零退出码不掩盖辅助失败、130 条结果分轮领取完才 Oust。 |
| Shell host 测试 | 13 项通过。 |
| 系统结构边界测试 | 18 项通过。 |
| 无 initrd 的内核健康面 | 37 项通过；唯一 scene sentinel 按仓库约定失败，不计为产品 scene 通过。 |
| 整机 rig / accept | 迁移后的前期运行各 1 项通过。 |
| 最终 system-fault | 1 项通过（21.74 秒）；三个 System 角色各自的任务及后代回收断言通过。 |
| Shell edges | 最终代码 1 项通过（32.34 秒），交互驱动 `ok`；含错误退出、失败回滚、长输出和 broken pipe。 |
| Shell smoke | 完整交互脚本通过，QEMU 1 项通过（262.14 秒），驱动结果 `ok`。保留原脚本的 131072 次八字节写入、1 MiB 大小、暂停/恢复/取消、前后台读、Ctrl-C/Z 及退出断言；仅临时副本的等待时限改为 600 秒，runner 为 660 秒。 |

本轮沙箱禁止本机 socket，最终整机回归通过临时 QEMU 命名管道串口运行同一交互脚本。原有产品和 Shell 测试脚本未因吞吐问题改写；临时合并测试写入的尝试已撤回。

默认 60 秒与部分延长试跑超时；原 PR 快照的单独 1 MiB 传输也在 60 秒超时。这说明默认时限不足不是迁移独有，不能据此宣称两版性能相同。一次关闭指令计时的诊断确认原小写入完成并得到 1048576 字节；最终通过的是默认板子参数及上述延长时限。部分试跑在启动阶段返回 NotReady；即使内核 scene 报绿，只要交互驱动未完成，仍记作未通过。

完整 smoke 后补齐的两条异常/大结果量分支，分别由包含生产 task.rs 的回收 harness 和包含生产 hook.rs 的 130 条退出记录测试验证；随后编译和 system-fault 整机复查使用最终代码。

## 审阅与复现

迁移相对只读 PR 快照的 [独立 diff](../../.sqware-tools/unit-migration-verification/migration.patch) 与 [文件清单](../../.sqware-tools/unit-migration-verification/migration-files.txt) 已导出；diff 在隔离的原 PR 副本上通过 `git apply --check --whitespace=error`，不包含原 PR 本身的新增功能差异。

归档保留 [命名管道 qtest 转发器](../../.sqware-tools/unit-migration-verification/qtest-pipe.nu)、[仅延长等待时限的 smoke 副本](../../.sqware-tools/unit-migration-verification/smoke-long.py)、交互捕获及各项日志。工作树内临时诊断脚本已移除。复现时从实施树执行：

```sh
source /workspace/.sqware-tools/env.sh
export CARGO_TARGET_DIR=/tmp/unit-migration-review-target
export TRACE_OUT=/tmp/unit-repeat-trace
export CARGO_NET_OFFLINE=true
nu /workspace/.sqware-tools/unit-migration-verification/qtest-pipe.nu \
  --package kernel --scene product \
  --feed-script /workspace/.sqware-tools/unit-migration-verification/smoke-long.py
```

转发器沿用原脚本的板子参数和镜像构建流程；唯一区别是本地串口 transport 和上述 runner 时限。它不进入产品代码或提交的测试脚本。

## 实际边界

Reaped 事实发布和预留的成员/回收链接不分配；现有能力撤销 hooks、消息唤醒信标和等待续约仍可能准备元数据，其中部分在 Unit 锁内执行；不能将整个退出/唤醒链描述为完全不分配。

Rust 提前 Drop 的槽在后续 spawn、Join、Drop 或 trampoline 活动时清理；没有后续运行时活动时可能保留至 Team 空间回收。native fault/Slay 不提供 Rust 栈展开，不能保证任意中止闭包及 TLS 捕获对象的析构。Miri 未安装，未声称通过 Miri。

私有运行时完成判断仅针对本运行时在当前 Team 创建的任务：其观察权限不可撤销，因此记录已 prune 后的 Denied 可用于释放本地槽；不将其解释为成功的 T 或退出 reason。Control 的通用 Denied 不合成成功退出结果。
