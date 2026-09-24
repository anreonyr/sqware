# 门那一台 —— `crates/gate`（"完全消除测试脚本"）

> **用户的话**：**"我希望完全消除测试脚本"**。
>
> **裁决**：**甲 · 只消门** —— `scripts/boot.nu`（QEMU 起法的唯一出处）与 `scripts/runner.nu`
> （`cargo run` 那条交互路）**留着**；其余全部收进一个编外的宿主 crate `crates/gate`。
> 变异那一门（`teeth.py`）**收进 Rust**；读数表（`readings.txt`）**搬成 Rust**。
> 每门一条 `#[test]`，门内逐条判据自己汇总。三处松头（看机三份工具、`alloc-probe/run.sh`）
> **一起消**。

## 1 · 一句话：这不是"装一个测试框架"

**判据 + 汇总**那一半本来就不该是脚本——`cargo test` 白给（逐例判据、失败点名、退出码、汇总、
过滤、分层）。**台架**那一半（选场景构建 / 起 QEMU / 喂键 / 期限 / 收日志）任何框架都不做
（`embedded-test` / `defmt-test` / `utest` 三条路都量过，对不上多域 initrd + `DebugCall::Put`），
但它**也不需要脚本**：`std::process::Command` 一样做得成。

故换的是**外壳**：判据一条没改。

## 2 · 四关的落点

### 2.1 功能模型（脚本今天做的 29 件）

| 族 | 件数 | 内容 |
|---|---|---|
| 甲 · 构建 | 4 | 选场景（`SQWARE_ROOT`）· 选档位 · 定位产物 · 环境对齐（`QEMU_ICOUNT=`） |
| 乙 · 起机 | 6 | 凑 QEMU 参数 · **stdin 的所有权**（继承 / FIFO / 管道）· 期限 · 收日志 · 终局观察 · 证据归档 |
| 丙 · 喂键 | 3 | 定时重复喂 · **等标记再喂** · 按参数喂 |
| 丁 · 判据 | 10 | 前缀声明 · 三种匹配（`need`/`needE`/`need_absent`）· 关系 · 配对 · 基线 · 逐行 vs 逐族 · 产物 · 结构 · 停机 · 编译期不变量 |
| 戊 · 变异 | 3 | 按表改源码 · 账本 · 过滤与跳过 |
| 己 · 汇总 | 3 | 计数与打印 · 退出码 · 日志落盘 |
| （补） | 1 | **跑一次宿主命令并收全文**——它原先藏在 `host.sh` 里当裸命令，模型第一版漏了 |

### 2.2 结构（交集 = 只有四个）

```text
   Scenario + Profile ──▶ Image ──▶（boot.nu）──▶ Transcript ──▶ 判据
                            │                        │
                            └────▶ Mutation ──▶ 账
   Readings（表）──────────────────────────────────┘（逐行 / 逐族用同一份）
```

**枢纽一句**：**判据全是"吃 `&Transcript` 的纯函数`**。门之所以曾经是脚本，是因为"读日志 + 判 +
汇总 + 退出码"四件事被 shell 搅在一处；拆开之后**后三件 libtest 全包**，只剩"读日志判"要写。

### 2.3 原语（七个）

| # | 原语 | 吃 | 吐 |
|---|---|---|---|
| 1 | `build` | 场景 + 档位 | `Image` |
| 2 | `run` | `Bench`（机器那台 / 宿主那台） | `Transcript` |
| 3 | `hold` | `&Transcript` + `Mark[]` + `Reading[]` | 每条读数兑没兑现（**一次报全部缺口**） |
| 4 | `values` | `&Transcript` + 形状 | 同一形状的行按序取值（**关系留在门里**） |
| 5 | `pair` | `&Transcript` | `[case]` 的 run/ok 配平并**点名** |
| 6 | `count` | `&Transcript` + 形状 + 基线 | 数目对不对 |
| 7 | `mutate` | `Mutation` | 红 / 等价绿 |

**两处"操作塌成数据"**：三件喂键全是"什么时候往 stdin 写什么"⇒ `Schedule` 一个数据、`run` 一个
原语（"既定时又等标记"这种自相矛盾的状态**不可表达**）；`㉗㉘` 没有原语（libtest 给）。

### 2.4 类型吃掉的三条运行时判据

- **`Tier` 是枚举** ⇒ "narrative 却没写形状"**不可表达**——棘轮从运行时判变成编译期的事；
- **`Mark` 分 `Literal` / `Shape`** ⇒ **BRE 与 ERE 的语义由类型说了算**，不再靠"用哪个 grep"扛着；
- **`Image` 只有 `build` 造得出**（字段私有）⇒ "没构建就起机"不可表达。

## 3 · 目录与跑法

