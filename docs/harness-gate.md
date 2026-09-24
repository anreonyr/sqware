# 测具那一门 —— **结构门**（测试与程序分开，第二刀）

> **裁决（用户原话）**：**"为什么 programs 里面的测试和程序混在一起？我不希望这样，需要解决
> 方法"** ⇒ 走**甲2**：新 crate + **按场景分装配单** + 场景开关出正文，**外加声明式读数**；
> 名字叫 **`harness`**（用户定）。
>
> **落地读数**：31 个 bin 分成 **15 产品**（`programs`）+ **16 测具**（`harness`：6 探针 +
> 10 压测台，另加一件共享的 `tick`）；**release initrd 5,875,108 B → 4,184,504 B（−29%）**，
> 而压测那几张从"31 台"降到 **1~3 台**（`rig` 2 / `again` 2 / `group` 2 / `load` 3 / `beat` 1）。
>
> **门**：`examine` **3/3** · `soak 1` **1/1**（含新的读数对账）· `framework` ✓ ·
> `stress`（rig）✓ · `load` ✓ · `group` ✓ · `beat` ✓ · `again` ✓ ·
> `fair 1` **0/1**（**记录在案的那条读数**，见 `docs/fair-gate.md`——不是回归）·
> `host.sh` **118 例** · `cargo check --workspace --all-targets` 0 error（默认与 `fair` 各一遍）。

## 1 · 为什么"混"（三处，量过再判）

| 处 | 事实 |
|---|---|
| **源码** | 31 个程序一个 crate、一张 `[[bin]]` 表：15 产品 + 6 探针 + 10 压测台（+ `tick` 一件只给台子用的共享模块） |
| **镜像** | `kernel/build.rs` 那张清单是**并集**（31 条）——跑默认（验收）镜像也把 10 台压测台装进去，而那 10 台在那张镜像里**永远不会被起**（≈1.6 MB / 5.6 MB）；反过来 `rig` 镜像（只要一个受害者）也装着整套产品与探针 |
| **产品正文** | `supervisor/system/main.rs` 里：装配单列着 5 台试客（`probe-*`），外加**上一刀留下的** `#[cfg(sqware_fair)]`（`FAIR` + 第二张表 + 一句自报）——"某一景多一条聊天客人"写在了产品程序的正文里 |

**一个反直觉的事实**（决定能砍到哪）：**探针必须留在验收镜像里**——`soak` / `examine` /
`fair` 的判据就是它们打出来的那几行。所以"测具全出产品镜像"做不到；能做到的是**压测台出产品
镜像**（它们整台替换引导镜像，与验收没有交集）。

## 2 · 怎么分的（三处各自归位）

```text
programs/            15 份产品（内核起的那套服务 + 真客人）
harness/             16 份测具（6 探针 + 10 压测台）+ lib 里的 tick
  src/lib.rs         这一档是什么、与产品什么关系、为什么搬家
kernel/build.rs      PRODUCTS / PROBES / RIGS 三张表 + bins_for(场景) —— 唯一声明特权级处
system/scenario.rs   装配单（19 行 Program + 两张表 + announce()）—— 产品正文不再有场景分支
```

`harness` 只借产品那一件共享入口：每个测具一声 `extern crate programs;` ⇒ `programs::entry`
的 `_start`（`global_asm!`）与 panic 处理被拉进链接；链接脚本也是同一张（`programs/link.ld`，
镜像都链在 `IMAGE_BASE = 0x10000`）。

**照实记（先验的那一格）**：`extern crate programs;` 能不能把 `_start` 带过 **rlib 边界**，
是这一刀成立的前提——先拿**一份**探针试（`nm` 里 `T _start` 在、ELF 正常），才动的其余的。
验不过的退路是"同 crate 分目录"（乙）。

**照实记（`cargo test` 那两格）**：`harness` 也要 `[lib] test = false / bench = false /
doctest = false`——不然 `check --workspace --all-targets` 会去编它的 libtest，riscv 上
`can't find crate for test` + `#[panic_handler] function required`（与 `programs` 同一条理由）。

## 3 · 声明式读数（甲2 的第二半）

**做过的事**：`soak.sh` 头注那条口径——"凡在装配单里跑的程序，它打出来的读数都要在这里有一条
断言…… 对账的法子是（一轮一次即可）把日志按 `前缀:` 分一分、与本文里的断言逐条对"——是**人手
维护的第二份清单**，而它漏过一次（`probe-deep` 那条例外得专门写一段解释）。现在它是每轮机械
的一步：

