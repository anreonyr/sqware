# Session 聚合：Account 与 Terminal Unit 子模块

从 Source `293c15fc97a9d9a4295810d10fe804e4cdec63c4`（Draft PR51）继续。
用户选择两层 Git 子模块：**vanllia 固定 Cluster，Cluster 固定 Unit**。
当前 Source origin 仍为 https://github.com/anreonyr/sqware.git ，既有未合并
PR 栈保留；本页不表示最终 vanllia Integration 已全部交付。

## 固定版本与所有权

| 层 | 仓库 | 完整版本 |
| --- | --- | --- |
| Session Cluster | https://github.com/shuyra-arche/session.git | `f7c7542acf0bf010c1aac07d36d30a19e0591731` |
| Account Unit | https://github.com/shuyra-arche/account.git | `7dc701facdbe6368fbc09f7429920ad49dacdfca` |
| Terminal Unit | https://github.com/shuyra-arche/terminal.git | `f07a0954f65472e9a16e878e52c5f1d34146bc64` |
| Terminal API/client | 同仓，保留既有身份 | `c447c0761835339a39efdf2ddd8101ebf80db15a` |

`components/session` 固定 Cluster，其 `units/account`、`units/terminal`
分别固定 Unit。Unit 在自身仓库拥有 declaration/api/client/server/tests；
同仓允许 path，跨仓使用完整公开 GitHub rev。每个 crate 不单独建仓，
Shell 与六个命令仍归同仓。

Session 工作区只有纯 session-cluster 包，Unit 子模块是独立工作区。
Cluster 显式提供两个 UNITS/IMAGE_BUILDS 与联合验收合同，无 server/client
依赖、全局 Unit 注册表或新 runtime Cluster 类型。Source/vanllia 拥有实际
PROGRAMS/DEPLOYMENTS、选定 Deployment、Login/Shell 适配及 AccountRig。
Login 仍在 Source，未宣称完整 Session 已迁移。

镜像 manifest 逐层增加前缀：Unit 的 server/Cargo.toml，Cluster 的
units/<unit>/server/Cargo.toml，Source 的
components/session/units/<unit>/server/Cargo.toml。Account image/bin 是
account，package 是 account-server；Terminal image/bin 是 terminal，package
是 terminal-server。features/default-features/target/profile 各自保持。
逻辑镜像仍89：86个 Source binary 与 Pipe、Account、Terminal 三个提供方
目标；该数不是 crate/Unit 数或迁移进度的百分比分母。

## 来源与行为合同

Account 来源 Source293c15f，Unit PROVENANCE 公开八个原文件路径、SHA-256、
真实作者与许可证状态。原函数/字段、Request 编解码怪癖、接口标识、
owner/mark 授权、重放/取消、CreateId/水位/分配顺序、64caller/256record、
失败码、生命周期及等待保持。原实现只在公开证明和原始哈希核对后删除。
Terminal 使用已接纳 f07，无运行代码重写；旧 components/terminal 的61个
跟踪文件与新嵌套 Terminal 逐字节一致后，Root 才正常移除旧 gitlink。

先前 Session 单仓6d27af19ee4f94de04bb644ac8fe78e8001e99e7 保留在历史中。
旧边界 Source56项宿主/12个ELF仅为历史，未沿用到新依赖。既有20字节
golden、usize fixture、TeamId 导入修复逐字节带入 Unit。初次 fixture/
导入失败及不支持的 freestanding host-bin raw101 均保留；支持范围是
host libraries 与真实 RV binary，不虚报旧失败为通过，不杜撰许可证。

Kernel 固定3aa0a155f825fbf377fc8d04009472ca67cbbf8c，System runtime 固定
9001af65f97501dfa490c9397c61d593c871136d；System API/client-support 保持
9e5d63ca536a176583cf1b0054b4ecf7ddd88257 /
5a792ff2d39901a970925b1a6d63206adf6c16d8，Pipe4fce及image-builder17f保持。
既有 Face/probe-watch UNKNOWN 断言、1000ms 等待及已记录负向 wiring 覆盖
限制保持；没有为本切片放宽超时或回收判据。

## 验证

真正 DeepSeek 独立 namer/worker/reviewer 组使用
opencode-go/deepseek-v4.1-flash，同 worker SID/cwd 持续返工。官方 patch
仅移除 shell/pwsh/jobs 注册，FS sandbox/观察/权限策略保持。实际 routing
来自持久化 request/context；JSON outcomes/raw/completed/SID/cwd/代码检查点
分别保存。Root 独立选择执行 native/Git，不转交模型任意 shell。

Account 本地及公开全新检出29文件哈希一致，API4/configuration4、四个host
libraries、RV debug/release binary 均通过。实际 Cargo JSON 定位 ELF，验证
RISC-V与唯一启动/panic/分配器，独立保留字节/哈希。Session 本地及公开
递归检出通过 locked 在线/离线 metadata、host library、RV两档纯库；11个
自有文件、两个gitlink及29+61个Unit文件前后干净一致。纯Cluster不宣称新增
Unit测试或运行ELF；缓存复用不说成冷缓存。

新 Source 完成9次正常及9次locked/offline metadata、56项宿主测试、六个
受影响Source binary两档共12个真实RISC-V ELF；锁tuple仅Account/Session归属
变化，无无关升级。首次system-shape raw101仍期待旧单仓路径；原worker只修正
Unit/Cluster各层路径事实并增加两个Cluster前缀断言，重跑7项通过，未删除断言。

六组SMP4/8共12次实际整机全部raw0、独立严格judge0：product terminal-session、
shell-smoke、accept、system-fault、memory-transfer、driver iommu-ruin。均自然
halt、选定entry正常退出，最终帧保持非零前后相等（4核85/85，8核153/153），
交互feed及原场景DMA判定通过。测试使用本次冻结的新输入，没有沿用旧结果。
这里的Hub/Driver仍是Source原实现，新Driver聚合尚待下一切片接入与独立验收。

release首次因构建前低于6GiB而未启动；只在精确可重建缓存核验/清理且旧、新
ELF证据保留后恢复，随后实际release通过。原始退出码、独立judge及缓存操作
分别留档，每次新构建前仍至少6GiB、native共用锁、QEMU串行。最终运行后
仅本页结果记录更新，功能代码保持冻结。不跑1024或旧Kernel全矩阵，不宣称
全部阶段3/4、Session/Driver/Shell或整体迁移完成。
