use super::{CAPACITY, EOF, Page, WORK_LIMIT};
use super::super::{end::End, function::Function, reader::Reader, scan, snapshot::Snapshot};

pub fn scan(reader: &impl Reader, from: u32) -> Page {
    let mut found = [Function { requester: 0, vendor: 0, device: 0, class: 0 }];
    let Some(page) = scan::scan(reader, from, &mut found) else {
        return Page::of(super::super::api::fail_to_code(Some(super::super::api::Fail::Denied)));
    };
    let mut reply = Page {
        status: wire::OK, next: page.next, requester: 0, count: page.count as u8,
        end: match page.end { End::Capacity => CAPACITY, End::WorkLimit => WORK_LIMIT, End::Eof => EOF },
        bytes: [0; 256],
    };
    if page.count == 1 {
        let Some(snapshot) = Snapshot::read(reader, found[0].requester).filter(Snapshot::valid) else {
            return Page::of(super::super::api::fail_to_code(Some(super::super::api::Fail::Unavailable)));
        };
        reply.requester = snapshot.requester();
        reply.bytes.copy_from_slice(snapshot.bytes());
    }
    reply
}