```text
scripts/readings.txt   一面表：前缀 <TAB> auto|narrative|manual <TAB> 理由
scripts/readings.awk   对账器（门调它）：读那面表 + **从 soak.sh 抽出来的断言表** + 这一轮日志
soak.sh 每一轮末尾      awk -v asserts=… -v table=scripts/readings.txt -f scripts/readings.awk "$log"
```

三条判据：

1. 日志里出现的**每个前缀都要在表里**（新读数没人管 ⇒ 红）；
2. `auto` 档：这一前缀的**每一行**都要被本门某条断言匹配（新形状没人判 ⇒ 红）；
3. `narrative` / `manual` 档：**理由必填**；`narrative` 至少要有一条被匹配。

（**后来动过**：`narrative` 那一档多了第 4 栏"没判的那几行长什么样"的棘轮，表里的数与档位也在
§7 里重标定过——本节记的是当时那一版。）

**三条牙口（实测）**：往日志里塞一行 `newprog: hello=1` ⇒ 红（没声明）；把
`board: bye tid=17 …` 改成别的形状 ⇒ 红（auto 档要逐行）；**但**在 `narrative` 档里加一行新形状
⇒ **绿**（这是已知的粗粒度：那一档只挡前缀层面的漂移，形状层面留给下一刀——十来个前缀逐个写
形状是填表活，不是设计活）。

**表的现状（量出来的，不是想出来的）**：27 个前缀——**17 auto** · 9 narrative · 2 manual
（`task:` 的判据是 soak 里那段 `if grep -q`；`probe-deep:` 只在公平台起，判据在 `fair.sh`）。
第一跑就照出一个洞：**`board:` 那 9 行一条判据都没有**——已补一条交替形状断言（`bye …` 每轮
都有、`swept …` 只在真扫过时才打，故一条形状既覆盖全部行、又不因为"这一轮没扫"而假红）。
各档"还没判的那几行"写在表里的理由那一格（`member` 2/27、`system` 5/23、`router` 8/18 …），
**让它们看得见，别悄悄长**。

## 4 · 照实记（两个当场逮住自己的坑）

- **awk 里未初始化的变量当下标是空串，不是 `"0"`**：对账器第一版没写 `np = 0` ⇒ `apat[0]` 从没
  被赋值 ⇒ 取回空串 ⇒ `$0 ~ ""` **匹配一切** ⇒ 整台对账器"全绿"（连"这条读数没人判"的样例都
  判绿）。是拿一条**确定没人判**的真实行（`member: me=13`）去顶它才露的馅。
- **`\r` 那一格**：板那几行**带 `\r`**（`od -c`：`swept=0\r\n`）。新补的那条断言写成
  `…)$` ⇒ 对账器判它"9 行没判"（形状写严了也是错的）。加一格 `[[:space:]]*` 才是今天那几行的
  真形状——与本仓 `timer:` / `doom:` 那几条同款。

## 5 · 没做与下一刀

- **narrative 档的形状级 ratchet**：把那 9 个前缀"承诺打的形状"逐个写进表里 ⇒ 那一档也能挡
  形状漂移。填表活，估 50 行左右。→ ✅ **已做**（§7.3）；**而且那几个前缀的数当时是错的**，
  见 §7.1。
- **把 soak 的逐行判据补齐**：表的理由是"这些行还没判"，补齐判据之后它们就能升到 `auto`
  （`member` / `system` / `policy` 那三族最大，且都是**能写形状的**）。
- **内核侧换 `custom_test_frameworks`**：`kernel/src/framework/` 是手抄的
  `os-test-framework` 形态；工具链是 nightly，官方那条路（`#![feature(custom_test_frameworks)]`
  + `#[test_case]` + `harness = false`）能用，换过去少一层自造的链接期发现层。**另一刀**，
  动的是门的构建档。
- **镜像程序那一侧没有合用的现成框架**（这一问量过）：`embedded-test` 要 probe-rs + semihosting
  + 单镜像 + 每例复位；这里是多域、initrd、内核装载、控制台走 `DebugCall::Put`。`defmt-test`
  更窄，`utest` 只认 cortex-m。故 `harness` 不引框架——**但"自定义 test"这条自造路走通了**，
  见下一节。

## 6 · 程序侧 pilot（用户裁定）：一次跑通的读数与代价

> **裁决（用户原话）**："能不能用测试框架把脚本替换掉？" → 量过之后：**能替一半**（判据 +
> 汇总那一半；**台架**那一半——按场景构建 / 起 QEMU / 等标记喂键 / 超时 / 收日志——不是框架的
> 活）。再问"**自定义 test？**" ⇒ 先拿**一份探针**（`probe-rule`）做 pilot。