```text
crates/gate/src/lib.rs       台（build / run / Transcript / Bench / Schedule）+ 判据四件
crates/gate/src/soak.rs      默认那一景的读数门：118 条 marks + 读数表 + verdict()
crates/gate/src/mutations.rs 变异那一门：75 条表 + 账 + mutate()
crates/gate/tests/*.rs       一门一条 #[test]（判据住 src/，tests/ 只管起机与报数）

.cargo/config.toml [alias] gate = "test --manifest-path crates/gate/Cargo.toml --target x86_64-unknown-linux-gnu"
```

| 跑法 | 跑什么 | 实测 |
|---|---|---|
| `cargo gate` | **快门**：宿主那一门 | 118 例，**0.24 s** |
| `cargo gate -- --ignored` | 要起 QEMU 的八门 | 见下表 |
| `cargo gate -- --include-ignored` | 全量（含变异那一门） | — |
| `cargo gate <名字> -- --ignored` | 只跑那一门 | — |

| 门 | 判据 | 实测 |
|---|---|---|
| `host` | 八靶逐个对基线 + 总 118 + 无 `FAILED` | 118 例 |
| `examine` | 自退 · 无 panic · `echo: ready` · 回显整行相等 · 停机行 · `router: line=10` | 3/3，39 s |
| `soak` | 停机行 + 118 条读数 + 三条 `policy: me=` 的关系 + `[case]` 配平 + 读数表对账 | round 1 PASS，25 s |
| `framework` | 汇总行 `[case] cases N`（N ≥ 1）· 无 FAIL · 无 panic · 停机行 | 8 例全过，3.1 s |
| `stress` | `rig: total` · 停机行 | `n=328 late=0 lost=0`，3.5 s |
| `load` | `load: spawned rows=` · 停机行 · `timer: late_n=` | `late_n=81 late_max_ms=0 traps=648`，56 s |
| `group` | `group: PASS` · 停机行 | `hung=2 woke=2` |
| `fair` | 停机行 → 客人跑完 → 用例 5/5 → 受害者读数逐字照旧 | **红**（记录在案的缺口，见 `fair-gate.md` §4.3） |
| `console`（`--ignored`，不是门） | 无：起一台、喂一把、把读数交给你 | 2.5 s 自退 |
| `mutations`（`--ignored`） | 75 条：该红的红、该绿的绿 | 默认**真空跑**（账里都验过） |

## 4 · 旧 → 新（十六份脚本的去处）

| 旧 | 新 | 备注 |
|---|---|---|
| `scripts/host.sh` | `crates/gate/tests/host.rs` | 判据四条原样 |
| `scripts/examine.nu` | `crates/gate/tests/examine.rs` | 六条判据原样 |
| `scripts/soak.sh` | `crates/gate/src/soak.rs` + `tests/soak.rs` | 118 条断言 + 读数表**机械抽出来** |
| `scripts/framework.sh` | `crates/gate/tests/framework.rs` | |
| `scripts/stress.sh` | `crates/gate/tests/stress.rs` | |
| `scripts/load.sh` | `crates/gate/tests/load.rs` | `QEMU_SMP=1` 走 `Bench::Machine` 的 `env` |
| `scripts/group.sh` | `crates/gate/tests/group.rs` | |
| `scripts/fair.sh` | `crates/gate/tests/fair.rs` | |
| `scripts/fast.sh` / `quick.sh` / `probe.sh` | `crates/gate/tests/console.rs` | 三份看机工具收成一份 |
| `scripts/readings.awk` | （**不需要了**） | 前缀声明 / 逐行 / 形状棘轮 / 理由四样都在 `Tier` 里 |
| `scripts/readings.txt` | `crates/gate/src/soak.rs` 的 `READINGS` | 29 条搬成 Rust |
| `scripts/teeth.py` | `crates/gate/src/mutations.rs` + `tests/mutations.rs` | 75 条表 + 账，键逐字相同 |
| `crates/alloc-probe/run.sh` | `crates/alloc-probe/.cargo/config.toml` | 十条 `[alias]` |
| `scripts/boot.nu` | **留着** | QEMU 起法的唯一出处 |
| `scripts/runner.nu` | **留着** | `cargo run` 那条交互路 |

## 5 · 五刀的账

| 刀 | 落点 | 提交 |
|---|---|---|
| 一 | `crates/gate` 骨架 + `host` 门；`host.sh` 删 | 0a70778 |
| 二 | 判据四件 + `examine` + `console`；`examine.nu` / `fast.sh` / `quick.sh` / `probe.sh` 删 | bc6dea8 |
| 三 | 读数门；`soak.sh` / `readings.awk` / `readings.txt` 删 | b166194 |
| 四 | 五门 + 台子的旋钮 + "机器只有一台"；`framework.sh` / `stress.sh` / `load.sh` / `group.sh` / `fair.sh` 删 | 9ed0db2 |
| 五 | 变异那一门 + `alloc-probe` 的路；`teeth.py` / `crates/alloc-probe/run.sh` 删 | b16b80f |

