use std::env as std_env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use env::ProgramKind;
use env::wire::manifest;

/// initrd 承载的引导期程序（**临时机制**，见 `kernel/src/platform/initrd.rs`）：
/// (清单名, cargo bin 名, 特权级)。顺序无关——**清单解释权在 root 域程序**，
/// 内核只按 `SQWARE_ROOT`（本脚本运行时读）选出的 `ROOT_OFFSET`/`ROOT_LEN` 取引导镜像。
/// 这里是**唯一**声明「程序装成哪种空间」的地方——root 从清单里读，不再硬编码。
/// 码（`ProgramKind` → u32）在 `env::wire::manifest` 里写死一次，本表只用类型。
/// 引导镜像的**清单名**：默认 `root`；压测台用 `SQWARE_ROOT=rig` 换一个（见
/// `harness/src/rig.rs`）——换的只是"谁的镜像被当引导镜像"，内核其余一字不改。
///
/// **在 build 脚本里按运行时读**（不是 `option_env!`）：`option_env!` 会把值烘进这台
/// 脚本自己的二进制，而脚本何时重编不由那个变量决定（实测换变量后引导镜像没换）；
/// 运行时读 + `rerun-if-env-changed` 才是 build 脚本该用的那一对。
fn root_name() -> String {
    std::env::var("SQWARE_ROOT").unwrap_or_else(|_| "root".to_string())
}

