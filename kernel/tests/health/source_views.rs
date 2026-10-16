//! 来源↔关系关联：核心关系在首条 PTE 之前登记，死项及时摘掉、不留高水位。

use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::VirtAddr;
use crate::core::memory::flat::Frame;
use crate::core::memory::flat::allocator::statistics;
use crate::core::memory::flat::allocator::statistics::{Kind, Kinds};
use crate::core::memory::nest::View;
use crate::core::memory::nest::{Access, source_views};
use crate::core::work::unit::space::{Space, SpaceBuilder};

use super::support;

const BASE: usize = 0x6A00_0000;
const OTHER: usize = 0x6C00_0000;
const HOLD: usize = 0x6E00_0000;
const BURST: usize = 0x7000_0000;
/// 同时在世的瞬时关系条数：一次分配的记录很小，要让一条全局容量的高水位
/// 落在一个整帧以上，同时在场的条数就得够多。
const BURST_COUNT: usize = 160;

/// 一根关系压在一份来源上；另一份来源不掺进来。
///
/// 强快照都在不持任何 core 锁的地方放掉；回收的真账由 `footprint` 用分配器读数看。
pub fn accept() {
    let space = SpaceBuilder::user().build().expect("source_views: space");
    let rw = Access::READ | Access::WRITE;
    let frames = vec![Frame::allocate().unwrap(), Frame::allocate().unwrap()];
    let view = support::map_frames(&space, VirtAddr::wrap(BASE), frames, rw, rw)
        .expect("source_views: first view");
    let source = view.source();

    let first = source_views::views_over(&source).expect("source_views: lookup");
    crate::expect!(
        first.iter().any(|known| Arc::ptr_eq(known, &view)),
        "source_views: a relation is registered by its source"
    );

    let other_frames = vec![Frame::allocate().unwrap()];
    let other_view = support::map_frames(&space, VirtAddr::wrap(OTHER), other_frames, rw, rw)
        .expect("source_views: second view");
    let other = other_view.source();
    crate::expect!(
        other.source_id() != source.source_id(),
        "source_views: two windows, two sources"
    );
    let second = source_views::views_over(&source).expect("source_views: lookup again");
    crate::expect!(
        second.iter().any(|known| Arc::ptr_eq(known, &view)),
        "source_views: the first source keeps its own relation"
    );
    crate::expect!(
        !second.iter().any(|known| Arc::ptr_eq(known, &other_view)),
        "source_views: another source stays out of these candidates"
    );
    let third = source_views::views_over(&other).expect("source_views: other lookup");
    crate::expect!(
        third.iter().any(|known| Arc::ptr_eq(known, &other_view)),
        "source_views: the other relation is registered under its own source"
    );

    // 每一份强快照各自放掉。
    drop(first);
    drop(second);
    drop(third);
    // 关掉这个地址空间，把两段关系从索引与待撤表里交回来。
    space
        .nest
        .close()
        .expect("source_views: close the address space");
    drop(other_view);
    drop(view);
    drop(space);

    // 登记是幂等的：同一关系同一来源不会因为重跑再添一条。
    let repeat = support::map_frames(
        &SpaceBuilder::user()
            .build()
            .expect("source_views: second space"),
        VirtAddr::wrap(BASE),
        vec![Frame::allocate().unwrap()],
        rw,
        rw,
    )
    .expect("source_views: third view");
    source_views::views_over(&repeat.source()).expect("source_views: repeat lookup");
    footprint();
}

