# 本地用户登录示例

启动 product 镜像后，使用测试账户 `anran`、密码 `sqware` 登录。密码不回显，退格和 Ctrl-U 仍可编辑。

认证成功后启动独立 Shell ELF，显示 TaskId 和用户 Principal，进入 `lisp>`。Ctrl-D 取消作业并结束会话，返回登录提示；Ctrl-C 取消前台作业或当前求值，Ctrl-Z 暂停前台作业。密码阶段的 Ctrl-D、Ctrl-C 取消登录；用户名提示处的 Ctrl-D 结束示例 Login。

Login 保存带盐的 Argon2id 密码校验值。Account 服务通过 Identity 创建本次系统运行中的长期用户 Principal，并将 `anran` 关联到 Operator 中真实的 Account 创建条目。每次登录创建新的 Team 和 Task；注销清理实例及身份绑定，保留用户 Principal。重启后身份重新建立。

账户入口 `/svc/account/create` 只接受当前活 Login 的账户名，通过能力授予的 Control 构造接口请求创建，返回暂停 Instance。Login 通过 `/svc/sys/control/instance` 使用现有放行、状态和销毁协议；处理侧核对实例 owner，并拒绝按服务名操作。Control 没有 Session 状态表，领取期限、owner 死亡和系统停止均复用 Instance 回收。状态报告 Dead 时，身份绑定、授权和该实例的资源已清理。

验证：

```sh
cargo test --manifest-path programs/tests/login/Cargo.toml --target x86_64-unknown-linux-gnu
cargo test --manifest-path programs/tests/terminal/Cargo.toml --target x86_64-unknown-linux-gnu
nu scripts/qtest.nu --package kernel --scene product --feed-script programs/tests/terminal/session.py
nu scripts/qtest.nu --package kernel --scene system-fault
```

system-fault 中的隔离探针验证外部调用拒绝、领取超时、放行失败和 Login 死亡清理。

结构与验收记录见 [本轮 System 重构方案](../../../../docs/system-next-refactor-plan.md)。