/// 引导镜像在清单里的**名字**：`fair` 那一台用的是**同一份镜像**（`root`）——`SQWARE_ROOT=fair`
/// 换的是**编排域那张装配单**（多一条"聊天客人"的行），不是镜像本身。
///
/// **照实记（为什么要有这一格）**：一开始给 `fair` 另加了一条清单别名指向同一个 `prog-root`
/// ⇒ initrd 里**同一份镜像装了两遍**、当场变大 ⇒ QEMU 报 `Not enough memory to place DTB
/// after kernel/initrd`（`scripts/fair.sh` 第一次跑就是这个）。故这里归一名字，镜像仍只有一份。
fn image_name() -> String {
    let name = root_name();
    if name == "fair" {
        "root".to_string()
    } else {
        name
    }
}
/// 产品那一档（**15 份**）：内核起的那套服务 + 真客人。三张表合起来就是镜像里可能有的全部
/// 程序；**哪几台进哪一张镜像**由 [`bins_for`] 按场景选。
///
/// (清单名, cargo bin 名, 特权级)——**这里是唯一声明「程序装成哪种空间」的地方**：内核与各域
/// 都从清单里读，不再硬编码。码（`ProgramKind` → u32）在 `env::wire::manifest` 里写死一次。
const PRODUCTS: &[(&str, &str, ProgramKind)] = &[
    ("root", "prog-root", ProgramKind::Supervisor),
    // 调试回显：**U 态**（最小特权）——它只走 `env` 的调试面（`DebugCall`），
    // 够不着建域那道 S 态门。
    ("echo", "prog-echo", ProgramKind::User),
    // 客人：**U 态**（与 `echo` 同一档）——按名字找到一个服务、说一句话。铸孔、交出、
    // 一问一答都不需要 S 态，故最小特权的域也能用板。
    ("guest", "prog-guest", ProgramKind::User),
    // 过客：**U 态**（同上）——起来、挂一个名字、**直接死**（不说再见）。它与 `guest` 只差
    // 少说那一句退场：板上那两本账的"死"判据读的都是"那一枚入口还答得出吗"（`Probe`）。
    ("passer", "prog-passer", ProgramKind::User),
    // 房客：**U 态**（同上）——起来、占一条线、**直接死**。它与 `passer` 在线轴上同形：两位
    // 喂的都是"看出来的"那一档（板那本账 / 线那本账）。它领一枚门闩（`virtio_mmio@10001000`，
    // 1 号线——**一条没人要的线**）却从不映视图：领它只为"主人"这个说法是真的；占住线之后
    // 一句话不说就走，路由者靠 `sweep` 收掉它（读数 `router: line 1 = virtio_mmio@10001000`
    // 与 `router: vacate line=1`）。**照实记**：它从前占的是 11 号线（那时钟），第二台设备
    // 驱动上来之后那条线有主了，故换成 1 号线。
    ("lodger", "prog-lodger", ProgramKind::User),
    // 线路由者（中断面域）：**U 态**——实测（本行下面那条注里的疑点已经量掉）：它只读
    // PLIC 的寄存器（banner 里 PLIC 的 PMP 是 **S/U (R,W)**）、claim/complete、铸孔、挂组，
    // 全都不需要 S 态；它那枚铃是**内核给的**（铸铃那一格才是 S 态，本域不铸）。
    ("router", "prog-router", ProgramKind::User),
    // 串口驱动：**U 态**（同上）——持有 `serial@10000000`（PMP 也是 S/U (R,W)），把"收到
    // 字节就拉线"打开。**照实记**：这两格从前写 `Supervisor` 是照搬旧树，理由（"要读写
    // 寄存器"）与 banner 里那张 PMP 对不上；改成 U 态之后两道门（examine / soak）照旧全过。
    ("uart", "prog-uart", ProgramKind::User),
    // 第二台设备驱动：**U 态**（同上）——持有 `rtc@101000`（11 号线），武装闹钟、到点自己
    // 拉线；客人定的闹钟到点就清掉那一格、把"那一声"推回去。**它是"抽象等第二个实例"的那个
    // 第二例**：线那四格、配给、设备面这一整套在第二台真设备上再走一遍，**服务面**也在它上面
    // 第二次落地（`uart` 那一面只有一个方向，它这一面两个方向都有）。
    ("rtc", "prog-rtc", ProgramKind::User),
    // 客人：**U 态**（同上）——`/device/rtc` 那面服务的第一位用家：问一声现在几点、约一个时刻
    // （失败域那两格也各走一趟，见 `programs/src/user/sleeper.rs`），等到那一声就退场。
    ("sleeper", "prog-sleeper", ProgramKind::User),
    // 身份服务（**U 态**）：名册（TID → 当前 PrincipalId）与谱系（PrincipalId 的树）两张表，
    // 七条原语见 `protocol::principal`。它**不持有、不授予、不解释任何 Pie**——只读写自己
    // 那两张表，故不进"转授权中枢"那一档（与 `operator` 的差别正是在这里）：目录按角色分、
    // 特权级各自在这里声明，它是这一族里第一位 **U 态**的服务。
    ("principal", "prog-principal", ProgramKind::User),
    // 主体（**U 态**）：身份服务的第一位真客人——问自己是谁、查父（三态）、验自反与否、
    // 派生一条自己的子身份、再越权趟一次（读数见 `programs/src/user/subject.rs` 头注）。
    ("subject", "prog-subject", ProgramKind::User),
    // 结盟服务（**U 态**）：横向那张盟籍表（一条关系 + 一枚计数器），六条原语见
    // `protocol::coalition`。它同样**不持有、不授予、不解释任何 Pie**；它与身份服务那一台
    // 的差别只有一处——**它是身份服务的客人**：起手在树上找到 `/sys/principal`，每条写原语
    // 嵌一次 `Resolve(发送者)`（故"self"在适配层，不在核心）。
    ("coalition", "prog-coalition", ProgramKind::User),
    // 盟友（**U 态**）：结盟服务的第一位真客人——立两枚盟、进进出出、验幂等与第三态，
    // 再用派生的第二条身份验"同一枚盟里有两位"（读数见 `programs/src/user/member.rs` 头注）。
    ("member", "prog-member", ProgramKind::User),
    // 命名树的服务：**S 态**——它不建域、不碰 MMIO、不读设备，但它是这台机器的**转授权
    // 中枢**：谁在树上查到一条，它就 ship 一枚带 `VEST` 的副本出去（`protocol::operator::call::give`）。
    // 故它不进"最小特权"那一档（`echo` / `guest` / `passer` / `lodger`），与监督侧同档。
    ("operator", "prog-operator", ProgramKind::Supervisor),
    // 编排域：**S 态**——它要 mint/hatch（那是"建域 + 产线程 + 放行"整套），且整台机器
    // 的服务都由它起。它自己由**引导域**起：内核把 initrd 区与配对块只读借映进引导域，
    // 之后"这批字节交给谁"由域自己决定（见 `platform/devices.rs::supply_initrd`）。
    ("system", "prog-system", ProgramKind::Supervisor),
];