/// 一条常驻 + 一大批瞬时关系同时在世：瞬时全退之后，元数据回到只剩常驻的读数；
/// 常驻最后也走了就回到基线。读的是真实分配器（帧池净占用与逐类计数），
/// 不放宽成 `<=`，也不另加字节计数器。
pub fn footprint() {
    let rw = Access::READ | Access::WRITE;
    // 热身：先跑一遍同形状的建拆，让帧池与块池的驻留页先到位，读数才可比。
    let (warm_space, warm_live) = burst(rw);
    release_burst(warm_space, warm_live);
    let baseline = reading();

    let holder = SpaceBuilder::user()
        .build()
        .expect("source_views: holder space");
    let holder_entries = source_views::len();
    let held = support::map_frames(
        &holder,
        VirtAddr::wrap(HOLD),
        vec![Frame::allocate().unwrap()],
        rw,
        rw,
    )
    .expect("source_views: persistent view");
    let seated = reading();
    crate::expect!(
        seated.2 == holder_entries + 1,
        "source_views: one live relation is one entry: {} + 1 → {}",
        holder_entries,
        seated.2
    );

    // 一大批瞬时关系同时在世：它们真的占着帧，也真的都在链上。
    let (burst_space, live) = burst(rw);
    let crowded = reading();
    crate::expect!(
        crowded.2 > seated.2,
        "source_views: the burst and its space are on the chain: {} → {}",
        seated.2,
        crowded.2
    );
    crate::expect!(
        crowded.0 > seated.0,
        "source_views: a live burst occupies real frames: {} → {}",
        seated.0,
        crowded.0
    );
    release_burst(burst_space, live);
    expect_same("a released burst", seated, reading());

    holder
        .nest
        .close()
        .expect("source_views: close the holder space");
    drop(held);
    drop(holder);
    expect_same("a released persistent relation", baseline, reading());
}

/// 一次真账读数：帧池净占用、逐类计数、链上条数。
fn reading() -> (usize, Kinds, usize) {
    (
        statistics::frame_occupied() - statistics::block_occupied(),
        statistics::kinds(),
        source_views::len(),
    )
}

/// 两个读数必须逐项一样：条数、帧池净占用与每一类计数都不许漂。
///
/// `Prime` 是块池取自帧池的整页：它的账在 `block_occupied` 上，不在帧类目里。
fn expect_same(what: &str, before: (usize, Kinds, usize), after: (usize, Kinds, usize)) {
    crate::expect!(
        after.2 == before.2,
        "source_views: {what}: entries {} → {}",
        before.2,
        after.2
    );
    crate::expect!(
        after.0 == before.0,
        "source_views: {what}: frame occupancy {} → {} (net must be 0)",
        before.0,
        after.0
    );
    for kind in Kind::ALL {
        if kind == Kind::Prime {
            continue;
        }
        crate::expect!(
            after.1.get(kind) == before.1.get(kind),
            "source_views: {what}: kind {}: {} → {} (net must be 0)",
            kind.name(),
            before.1.get(kind),
            after.1.get(kind)
        );
    }
}

/// 一批瞬时关系同时在世：每条自己一条来源、自己一条关系，全部在同一地址空间里。
fn burst(rw: Access) -> (Space, Vec<Arc<View>>) {
    let space = SpaceBuilder::user()
        .build()
        .expect("source_views: burst space");
    let space_entries = source_views::len();
    let mut live = Vec::new();
    live.try_reserve_exact(BURST_COUNT)
        .expect("source_views: burst list");
    for index in 0..BURST_COUNT {
        let view = support::map_frames(
            &space,
            VirtAddr::wrap(BURST + index * PAGE_SIZE),
            vec![Frame::allocate().unwrap()],
            rw,
            rw,
        )
        .expect("source_views: burst view");
        live.push(view);
    }
    crate::expect!(
        source_views::len() == space_entries + BURST_COUNT,
        "source_views: all transient views are registered: {space_entries} + {BURST_COUNT} → {}",
        source_views::len()
    );
    (space, live)
}

/// 瞬时关系全部退场：地址空间关闭、关系放掉，链上一条不留。
///
/// 常驻那条不在这一批里，所以这一轮只证明“逐条释放”，不证明“全局空时清容量”。
fn release_burst(space: Space, live: Vec<Arc<View>>) {
    space
        .nest
        .close()
        .expect("source_views: close the burst space");
    drop(live);
    drop(space);
}
