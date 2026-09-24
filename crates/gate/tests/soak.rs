//! 默认那一景的读数门：**判据住 `gate::soak`**，这一层只管起机与报数（变异那一门要能直接调
//! 那份判据）。起机那几格的照实记（等标记再喂、icount、`GATE_ROUNDS`）都在 `gate::soak` 头注。

use gate::*;

#[test]
#[ignore = "要起 QEMU"]
fn soak() {
    let rounds: usize = std::env::var("GATE_ROUNDS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let image = build(Scenario::Root, Profile::Debug).expect("造不出那颗要跑的");

    let mut pass = 0;
    let mut why_all = Vec::new();
    for i in 1..=rounds {
        let t = run(&Bench::Machine {
            image: image.clone(),
            sched: soak::feed(),
            within: secs(45),
            env: &[],
        })
        .expect("机器那一台起不动");
        let _ = t.keep(&log_path(&format!("soak-{i}")));

        match soak::verdict(&t) {
            Ok(()) => {
                pass += 1;
                println!("round {i}: PASS");
            }
            Err(gaps) => {
                println!("round {i}: FAIL —— 下面这几条没过：");
                for g in &gaps {
                    println!("  {g}");
                }
                why_all.push(format!("round {i}: {} 条不过", gaps.len()));
            }
        }
    }

    assert_eq!(
        pass,
        rounds,
        "soak {pass}/{rounds}\n  {}",
        why_all.join("\n  ")
    );
}