**形状（与内核那台不同，这是 pilot 量出来的结论）**：**运行时登记**——不碰链接脚本、不用
`cargo test`、构建管线一个字不动。

```rust
let mut suite = cases::Suite::new();
suite.case("in_covers_that_league", move || assert_eq!(inside, ocall::OK));
…
suite.run();     // 全过才返回；失败走 panic 通道（域当场死）
```

两条路都省了，各有实测：

| 那条路 | 为什么不用 |
|---|---|
| **官方那台**（`custom_test_frameworks` + `#[test_case]` + 自定运行器） | 它的接线**只挂 `--test` 那一档**：同一个 `no_std` 文件编两遍，普通构建报 `warning: function 'run' is never used`、`--test` 那一遍不报（即运行器接上了）⇒ 镜像程序是**嵌套 `cargo build` 出来的 bin**，永远不在那一档 |
| **链接期段收集**（内核那台的做法） | 要改 `programs/link.ld`（加 `.cases` + 两个边界符号），而读数还在局部变量里——`fn()` 捕不了环境，就得再把读数搬进全局量。程序侧的读数**本来就在同一个 `main` 的同一段**里，就地登记一例只多一行 |

汇报格式（门只认这几行，`harness/src/cases.rs` 是出处）：

```text
[case] <台名>: N cases
[case] <台名>: run <名>      ← 开跑前打（内核那头"先打 ok 再跑"撒过谎）
[case] <台名>: ok <名>       ← 只有跑完才打
[case] <台名>: cases N ok M fail K   ← 全过才有这一行
```

**照实记（`<台名>` 那一格是补出来的）**：第一版不带台名，于是 `probe-owner` 与
`probe-rule-other` 的汇总行**逐字相同**（都恰好 3 例）——门的基线断言因此分不出是哪一台
（一台没登记、另一台顶上，断言照绿）。台名一进协议行，基线就是**逐台唯一**的了。

**收益（实测）**：把 `in_covers_that_league` 的期望值**故意改错**，一台 soak 报的是
`[case] run/ok 不配对（run=4 ok=3）——失败的那一例是：[case] run in_covers_that_league`，机器
那侧跟着是 ``assertion `left == right` failed  left: 0  right: 8 at harness/src/probe_rule.rs:372:49``
——**用例名 + 两个值 + 文件:行:列**。对照：同一处断了，以前只报 `probe-rule: a rule did NOT
hold`，得回头看那 19 个计数器才知道是哪一条。

**代价（实测，`git diff --stat`）**：约 **220 行**——新 `cases.rs` 90 行（一半是照实记）+ 那台
探针的判据改写 ~70 行（13 条 `&&` → 16 例，一例一行）+ 门那三条与对账器的 `[case]` 支持 ~40 行
+ 顺带修的一格（见下）。其中可复用的（`cases.rs` / 门那三条 / 对账器）是**一次性**的；**再搬
一台探针约 30–60 行**。
**一次只报一个失败**（`panic = abort`，没有 unwind）——这是把判据搬进 SUT 的代价，取舍写在 §5。

### 照实记（pilot 当场逮住的两格）

- **`kernel/build.rs` 的"盯哪些文件"漏了 `harness`**：搬家那一刀只走 `programs/` 与 `crates/`，
  于是**改测具不重打包**——pilot 第一跑就撞上"日志里一行 `[case]` 都没有，而 initrd 里明明有
  那个串"（`grep -c '\[case\]' initrd.img` = 4、探针产物也是新的）。修法：
  `for tree in ["programs", "harness", "crates"]`。**这一格是搬家漏的，不是 pilot 引入的。**
- **`[case]` 在 `grep` 里是字符类**：那两条断言第一版写成 `need "[case] 16 cases"` ⇒ `[case]` 被
  当成"`c`/`a`/`s`/`e` 里任一个" ⇒ **永远不匹配**，门报"缺这两条"而日志里那 32 行都在。改走
  `needE "^\[case\] …[[:space:]]*$"`。

### 第二刀：其余五台探针照搬（用户裁定"行"）

| 台 | 例 | 名字（就是结论） |
|---|---|---|
| `probe-rule`（第一刀） | 16 | `three_rules_landed` / `opens_refuses_someone_elses_door` / … |
| `probe-denied` | 2 | `the_landing_is_denied` / `the_cell_is_still_free_after_the_refusal` |
| `probe-owner` | 3 | `a_living_owners_plate_refuses_me` / `that_cell_did_not_move` / `a_dead_owners_name_can_be_taken_over` |
| `probe-lease` | 1 | `the_plate_landed` |
| `probe-rule-other` | 3 | `a_foreign_identity_cannot_use_the_is_cell` / `nor_the_under_cell` / `nor_a_cell_pointing_at_someone_elses_door` |
| `probe-deep`（只在公平台） | 5 | `the_chain_reaches_the_cap` / … / `the_chain_is_trimmed_clean` |

