# 本地用户登录示例

启动 product 镜像后，使用测试账户 `anran`、密码 `sqware` 登录。密码不回显，退格和 Ctrl-U 仍可编辑。

认证成功后启动独立 cat ELF，显示 TaskId 和用户 Principal，随后回送输入行。Ctrl-D 或 Ctrl-C 结束此次会话并返回登录提示；`exit` 是普通输入。密码阶段的 Ctrl-D、Ctrl-C 取消登录；用户名提示处的 Ctrl-D 结束示例 Login。

Login 保存带盐的 Argon2id 密码校验值。Control 创建本次系统运行中的长期用户 Principal，以 `/idt/principal/anran/ref` 发布索引。每次登录创建新的 Team、Task 和 `/uit/<team>/<task>` 运行目录；注销清理实例及身份绑定，保留用户 Principal。重启后身份重新建立。

授权创建入口 `/svc/sys/control/account` 只接受当前活 Login 的账户名，返回已有的暂停 Instance。Login 通过 `/svc/sys/control/instance` 使用现有放行、状态和销毁协议；处理侧核对实例 owner，并拒绝按服务名操作。Control 没有 Session 状态表，领取期限、owner 死亡和系统停止均复用 Instance 回收。状态报告 Dead 时，身份绑定和 `/uit` 已清理。

验证：

```sh
cargo test --manifest-path programs/tests/login/Cargo.toml --target x86_64-unknown-linux-gnu
cargo test --manifest-path programs/tests/terminal/Cargo.toml --target x86_64-unknown-linux-gnu
nu scripts/qtest.nu --package kernel --scene accept --feed-script programs/tests/terminal/session.py
nu scripts/qtest.nu --package kernel --scene system-fault
```

system-fault 中的隔离探针验证外部调用拒绝、领取超时、放行失败和 Login 死亡清理。

System 装配位于 `system::app`。账户与启动程序选择由装配策略提供，Loader 只装载镜像并构造暂停任务，Control 独占实例登记与状态推进。身份安装和 `/uit` 登记作为 Prepare hook 接入；发布撤销、目录移除、身份解绑和域回收作为 Retire hook 接入。每个实例保存独立 schedule 游标，Pending 不阻塞其他实例；准备失败转入回收，清理失败保留 Stopping 并重试，完成后才报告 Dead。
