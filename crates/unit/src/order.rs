//! 这一张单自己算不了的那一件事：次序。

use super::{Bootstrap, Dependency, Ready, Target, UnitFile};

/// 图上说不通的那三种——每一种都报出**名字**
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DepsFail {
    /// 一条边指着本单里没有的台（名字）
    Unknown(&'static str),
    /// 被指着的那一台**没有凭据**（`demand.supply` 空 ⇒ 它交不出"我答得动"）
    NoEvidence(&'static str),
    /// 取不出可排的台 ⇒ 环（名字 = 卡住的那一个）
    Cycle(&'static str),
    Bootstrap(Bootstrap),
    DuplicateStart(&'static str),
    DuplicateTarget(&'static str),
}

/// 的排**最后**
/// 同一批按**名字**排（与声明次序无关，可复现）。图上那三种说不通当场挑出来（DepsFail）
pub fn order_scene(
    list: &mut [&'static UnitFile],
    targets: &[Target],
    bootstrap: &[Bootstrap],
) -> Result<(), DepsFail> {
    for i in 0..list.len() {
        if list[..i].iter().any(|prior| prior.name() == list[i].name()) {
            return Err(DepsFail::DuplicateStart(list[i].name()));
        }
    }
    for i in 0..targets.len() {
        if targets[..i]
            .iter()
            .any(|prior| prior.name == targets[i].name)
        {
            return Err(DepsFail::DuplicateTarget(targets[i].name));
        }
    }
    // 一、每条边都要落得下：指得到本单里的台，且那一台说得出"我答得动"。
    //     （见 SCENE 的头注）。
    let mut i = 0;
    while i < list.len() {
        if let Some(deps) = list[i].control.after {
            let mut d = 0;
            while d < deps.len() {
                match deps[d] {
                    Dependency::Unit(name) => match find(list, name) {
                        None => return Err(DepsFail::Unknown(name)),
                        Some(program) if !has_ready_evidence(program.control.ready) => {
                            return Err(DepsFail::NoEvidence(name));
                        }
                        Some(_) => {}
                    },
                    Dependency::Target(name) if target(targets, name).is_none() => {
                        return Err(DepsFail::Unknown(name));
                    }
                    Dependency::Target(name) => {
                        let barrier = target(targets, name).expect("validated target");
                        if let Some(member) = barrier
                            .members
                            .iter()
                            .find(|member| find(list, member).is_none())
                        {
                            return Err(DepsFail::Unknown(member));
                        }
                    }
                    Dependency::Bootstrap(task) if !bootstrap.contains(&task) => {
                        return Err(DepsFail::Bootstrap(task));
                    }
                    Dependency::Bootstrap(_) => {}
                }
                d += 1;
            }
        }
        i += 1;
    }
    for barrier in targets {
        for member in barrier.members {
            if find(list, member).is_none() {
                return Err(DepsFail::Unknown(member));
            }
        }
    }
    // 二、拓扑：一轮取"前置都排好了"的那一个；同批挑名字最小的（可复现）。
    let n = list.len();
    let mut placed = 0usize;
    while placed < n {
        let mut pick: Option<usize> = None;
        let mut i = placed;
        while i < n {
            if ready(list, i, placed, targets, bootstrap) {
                match pick {
                    Some(best) if list[best].name() <= list[i].name() => {}
                    _ => pick = Some(i),
                }
            }
            i += 1;
        }
        let Some(i) = pick else { break };
        list.swap(placed, i);
        placed += 1;
    }
    if placed < n {
        return Err(DepsFail::Cycle(list[placed].name()));
    }
    Ok(())
}

fn has_ready_evidence(ready: Ready) -> bool {
    match ready {
        Ready::OnSpawn => true,
        Ready::Signal(mark) => mark != env::Mark::NONE,
    }
}

/// 这一台的**边都排好了吗**（`list[..placed]` 里找得到每一条边指着的那一台）
fn ready(
    list: &[&'static UnitFile],
    i: usize,
    placed: usize,
    targets: &[Target],
    bootstrap: &[Bootstrap],
) -> bool {
    let Some(deps) = list[i].control.after else {
        return true;
    };
    let mut d = 0;
    while d < deps.len() {
        let found = match deps[d] {
            Dependency::Bootstrap(task) => bootstrap.contains(&task),
            Dependency::Target(name) => target(targets, name).is_some_and(|barrier| {
                barrier.members.iter().all(|member| {
                    list[..placed]
                        .iter()
                        .any(|program| program.name() == *member)
                })
            }),
            Dependency::Unit(name) => list[..placed].iter().any(|program| program.name() == name),
        };
        if !found {
            return false;
        }
        d += 1;
    }
    true
}

fn target<'a>(targets: &'a [Target], name: &str) -> Option<&'a Target> {
    targets.iter().find(|barrier| barrier.name == name)
}

/// 本单里按名字找那一台（**只查不比存** ⇒ 借 `&str`）
fn find<'a>(list: &[&'a UnitFile], name: &str) -> Option<&'a UnitFile> {
    let mut i = 0;
    while i < list.len() {
        if list[i].name() == name {
            return Some(list[i]);
        }
        i += 1;
    }
    None
}