**口径：只搬每一台**已经在判**的东西**——不新造判据。这一条不是洁癖：账里那两条**等价变异**
（"租赁那一趟不声明归自己" / "接手那一趟反而声明归自己"）正是"机器读数看不见、宿主靶管着"的
那两格；若借搬家的机会替探针加一条它原先没有的断言，等效就变红，账会被我改成假的。

**门那一侧跟着缩**：探针那一族原先是一串**钉死值**的形状（`probe-owner: tree land=8 before=… after=id=`、
`probe-rule: tree part=… 19 个计数器…`），现在只剩三样——① 读数那一行**还在**（只查前缀）、
② 每台**登记了几例 / 跑完几例**（基线，逐台唯一）、③ 走通那一句（`exit … note:` ⇒ 正常退场
而不是 panic）。**值归用例**。

### 下一刀（若要把这条路走完）

1. **服务那一侧要分开谈**：探针的每一行读数**本来就是判据**，所以搬得干净；而
   `echo` / `guest` / `sleeper` / `subject` / `member` / `router` / `uart` / `rtc` 打的多是
   **叙事**（"我拿到了哪一号"），它们的契约要么已经由门/probe 证了，要么根本没被断言过。
   硬搬只会把叙事当判据（假判据），或者把 `bail`（起手没走通）改个名字——两样都不赚。
   真要做，得先逐条回答"这一行**判的是什么**"，那是另一刀。
2. 系统级那一半（`devices: 21 handed to root`、停机行、内核 `timer:` / `doom:` / `sched:` /
   `irq:`、时序）**留在宿主的门**——SUT 里的用例看不到它们。
3. 到那时再看 `scripts/*.sh`：正文应该只剩"起机 · 喂键 · 读汇总"，也就是**一行壳**。

## 7 · 对账器修错 + 叙事那一档上棘轮（用户裁定"下一步"）

**这一刀起手不是设计，是量出来的一处错账。** 要收"服务那一侧"的账，先得知道那笔账到底是多少
——于是把 `scripts/readings.awk` 的 `report=1` 又跑了一遍，顺手逐条核了几行"没人判"的读数，
结果核出**量具自己错了**。

### 7.1 错在哪（一句话）

`soak.sh` 的 `need()` 走 `grep -q`（**BRE**：`(` `)` `{` `}` `+` `?` `|` 都是**字面**），
`needE()` 走 `grep -qE`（**ERE**：那几个是元字符）。对账器把两者**都**按 awk 的 ERE 判 ⇒ **22 条
带括号的 `need` 断言在这里永远不命中**：ERE 里 `policy: adopt(sub)=ok` 要的是 `policy: adoptsub=ok`。

实测（同一份日志、同一份断言表）：

```text
  grep -q "policy: adopt(sub)=ok"  …  ⇒ 命中   （门那一侧判对了，所以门一直是绿的）
  awk   $0 ~ /policy: adopt(sub)=ok/ ⇒ 不命中 （对账器把它记成"没人判"）
```

于是那张表上的数是假的：

| 前缀 | 表里原来写的 | 真相 |
|---|---|---|
| `member:` | 判 2/27 | **19/27** |
| `policy:` | 判 2/15 | **12/15** |
| `router:` / `uart:` / `lodger:` | 8/18 · 8/10 · 3/5 | 8/17 · 7/9 · 3/5 |

**这不是门的错，是量具的错**：门那一侧（grep）一直判对了；错的是一台**用来定档位的**仪器。故第
一件事是修它——`need` / `need_absent` 那一族先按 BRE 把 ERE 元字符转义回本义（`bre2ere()`），
`needE` 原样。

**照实记（同一个坑第二次）**：§4 那条"awk 未初始化变量当下标是空串、`$0 ~ ""` 匹配一切"的坑，
本轮在新加的 `nore[]`（第 4 栏那个白名单）上**又露了一次**——没写白名单的那一档取回空串 ⇒ 判绿。
故主循环里必须先判空串再判正则（`nore[p] == "" || $0 !~ nore[p]`）。

**照实记（行尾那个 `\r`）**：§4 那一条当时的修法是"给每条断言的尾巴挂 `[[:space:]]*`"；本轮改成
**对账器统一去掉行尾 CR**——第 4 栏那十串形状才写得下 `$`，不然每条都得挂个尾巴。

