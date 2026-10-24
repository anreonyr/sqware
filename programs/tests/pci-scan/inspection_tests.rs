use env::{PieToken, wire::Span as _};
use wire::Message;
use crate::inspection::{self, Page, Scan};

#[test]
fn cursor_request_roundtrips_and_rejects_bad_boundaries() {
    let request = Scan { from: 65536, back: PieToken::mint(9) };
    let mut bytes = Scan::EMPTY;
    let len = request.store(&mut bytes).unwrap();
    assert_eq!(Scan::fetch(&bytes[..len]), Some(request));
    for end in 0..len { assert!(Scan::fetch(&bytes[..end]).is_none()); }
    let mut extended = bytes.to_vec();
    extended.push(0);
    assert!(Scan::fetch(&extended).is_none());
    Scan { from: 65537, ..request }.store_at(&mut bytes, 0).unwrap();
    assert!(Scan::fetch(&bytes).is_none());
}

#[test]
fn full_snapshot_roundtrips_including_last_byte() {
    let mut page = Page { status: wire::OK, next: 65536, requester: 65535, count: 1,
        end: inspection::EOF, bytes: [0; 256] };
    page.bytes[0] = 0x34;
    page.bytes[255] = 0xAB;
    let mut bytes = Page::EMPTY;
    assert_eq!(page.store(&mut bytes), Some(Page::LEN));
    assert_eq!(Page::fetch(&bytes), Some(page));
    for end in 0..Page::LEN { assert!(Page::fetch(&bytes[..end]).is_none()); }
    let mut extended = bytes.to_vec();
    extended.push(0);
    assert!(Page::fetch(&extended).is_none());
}

#[test]
fn invalid_count_end_or_replayed_cursor_cannot_be_serialized() {
    let base = Page { status: wire::OK, next: 8, requester: 7, count: 1,
        end: inspection::CAPACITY, bytes: [0; 256] };
    for page in [Page { count: 2, ..base }, Page { next: 7, ..base },
        Page { next: 65537, ..base }, Page { end: 4, ..base }, Page { count: 0, ..base }] {
        assert!(page.store(&mut Page::EMPTY).is_none());
    }
    let empty = Page { count: 0, end: inspection::WORK_LIMIT, ..base };
    let mut bytes = Page::EMPTY;
    empty.store(&mut bytes).unwrap();
    assert_eq!(Page::fetch(&bytes), Some(empty));
}

#[test]
fn production_reply_enumerates_other_vendors_and_keeps_empty_continuations() {
    let mut reader = crate::sparse::Sparse::new(513);
    reader.present(256, 0x1234, 0xABCD, 0x12_34_56);
    let empty = inspection::enumerate::scan(&reader, 0);
    assert_eq!((empty.status, empty.count, empty.next, empty.end),
        (wire::OK, 0, 256, inspection::WORK_LIMIT));
    let found = inspection::enumerate::scan(&reader, empty.next);
    assert_eq!((found.status, found.count, found.requester, found.next), (wire::OK, 1, 256, 257));
    assert_eq!(&found.bytes[..4], &[0x34, 0x12, 0xCD, 0xAB]);
    assert_eq!(&found.bytes[9..12], &[0x56, 0x34, 0x12]);
    assert_eq!(found.bytes.len(), 256);
    let eof = inspection::enumerate::scan(&reader, found.next);
    assert_eq!((eof.count, eof.next, eof.end), (0, 513, inspection::EOF));
}

#[test]
fn production_reply_rejects_bad_geometry_before_reading_and_reports_snapshot_failure() {
    let mut reader = crate::sparse::Sparse::new(3);
    assert_ne!(inspection::enumerate::scan(&reader, 4).status, wire::OK);
    assert_eq!(reader.reads(), 0);
    reader.present(1, 0x1234, 0x4321, 0);
    reader.fail(1, 255);
    let failed = inspection::enumerate::scan(&reader, 0);
    assert_ne!(failed.status, wire::OK);
    assert_eq!(failed.count, 0);
    assert_ne!(failed.end, inspection::EOF);
}
