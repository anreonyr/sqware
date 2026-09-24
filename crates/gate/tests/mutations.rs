//! 变异那一门 —— **判据的判据**。判据、表的来历与那几条教训都在 `gate::mutations` 头注。
//!
//! 这一门会**改源码再还原**（`Drop` 里做，panic 也还原），故它跑得慢、且**必须独占**
//! （libtest 默认多线程；这一门是唯一会写工作区源码的）。表里的 75 条在原量具那两遍全量里
//! 实测过，账接着用 ⇒ 默认一跑是**真空跑**。

use gate::mutations::{self, Hope};
use gate::*;

#[test]
#[ignore = "会改源码、且机器那一侧一轮半小时"]
fn mutations() {
    let all = mutations::all().expect("造不出那颗要跑的");
    let full = std::env::var("GATE_ALL").is_ok();
    let only = std::env::var("GATE_ONLY").ok();
    let at = std::env::var("GATE_AT").ok();

    let mut book = mutations::ledger_load();
    let mut rows: Vec<(String, String, String)> = Vec::new();
    let mut skipped = 0;
    let mut bad: Vec<String> = Vec::new();

    for m in &all {
        if let Some(a) = &at {
            let machine = matches!(m.bench, Bench::Machine { .. });
            if (a == "soak") != machine {
                continue;
            }
        }
        if let Some(o) = &only
            && !m.name.contains(o.as_str())
        {
            continue;
        }
        let k = mutations::key_of(m);
        if !full && book.contains_key(&k) {
            skipped += 1;
            continue;
        }

        let ctrl = m.name.starts_with("对照");
        match mutations::mutate(m) {
            Ok(v) => {
                let want_red = m.hope == Hope::Red;
                let verdict = if v.red {
                    if !want_red || ctrl {
                        bad.push(format!("{}：期望绿却红了——{}", m.name, v.detail));
                    }
                    "红".to_string()
                } else if ctrl {
                    "绿（对照，理应如此）".to_string()
                } else if want_red {
                    bad.push(format!("{}：**没牙**（该红却绿）", m.name));
                    "**绿**（没牙）".to_string()
                } else {
                    "绿（等价变异，理应如此）".to_string()
                };
                // 只把**真结论**记进账：编译红 / 补丁不唯一那几档走 `Err`，不记。
                book.insert(k, verdict.clone());
                rows.push((m.name.to_string(), verdict, v.detail.clone()));
            }
            Err(e) => rows.push((m.name.to_string(), format!("{e}"), String::new())),
        }
    }

    if !rows.is_empty() {
        mutations::ledger_save(&book);
    }
    if skipped > 0 {
        println!("（跳过 {skipped} 条：账里已经验过；要全跑加 `GATE_ALL=1`）");
    }
    if rows.is_empty() {
        println!("\n变异那一门：这次没有实跑的（都验过了）——`GATE_ALL=1` 全跑，或给 `GATE_ONLY=<名字>`。\n");
        return;
    }

    println!("\n{:<44}{:<22}红在哪", "变异", "门");
    for (n, v, d) in &rows {
        println!("{n:<44}{v:<22}{d}");
    }
    let red = rows.iter().filter(|(_, v, _)| v == "红").count();
    let ctrl = rows
        .iter()
        .filter(|(n, _, _)| n.starts_with("对照"))
        .count();
    let equiv = rows
        .iter()
        .filter(|(n, _, _)| n.starts_with("等价"))
        .count();
    let n_mut = rows.len() - ctrl - equiv;
    println!("\n变异那一门：{red}/{n_mut} 条变异被逮住（对照 {ctrl} 条 · 等价 {equiv} 条，预期绿）\n");

    assert!(bad.is_empty(), "牙口不过：\n  {}", bad.join("\n  "));
}
