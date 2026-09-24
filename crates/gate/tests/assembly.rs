//! 装配单自己的门 —— **不起机、不造镜像**：只看那张表（`plan::assembly::ALL` + `ENTRY`）。
//!
//! # 为什么值得单开一门
//!
//! `ALL` 是一张**数据表**，而它上面挂着几条"不许崩"的关系：景名得有人认 · 引导镜像得在清单里 ·
//! 次序得两两不同 · `spot`（角色）与 `scenes`（进哪张镜像）不许互相矛盾……这些都不是"哪一台的
//! 判据"，而是**这张表自己的判据**。它们出错时机器那一侧的症状是"某一景忽然起不来"，而根因在
//! 这张表里的一个字。故在这一层判：**秒级、不用 QEMU**，正好进 `cargo gate` 的默认那一档。
//!
//! **照实记（第四条是从一次红换来的）**：加 `product` 那一景时，打包那一格报
//! `initrd: SQWARE_ROOT=product 不在这一景的清单里`——而这一条**在表里本来就判得出来**
//! （`ENTRY` 说那一景由 `root` 起，可 `root` 那一行当时没写 `"product"`）。它现在在这里判：
//! 用不着先把镜像造出来，也就不会等到起机那一刻才知道。
//!
//! **照实记（第六条是 `spot` 收口那一刀留下的闸）**：`spot` 与 `scenes` 是两件独立的事
//! （角色 / 装不装），而它们**今天恰好有一条关系**："产品镜像 = 两个域 + 常驻服务 + 调试回显"。
//! 那条关系写在这里，是**闸**不是第二份账：谁把一位常客塞进产品镜像、或者给某台服务改了角色，
//! 当场红——而"进哪张镜像"仍然只有 `scenes` 一处说。

use plan::assembly::{ALL, ENTRY, Spot, entry_of};

/// 这一景要装的程序（与 `crates/image::bins_for` 同一条过滤；**次序即装载次序**）。
fn bins(scene: &str) -> Vec<&'static str> {
    ALL.iter()
        .filter(|row| row.scenes.contains(&scene))
        .map(|row| row.name)
        .collect()
}

#[test]
fn assembly() {
    let mut gaps: Vec<String> = Vec::new();

    // 一、名字不重复（清单名是"按名挑程序"那一手的键）。
    for (i, row) in ALL.iter().enumerate() {
        if ALL[..i].iter().any(|r| r.name == row.name) {
            gaps.push(format!("名字 `{}` 出现了两遍", row.name));
        }
    }

    // 二、每一行写的景名都得有人认（拼错 ⇒ 那一行**永远没人装**，而且一个信都不报）。
    for row in ALL {
        for scene in row.scenes {
            if entry_of(scene).is_none() {
                let known: Vec<&str> = ENTRY.iter().map(|(s, _)| *s).collect();
                gaps.push(format!(
                    "`{}` 写了一个没人认得的景 `{scene}`（认得的：{}）",
                    row.name,
                    known.join(" / ")
                ));
            }
        }
    }

    // 三、`ENTRY` 的每个景都得真的装得起（空景 = 一条也装不出来的场景）。
    // 四、**每个景的引导镜像必须在它自己的清单里**——`crates/image` 那一步的宿主侧孪生。
    for (scene, entry) in ENTRY {
        let list = bins(scene);
        if list.is_empty() {
            gaps.push(format!("景 `{scene}` 一条也装不出来（`ENTRY` 里列着它）"));
            continue;
        }
        if !list.contains(entry) {
            gaps.push(format!(
                "景 `{scene}` 的引导镜像 `{entry}` 不在它的清单里：{list:?}"
            ));
        }
    }

    // 五、次序是契约（`sort_by_key(order)` ＋ "等最后一条退场"）：两条同号 ⇒ "谁是最后一条"
    //     没有答案，而整个收场就挂在那一句上。
    let mut orders: Vec<(u8, &str)> = ALL
        .iter()
        .filter_map(|row| row.plan.as_ref().map(|p| (p.order, row.name)))
        .collect();
    orders.sort_unstable();
    for w in orders.windows(2) {
        if w[0].0 == w[1].0 {
            gaps.push(format!(
                "`{}` 与 `{}` 抢同一个 `order`={}",
                w[0].1, w[1].1, w[0].0
            ));
        }
    }

    // 六、`spot`（角色）与 `scenes`（进哪张镜像）**不许互相矛盾**——"两处说同一件事"的那道闸。
    for row in ALL {
        let in_product = row.scenes.contains(&"product");
        let wants = matches!(row.spot, Spot::Domain | Spot::Service | Spot::Console);
        if in_product != wants {
            gaps.push(format!(
                "`{}`：`spot={:?}` 与 `scenes={:?}` 对不上 —— 产品镜像里只该有两个域 / 常驻服务 / 调试回显",
                row.name, row.spot, row.scenes
            ));
        }
    }

    assert!(gaps.is_empty(), "装配单本身不对：\n  {}", gaps.join("\n  "));
    println!(
        "assembly: {} 行 · {} 景 · 产品那一景 {} 条 · 编排域要起 {} 条",
        ALL.len(),
        ENTRY.len(),
        bins("product").len(),
        ALL.iter().filter(|r| r.plan.is_some()).count()
    );
}