## 6 · 照实记（这一刀一路量出来的东西）

- **括号那一族，三次**：① 声明式对账器把 `need` 的**裸**括号按 ERE 判成分组 ⇒ 22 条断言永远不
  命中、"判了"被记成"没人判"；② 给它加形状白名单时 `$0 ~ ""` 匹配一切 ⇒ 判绿；③ 搬 `needE` 时
  我"顺手"把 `\(` 去成 `(` ⇒ 当场红（`policy: derive(me)=[0-9]+` 在 Rust 的 regex 里是个**分组**）。
  **grep 的 ERE 与 Rust 的 regex 在这一点上一致：`\(` 都是字面括号** ⇒ 那一格是恒等变换。
  **"照抄"比"顺手改"安全。**
- **`cargo test` 的汇总行不能按"最近一条 `Running`"归属**：cargo 打出下一台的 `Running` 时，上一台
  的测试进程**还没把最后一块冲出来**（libtest 的 stdio 是块缓冲）⇒ 读数会整体错位一格。八台是
  **依次**跑的 ⇒ 正确归属是**按启动序**（FIFO）。老 `host.sh` 那条 awk 正是"最近一条"那种写法
  ——老那一跑没红过，不等于它没错。
- **控制台是共享的，行不是原子的**：看机那一台第一次跑就当场看到两行被**另一个域**的行插进中间。
  **不是收的时候合的**（`read_line` 只按 `\n` 切）⇒ 是机器的事实。`soak` 看不到它（喂键等
  `echo: seq=0` 之后才动），故那些整行形状从没被这一格撞过。
- **`sleep 12` / `sleep 3` 那些尾巴不用写了**：它们是"别在 guest 收尾之前关掉写端"，现在把写端
  攥到收尾那一步才放（靠作用域），不写魔法秒数。
- **`icount` 统一在 `run` 里关**：旧脚本每一门各自记得写一遍，而**台子与忙机台就漏过一次**
  （两边读数因此不可比）。收进一处，这条纪律不用再记。
- **机器只有一台**：门可以并行跑，台不能。`run` 在机器那一支上拿一把进程内的锁——两门同跑既会
  互相抢核（`timer:` 那几格是时序读数），也会让"读数不可比"换个地方复活。
- **签名长出来的一格**：`Bench::Machine` 多了一个 `env`——忙机台那条债的开关**就是核数**
  （`QEMU_SMP=1` 机理档 / `=4` 对照档），而它不是 `Scenario` 能表达的。这一格不是"再想一个字段"，
  而是把 `boot.nu` 已经有的那套旋钮原样透给它。
- **变异那一刀的签名缺口**：`Bench::Machine` 里装的是**造好的** `Image`，而表要能整行写成常量
  ⇒ 表里存常量版的 `Raw`，`all()` 再把 `Image` 填进去。另加 `rebuild()`：**绕开缓存**重造——改完
  源码跑的还是旧那颗，那就一条变异都逮不住。
- **账一模一样地接上了**：变异那门的键仍是 `名字|文件|sha1(锚点)[:12]`、文件名仍是
  `target/teeth-ledger.json` ⇒ 旧账直接接着用，**75 条键逐字相同**（默认一跑是"跳过 75 条"）。
  这等于把"表搬对了、指纹没换算法"量了出来。
- **旧变异量具的两条限制取消了**：还原从 `git checkout --` 改成"把读到的原文写回去"（`Drop` 里做）
  ⇒ 不再要求工作区干净；判"编译红"从**扫日志**改成看 `rebuild` 返回的 `Err` ⇒ 不用猜。

## 7 · 没做 / 悬而未决

1. **`fair` 还是红的**：修法是调度 / 配额那一族（`docs/fair-gate.md` §4.3），**设计分叉，等裁决**。
2. **内核侧换官方 `custom_test_frameworks`**：镜像程序那一侧量过、不通用（接线只挂 `--test`），但
   **内核 crate 那一侧可以换**——那是另一刀，动的是构建档。
3. **服务台搬进 SUT**：探针那些读数本来就是判据，故搬得干净；服务台打的多是叙事，真搬要先逐条
   回答"这一行判的是什么"（`docs/harness-gate.md` §6/§7）。
4. **三条 sanitizer 路**（`cargo tsan` / `lsan` / `miri`）要一个 `RUSTFLAGS` / `MIRIFLAGS`：那是
   **cargo 自己**读的，别名与 `[env]` 都表达不了 ⇒ 它们是一行命令，不是脚本文件。