/// 探针那一档（**6 份**）：**只读数**的客人——`soak` / `examine` / `fair` 的判据就是它们打出来
/// 的那几行，故它们**必须**在验收镜像里（这是"测具出产品镜像"唯一做不到的一格）。
///
/// 它们住在隔壁那个 crate `harness`（照实记：用户裁定"测试和程序分开"）。
const PROBES: &[(&str, &str, ProgramKind)] = &[
    // 负证客人（**U 态**）：一位**没有身份**的任务去撞树的门（`Program::bind = false`）——
    // 门禁那条"没绑身份 ⇒ 拒绝"的判据在真机上的反例。读数见 `harness/src/probe_denied.rs`。
    ("probe-denied", "prog-probe-denied", ProgramKind::User),
    // 第二种负证（**U 态**）：**有身份**、但那一格归别人（`Rule::Owner`）⇒ 也拒。
    ("probe-owner", "prog-probe-owner", ProgramKind::User),
    // 规矩那一格的证客（**U 态**）：有身份的一台把 `Is` / `Under` / `In` 三条规矩落下去，
    // 先以自己试（正证），再换一位代表试（负证 + "看支不看相等"）。读数见
    // `harness/src/probe_rule.rs`。
    ("probe-rule", "prog-probe-rule", ProgramKind::User),
    // 另一位客人（**U 态**）：**有身份**地去用别人立了规矩的那两格 ⇒ 都该拒。
    // 那是"第二道门"的反例（第一道由 `probe-denied` 量）。读数见 `probe_rule_other.rs`。
    (
        "probe-rule-other",
        "prog-probe-rule-other",
        ProgramKind::User,
    ),
    // **深度那一格的证客**（**U 态**）：一层层往下 `part`。改前它是炸弹（持树者按深度递归，
    // 栈是 16 KiB）；改成按号直达之后它是正证。**不进 soak**——读数见 `probe_deep.rs`。
    ("probe-deep", "prog-probe-deep", ProgramKind::User),
    // 会死的持有者（**U 态**）：落一块**声明归自己**的门牌然后直接死——好让下一台接手。
    ("probe-lease", "prog-probe-lease", ProgramKind::User),
];