### 7.2 修对之后露出来的洞（逐条读过源里头注才补的）

重新标定，`member:` 还剩 8 行没人判。逐条对着 `programs/src/user/member.rs` 头注那 16 步脚本读：

```text
  member: adopt(sub)=ok              步 8 的结论
  member: waive=ok                   步 12 的结论
  member: enter(out)=err:unknown     步 13 的第三态负证
  member: leave(out)=err:unknown     同上
  member: enter(c1)=ok               同一条判据的第二个号
  member: amid(me,c1)=true           同上
  member: me= / derive(me)=          叙事（每轮的号）
```

前六条**是这台机器当场就判了的**——口径与探针那一刀一模一样：**只搬它已经在判的东西**，不新造判据
（造了就等于把基线改成假的）。同理从 `router:` / `uart:` / `lodger:` 的头注里挑出**同一族里漏掉的
那一个实例**五条：`router: line 1 = virtio_mmio@10001000`（兄弟是 `line 10 = serial` / `line 11 = rtc`）、
`router: line=10`、`router: exhaust line=10`、`uart: line occupied`（兄弟 `rtc: line occupied`）、
`lodger: occupy=0`（三趟登记里**成功**那一格，兄弟 `taken=2` / `unknown=1`）。**一共 11 条**进
`soak.sh`。

剩下那些逐条读过，**确实是叙事**：`got N` / `reg=` / `op=` / `me=` / `root: done` /
`rtc: time A -> B` / `system: gone …`——值每轮都不一样，而能判的那部分（族在不在、形状对不对）由
别的机制管着。故它们留在 `narrative` 档，但**形状写进表里**。

### 7.3 棘轮（§5 那一刀，本轮落地）

表加第 4 栏：`narrative` 档**必须**声明"没判的那几行长什么样"（一串 ERE，`|` 分隔的形状）。
本轮没判的行，只要有一行不在那串形状里 ⇒ 红。

```text
  member  narrative  判 25/27：…  ^(member: derive\(me\)=[0-9]+|member: me=[0-9]+)$
  system  narrative  判 5/23：…   ^(system: gone [a-z0-9-]+ state=[A-Za-z]+ ousted=(true|false) heir=[^ ]+ wait=[a-z]+)$
  [case]  narrative  判 10/60：…  ^\[case\] [a-z0-9-]+: (run|ok) [_a-z0-9]+$
```

**牙口（实测：真日志 + 手塞一行）**：

| 塞什么 | 结果 |
|---|---|
| 原样 | 绿（退出码 0） |
| `member: brand_new=1`（叙事档的新形状） | **红**：`有 1 行既没人判、又不在声明的形状里（棘轮）` |
| `sleeper: brand_new=1`（auto 档） | 红：`有 1 行没被判（auto 档要逐行）` |
| 把 `policy` 那第 4 栏删掉 | 红：`是 narrative，但表里没声明「没判的那几行长什么样」` |
| `brandnew: x=1`（新前缀） | 红：`没在 scripts/readings.txt 里声明` |

**照实记（为什么白名单钉形状、不钉行数）**：这一刀写完连跑几轮真 soak，同一个前缀的行数会动
（`router:` 实测 17↔18、`uart:` 9↔10——**同形状、多一行或少一行**）：钉数会假红，钉形状不会。

### 7.4 表的现状（量出来的）

**29 条：17 auto · 10 narrative · 2 manual**（`[case]` 从"末行汇总"改成叙事那一档：汇总行由逐台
基线判，逐例的 `run` / `ok` 由那条点名检查判——它的判据在断言表里，只是不在"逐行"这一层）。
**没人判的行：表里那些数加起来是 125 行；修对量具之后是 98 行；补上 11 条断言之后剩 83 行**——
其中 50 行是 `[case]` 的协议行、18 行是每个域一行的 `system: gone`、1 行是 `task:`（`manual`），
**真正"值没人判"的只剩 14 行**，全是号 / 序号 / 时刻这类顺路读数：形状已钉，值本来就不该钉。

### 7.5 这一刀**没**做的

- **服务那一侧搬进 SUT**：本轮只把**账**修对（谁判什么、判到没有）。§6 那条边界照旧——服务台打的
  多是叙事，真搬要先逐条回答"这一行判的是什么"；本轮第一次把这份清单量准了（14 行顺路读数 +
  18 行族读数），搬不搬是下一刀的裁决。
- **`member:` 那六条为什么走宿主断言、不写成用例**：它们是**一条读数一行**（不是一族），宿主
  `need` 一行就够；用例那一半的价值在"一族多例 + 名字即结论"，吃到它的是探针那种 19 个计数器的台。
