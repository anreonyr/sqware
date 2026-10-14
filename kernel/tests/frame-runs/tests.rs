use alloc::sync::Arc;

use crate::core::memory::PAGE_SIZE;
use crate::core::memory::address::VirtAddr;
use crate::core::memory::flat::Frame;
use crate::core::memory::nest::NestError;
use crate::counting_alloc::count_allocations;

use super::runs::FrameRuns;
use super::{Access, ActiveRange};

const BASE: usize = 0x4000_0000;

const PA: usize = 0x8000_0000;

const RW: Access = Access::READ.union(Access::WRITE);

const R: Access = Access::READ;

type Segment = (usize, usize, usize, u64);

type Runs = Vec<Result<Segment, NestError>>;

// Access 不比相等（production 也比 bits），故期望值先折成 bits。
fn segment_at(start: usize, physical: usize, size: usize, current: Access) -> Segment {
    (start, physical, size, current.bits())
}

fn plan(pages: &[(usize, usize)]) -> Vec<(usize, Arc<Frame>)> {
    pages
        .iter()
        .map(|(page, physical)| (*page, Frame::at(*physical)))
        .collect()
}

fn range(start: usize, size: usize, current: Access) -> ActiveRange {
    ActiveRange {
        start: VirtAddr::from_raw(start),
        size: core::num::NonZeroUsize::new(size).unwrap(),
        ceiling: current,
        current,
    }
}

fn segment(frames: &[(usize, Arc<Frame>)], ranges: &[ActiveRange]) -> Runs {
    FrameRuns::new(VirtAddr::from_raw(BASE), frames, ranges)
        .map(|run| {
            run.map(|run| {
                segment_at(
                    run.start.as_usize(),
                    run.physical.as_usize(),
                    run.size,
                    run.current,
                )
            })
        })
        .collect()
}

#[test]
fn contiguous_pages_under_one_current_merge_into_a_single_run() {
    let frames = plan(&[(0, PA), (1, PA + PAGE_SIZE), (2, PA + 2 * PAGE_SIZE)]);
    let ranges = [range(BASE, 3 * PAGE_SIZE, RW)];
    assert_eq!(
        segment(&frames, &ranges),
        vec![Ok(segment_at(BASE, PA, 3 * PAGE_SIZE, RW))]
    );
}

#[test]
fn changed_current_cuts_contiguous_pages() {
    let frames = plan(&[(0, PA), (1, PA + PAGE_SIZE), (2, PA + 2 * PAGE_SIZE)]);
    let ranges = [
        range(BASE, PAGE_SIZE, RW),
        range(BASE + PAGE_SIZE, 2 * PAGE_SIZE, R),
    ];
    let got = segment(&frames, &ranges);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], Ok(segment_at(BASE, PA, PAGE_SIZE, RW)));
    assert_eq!(
        got[1],
        Ok(segment_at(
            BASE + PAGE_SIZE,
            PA + PAGE_SIZE,
            2 * PAGE_SIZE,
            R
        ))
    );
}

#[test]
fn page_index_gap_cuts_the_run() {
    let frames = plan(&[(0, PA), (2, PA + PAGE_SIZE)]);
    let ranges = [range(BASE, 3 * PAGE_SIZE, R)];
    let got = segment(&frames, &ranges);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], Ok(segment_at(BASE, PA, PAGE_SIZE, R)));
    assert_eq!(
        got[1],
        Ok(segment_at(
            BASE + 2 * PAGE_SIZE,
            PA + PAGE_SIZE,
            PAGE_SIZE,
            R
        ))
    );
}

#[test]
fn physical_gap_cuts_the_run() {
    let frames = plan(&[(0, PA), (1, PA + 2 * PAGE_SIZE)]);
    let ranges = [range(BASE, 2 * PAGE_SIZE, R)];
    let got = segment(&frames, &ranges);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], Ok(segment_at(BASE, PA, PAGE_SIZE, R)));
    assert_eq!(
        got[1],
        Ok(segment_at(
            BASE + PAGE_SIZE,
            PA + 2 * PAGE_SIZE,
            PAGE_SIZE,
            R
        ))
    );
}