/// 压测台那一档（**10 份**）：台主（S 态，`SQWARE_ROOT=<名字>` 时当引导镜像）与它们的受害者
/// （U 态）。住 `harness`。
///
/// **一台只要它 `find(&boot, …)` 的那几个受害者**，见 [`bins_for`]。
const RIGS: &[(&str, &str, ProgramKind)] = &[
    // 压测台的两个（`harness/src/`）：`churn` = 受害者——U 态，不停地在
    // "挂着"与"在台上"之间换（那正是"他杀偶发不生效"那道缝要的状态）；`rig` = 台主——
    // S 态，`SQWARE_ROOT=rig` 时当引导镜像，反复造/杀它。
    ("churn", "prog-churn", ProgramKind::User),
    ("rig", "prog-rig", ProgramKind::Supervisor),
    // 忙机台的另外两个：`busy` = 占核者——U 态，纯自旋**永不落核**；`load` = 台主——
    // S 态，`SQWARE_ROOT=load` 时当引导镜像。它把每一颗核钉住，好让「到点兑现」这条债
    // 在树内第一次变得可测（`soak`/`rig` 里总有核空闲，空闲核会替全局兑现到点）。
    ("busy", "prog-busy", ProgramKind::User),
    ("park", "prog-park", ProgramKind::User),
    // 他杀台的握手版受害者（rig A）：无限挂在自己的孔上、由台主 push 唤醒——上台/离核的
    // 转折点因此由台主定（旧版 `churn` 是"放行即跑"，量到的全是快路径）。
    ("hang", "prog-hang", ProgramKind::User),
    ("load", "prog-load", ProgramKind::Supervisor),
    // 到点台的打点者：**S 态**（与两个台主同档），`SQWARE_ROOT=beat` 时当引导镜像。
    // 它不造任何东西，只量"睡到绝对点"漂不漂（两段对照，见程序头注）。
    ("beat", "prog-beat", ProgramKind::Supervisor),
    // 重启台：**S 态**（要 mint/hatch 那道门），`SQWARE_ROOT=again` 时当引导镜像。
    // 它在同一张表、同一行上把"起 → 停 → 放下 → 再起"走三遍（协议 §六 的"重发"）。
    ("again", "prog-again", ProgramKind::Supervisor),
    // 共享组台的两个（`harness/src/`）：`waiter` = 等待者——U 态，把台主
    // 给的那枚孔挂进**共享组**并等组键（**多个等待者挂同一只键**）；`group` = 台主——
    // S 态，`SQWARE_ROOT=group` 时当引导镜像：一次投信，看两个等待者是不是**都醒**，
    // 以及那条消息是不是**只归一个人**。
    ("waiter", "prog-waiter", ProgramKind::User),
    ("group", "prog-group", ProgramKind::Supervisor),
];

/// **场景 → 这一张镜像里装哪些程序**。
///
/// 照实记（用户裁定）：这原先是一张**并集**——不管跑哪个场景，31 台全打进 initrd。于是默认
/// （验收）镜像里躺着 10 台压测台（≈1.6 MB / 5.6 MB），而那 10 台在那张镜像里**永远不会被起**；
/// 反过来 `rig` 镜像（只要一个受害者）也装着整套产品与探针。现在一台只装它真要起的那几台。
fn bins_for(scenario: &str) -> Vec<(&'static str, &'static str, ProgramKind)> {
    match scenario {
        // 验收两景：**同一份镜像**（产品 + 全部探针）。`fair` 与 `root` 的差别只在编排域那张
        // 装配单多一条"聊天客人"（见 `image_name()`），不在装什么。
        "root" | "fair" => PRODUCTS.iter().chain(PROBES.iter()).copied().collect(),
        // 台子那几景：台主 + 它要的受害者（名字取自各台主里的 `const VICTIM` / `*_ELF`）。
        "rig" => pick(RIGS, &["rig", "hang"]),
        "again" => pick(RIGS, &["again", "churn"]),
        "load" => pick(RIGS, &["load", "busy", "park"]),
        "group" => pick(RIGS, &["group", "waiter"]),
        "beat" => pick(RIGS, &["beat"]),
        other => panic!(
            "未知场景 SQWARE_ROOT={other}（认得的：root / fair / rig / again / load / group / beat）"
        ),
    }
}

/// 从某张表里按名字挑几台——**按表里的次序**（清单次序即装载次序，`ROOT_OFFSET` 按位次算）。
fn pick(
    table: &[(&'static str, &'static str, ProgramKind)],
    want: &[&str],
) -> Vec<(&'static str, &'static str, ProgramKind)> {
    table
        .iter()
        .copied()
        .filter(|(name, _, _)| want.contains(name))
        .collect()
}