#[test]
fn uncovered_frame_reports_no_region_and_iteration_resumes() {
    let frames = plan(&[(0, PA), (1, PA + PAGE_SIZE)]);
    let ranges = [range(BASE, PAGE_SIZE, R)];
    assert_eq!(
        segment(&frames, &ranges),
        vec![
            Ok(segment_at(BASE, PA, PAGE_SIZE, R)),
            Err(NestError::NoRegion)
        ]
    );

    let frames = plan(&[(0, PA), (1, PA + PAGE_SIZE), (2, PA + 2 * PAGE_SIZE)]);
    let ranges = [
        range(BASE, PAGE_SIZE, R),
        range(BASE + 2 * PAGE_SIZE, PAGE_SIZE, R),
    ];
    assert_eq!(
        segment(&frames, &ranges),
        vec![
            Ok(segment_at(BASE, PA, PAGE_SIZE, R)),
            Err(NestError::NoRegion),
            Ok(segment_at(
                BASE + 2 * PAGE_SIZE,
                PA + 2 * PAGE_SIZE,
                PAGE_SIZE,
                R
            )),
        ]
    );
}

#[test]
fn run_start_is_base_plus_page_index_times_page_size() {
    let frames = plan(&[(5, 0x9000_0000), (6, 0x9000_1000)]);
    let ranges = [range(BASE + 5 * PAGE_SIZE, 2 * PAGE_SIZE, R)];
    assert_eq!(
        segment(&frames, &ranges),
        vec![Ok(segment_at(
            BASE + 5 * PAGE_SIZE,
            0x9000_0000,
            2 * PAGE_SIZE,
            R
        ))]
    );
}

#[test]
fn base_need_not_be_page_aligned() {
    const ODD: usize = 0x5000_0123;
    let frames = plan(&[(7, 0xa000_0000)]);
    let ranges = [range(ODD + 7 * PAGE_SIZE, PAGE_SIZE, R)];
    let got: Runs = FrameRuns::new(VirtAddr::from_raw(ODD), &frames, &ranges)
        .map(|run| {
            run.map(|run| {
                segment_at(
                    run.start.as_usize(),
                    run.physical.as_usize(),
                    run.size,
                    run.current,
                )
            })
        })
        .collect();
    assert_eq!(
        got,
        vec![Ok(segment_at(
            ODD + 7 * PAGE_SIZE,
            0xa000_0000,
            PAGE_SIZE,
            R
        ))]
    );
}

#[test]
fn prepare_and_publish_passes_see_the_same_segmentation() {
    let frames = plan(&[(0, PA), (1, PA + 2 * PAGE_SIZE), (2, PA + 3 * PAGE_SIZE)]);
    let ranges = [
        range(BASE, 2 * PAGE_SIZE, RW),
        range(BASE + 2 * PAGE_SIZE, PAGE_SIZE, R),
    ];
    let prepare = segment(&frames, &ranges);
    let publish = segment(&frames, &ranges);
    assert_eq!(prepare, publish);
    assert_eq!(prepare.len(), 3);
}

#[test]
fn iteration_allocates_nothing() {
    let frames = plan(&[(0, PA), (1, PA + PAGE_SIZE), (2, PA + 2 * PAGE_SIZE)]);
    let ranges = [
        range(BASE, PAGE_SIZE, RW),
        range(BASE + PAGE_SIZE, 2 * PAGE_SIZE, R),
    ];
    let (pages, allocations) = count_allocations(|| {
        let mut pages = 0usize;
        for _ in 0..512 {
            for run in FrameRuns::new(VirtAddr::from_raw(BASE), &frames, &ranges) {
                pages += run.unwrap().size / PAGE_SIZE;
            }
        }
        pages
    });
    assert_eq!(pages, 512 * 3);
    assert_eq!(allocations, 0);
}