fn main() {
    // 内核链接脚本：workspace 化后不同 crate 用不同 -Tlink.ld（内核 0x80200000 /
    // 用户 0x10000），不能放根 .cargo/config.toml（全局 rustflags 冲突），改由
    // build.rs 传绝对路径。
    let ld = format!("{}/link.ld", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed=link.ld"); // link.ld 变更自动重链
    // 换引导镜像（压测台）只改这一个环境变量：让 cargo 在它变时重跑本脚本。
    println!("cargo::rerun-if-env-changed=SQWARE_ROOT");

    // 镜像程序 ELF 打包进 initrd（boot 不再 include_bytes 内嵌）：工作区没有
    // kernel→programs 的依赖边，`cargo clean` 后可能先编 kernel 而程序产物尚不存在
    // → initrd 打包报"文件缺失"。这里在编译前显式构建 programs crate，并从产物
    // 路径读字节写 initrd。
    //
    // 嵌套 cargo 必须用**独立 target 目录**（$OUT_DIR/programs）：宿主 cargo 会在
    // target 根持有 .cargo-build-lock，同目录再起 cargo 会互锁死等。隔离目录无此问题，
    // 且随 cargo clean 一并清除（每次从零重build，无陈旧产物）。
    let target = std_env::var("TARGET").expect("TARGET env missing");
    let bin_target =
        Path::new(&std_env::var("OUT_DIR").expect("OUT_DIR env missing")).join("programs");
    let cargo = std_env::var("CARGO").expect("CARGO env missing");
    let mut args = vec![
        "build".to_string(),
        // **两个包一起编**：产品（`programs`）与测具（`harness`）——后者依赖前者的 lib。
        "-p".to_string(),
        "programs".to_string(),
        "-p".to_string(),
        "harness".to_string(),
        "--target".to_string(),
        target.clone(),
        "--target-dir".to_string(),
        bin_target.to_str().expect("non-utf8 OUT_DIR").to_string(),
    ];
    // PROFILE = "debug" 对应 dev profile（cargo 不接受 --profile debug）；其余按名透传
    // （release 等）。内层 cargo 与宿主同 profile，产物目录名一致。
    let profile = std_env::var("PROFILE").expect("PROFILE env missing");
    if profile != "debug" {
        args.push("--profile".to_string());
        args.push(profile.clone());
    }
    // **场景旗标**：`SQWARE_ROOT=fair` 时给程序侧一个 `--cfg sqware_fair`。
    //
    // 照实记（两处坑，都是实测踩出来的）：
    //
    //  1) **不能用 `option_env!("SQWARE_ROOT")`**——cargo 不把 `env!` 读的变量算进指纹
    //     （本文件头注记着同一个坑：*"脚本何时重编不由那个变量决定（实测换变量后引导镜像没换）"*）。
    //     旗标得走 `rustflags` 那一族：它**进指纹**，换场景必然重编。
    //  2) **要写 `CARGO_ENCODED_RUSTFLAGS`，不是 `RUSTFLAGS`**——cargo 给 build 脚本的那个编码
    //     变量**优先级更高**：只设 `RUSTFLAGS` 会被它整个盖掉（实测：那版跑出来聊天客人一次
    //     都没出现）。而且编码变量里**已经带着** `.cargo/config.toml` 给本目标配的那几个
    //     `-C…` 旗标（`relocation-model` / `frame-pointers` / `code-model`）——往它后面追加，
    //     那几个才不会被顶掉。分隔符是 `\x1f`。
    let mut encoded = std_env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    if root_name() == "fair" {
        if !encoded.is_empty() {
            encoded.push('\x1f');
        }
        encoded.push_str("--cfg\x1fsqware_fair");
    }
    let status = Command::new(&cargo)
        .args(&args)
        .env("CARGO_ENCODED_RUSTFLAGS", &encoded)
        .status()
        .expect("failed to spawn cargo for programs crate");
    assert!(
        status.success(),
        "programs crate build failed (kernel packs program ELFs into initrd)"
    );

    // 写 initrd 清单（**临时机制**，见 kernel/src/initrd.rs）：与内核 ELF 同目录，
    // runner 从内核 ELF 的父目录取它传给 QEMU `-initrd`。格式的单一真相在
    // `env::wire::manifest`——读它的那一侧是 root 域，**写读共用一批判据**。
    let bin_dir = bin_target.join(&target).join(&profile);
    let main_profile = main_profile_dir(&std_env::var("OUT_DIR").expect("OUT_DIR env missing"));
    let blob_path = main_profile.join("initrd.img");
    let mut images: Vec<(ProgramKind, &str, Vec<u8>)> = Vec::new();
    let bins = bins_for(&root_name());
    for (name, bin, kind) in &bins {
        let elf = fs::read(bin_dir.join(bin))
            .unwrap_or_else(|e| panic!("initrd: read {bin} from {}: {e}", bin_dir.display()));
        images.push((*kind, name, elf));
    }
    let items: Vec<(ProgramKind, &str, &[u8])> = images
        .iter()
        .map(|(kind, name, elf)| (*kind, *name, elf.as_slice()))
        .collect();
    let (blob, spans) =
        manifest::pack(&items).expect("initrd: 清单越界（条数 / 名字长度 / 空镜像）");
    // 引导镜像在清单内的偏移/长度（内核不解析清单，按这两个常量取 root 的 ELF）
    let at = bins
        .iter()
        .position(|(name, _, _)| *name == image_name())
        .unwrap_or_else(|| panic!("initrd: SQWARE_ROOT={} 不在这一景的清单里", root_name()));
    let root_span = spans[at].clone();
    assert!(!root_span.is_empty(), "initrd: root image is empty");
    println!("cargo::rustc-env=ROOT_OFFSET={}", root_span.start);
    println!("cargo::rustc-env=ROOT_LEN={}", root_span.len());
    fs::write(&blob_path, &blob)
        .unwrap_or_else(|e| panic!("initrd: write {}: {e}", blob_path.display()));
    println!(
        "initrd packed: {} ({} B, {} programs)",
        blob_path.display(),
        blob.len(),
        bins.len()
    );
    // 程序侧任何一层变了，initrd 里的程序也得重打包——否则内核重编而镜像是旧的。
    //
    // **逐文件**，不是逐目录：`rerun-if-changed=<目录>` 只在**目录条目增删**时触发，
    // 原地改一个文件**不触发**（cargo 对目录只比 mtime，而改文件不改目录的 mtime）。
    // 这几条曾经漏掉过一次，之后改成 watch 目录——但仍然漏掉"原地改文件"，症状是
    // "改了程序、行为不变"，最费时间的一类假象。逐文件列全之后这一格不再存在。
    watch(Path::new(".."));
}

/// 逐文件登记"本脚本要盯的东西"：workspace 的源码与清单。
///
/// 走 `../programs`、`../crates` 两棵子树（跳过 `target`）＋工作区清单（`Cargo.toml`
/// / `Cargo.lock`：依赖版本变了同样要重打包）。**只盯文件，不盯目录**——理由见调用处。
fn watch(root: &Path) {
    for tree in ["programs", "crates"] {
        walk(&root.join(tree));
    }
    for manifest in ["Cargo.toml", "Cargo.lock"] {
        let at = root.join(manifest);
        if at.is_file() {
            println!("cargo::rerun-if-changed={}", at.display());
        }
    }
}

/// 递归登记一棵子树里**每一个文件**。
fn walk(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let at = entry.path();
        let name = entry.file_name();
        // 产物目录与隐藏目录不进：它们不是"源"，盯它们只会白白重跑。
        if name == "target" || name.to_string_lossy().starts_with('.') {
            continue;
        }
        if at.is_dir() {
            walk(&at);
        } else {
            println!("cargo::rerun-if-changed={}", at.display());
        }
    }
}

/// 从 OUT_DIR（`.../<profile>/build/<pkg>/<hash>/out`）向上找到 `<profile>` 目录：
/// 它是**唯一**直接包含 `build` 子目录的祖先（内核 ELF 与 runner 读 initrd 均在此）。
fn main_profile_dir(out_dir: &str) -> PathBuf {
    let mut cur: &Path = Path::new(out_dir);
    while !cur.join("build").is_dir() {
        cur = cur.parent().expect("OUT_DIR layout too shallow");
    }
    cur.to_path_buf()
}
