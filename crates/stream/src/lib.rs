#![no_std]
#![forbid(unsafe_code)]
use core::sync::atomic::{AtomicU8, AtomicU64, Ordering};

pub const DEFAULT_CAPACITY: usize = 16 * 1024;
pub const MAX_CAPACITY: usize = 1024 * 1024;
pub const MAGIC: u64 = 0x7371776172657069;
pub const VERSION: u64 = 1;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Read,
    Write,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    Invalid,
    Corrupt,
    Closed,
    BrokenPipe,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    Bytes(usize),
    Eof,
    Pending,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Write {
    Bytes(usize),
    Pending,
}
#[repr(C, align(64))]
pub struct Header {
    magic: AtomicU64,
    version: AtomicU64,
    capacity: AtomicU64,
    read: AtomicU64,
    write: AtomicU64,
    read_closed: AtomicU64,
    write_closed: AtomicU64,
    reserved: AtomicU64,
}
pub const HEADER_SIZE: usize = core::mem::size_of::<Header>();
impl Header {
    pub fn new(capacity: usize) -> Result<Self, Fail> {
        if capacity == 0 || capacity > MAX_CAPACITY {
            return Err(Fail::Invalid);
        }
        Ok(Self {
            magic: AtomicU64::new(MAGIC),
            version: AtomicU64::new(VERSION),
            capacity: AtomicU64::new(capacity as u64),
            read: AtomicU64::new(0),
            write: AtomicU64::new(0),
            read_closed: AtomicU64::new(0),
            write_closed: AtomicU64::new(0),
            reserved: AtomicU64::new(0),
        })
    }
    pub fn initialize(&self, capacity: usize) -> Result<(), Fail> {
        if capacity == 0 || capacity > MAX_CAPACITY {
            return Err(Fail::Invalid);
        }
        self.read.store(0, Ordering::Relaxed);
        self.write.store(0, Ordering::Relaxed);
        self.read_closed.store(0, Ordering::Relaxed);
        self.write_closed.store(0, Ordering::Relaxed);
        self.capacity.store(capacity as u64, Ordering::Relaxed);
        self.version.store(VERSION, Ordering::Relaxed);
        self.magic.store(MAGIC, Ordering::Release);
        Ok(())
    }
    pub fn close(&self, direction: Direction) {
        match direction {
            Direction::Read => &self.read_closed,
            Direction::Write => &self.write_closed,
        }
        .store(1, Ordering::Release);
    }
    pub fn closed(&self, direction: Direction) -> bool {
        match direction {
            Direction::Read => &self.read_closed,
            Direction::Write => &self.write_closed,
        }
        .load(Ordering::Acquire)
            != 0
    }
}
pub struct Ring<'a> {
    header: &'a Header,
    bytes: &'a [AtomicU8],
    capacity: usize,
}
impl<'a> Ring<'a> {
    pub fn attach(
        header: &'a Header,
        bytes: &'a [AtomicU8],
        capacity: usize,
    ) -> Result<Self, Fail> {
        if capacity == 0 || capacity > MAX_CAPACITY || capacity > bytes.len() {
            return Err(Fail::Invalid);
        }
        let ring = Self {
            header,
            bytes,
            capacity,
        };
        ring.check()?;
        Ok(ring)
    }
    fn check(&self) -> Result<(), Fail> {
        if self.header.magic.load(Ordering::Acquire) != MAGIC
            || self.header.version.load(Ordering::Acquire) != VERSION
            || self.header.capacity.load(Ordering::Acquire) != self.capacity as u64
        {
            return Err(Fail::Corrupt);
        }
        Ok(())
    }
    fn cursors(&self) -> Result<(u64, u64), Fail> {
        self.check()?;
        let read = self.header.read.load(Ordering::Acquire);
        let write = self.header.write.load(Ordering::Acquire);
        if write < read || write - read > self.capacity as u64 {
            return Err(Fail::Corrupt);
        }
        Ok((read, write))
    }
    pub fn read(&self, out: &mut [u8]) -> Result<Read, Fail> {
        self.check()?;
        if self.header.closed(Direction::Read) {
            return Err(Fail::Closed);
        }
        if out.is_empty() {
            return Ok(Read::Bytes(0));
        }
        let (read, mut write) = self.cursors()?;
        let closed = self.header.closed(Direction::Write);
        if write == read && closed {
            write = self.header.write.load(Ordering::Acquire);
        }
        if write < read || write - read > self.capacity as u64 {
            return Err(Fail::Corrupt);
        }
        if write == read {
            return Ok(if closed { Read::Eof } else { Read::Pending });
        }
        let n = out.len().min((write - read) as usize);
        let next = read.checked_add(n as u64).ok_or(Fail::Corrupt)?;
        for (i, byte) in out[..n].iter_mut().enumerate() {
            *byte = self.bytes[(read as usize + i) % self.capacity].load(Ordering::Relaxed);
        }
        self.header.read.store(next, Ordering::Release);
        Ok(Read::Bytes(n))
    }
    pub fn write(&self, input: &[u8]) -> Result<Write, Fail> {
        self.check()?;
        if self.header.closed(Direction::Write) {
            return Err(Fail::Closed);
        }
        if self.header.closed(Direction::Read) {
            return Err(Fail::BrokenPipe);
        }
        if input.is_empty() {
            return Ok(Write::Bytes(0));
        }
        let (read, write) = self.cursors()?;
        let n = input.len().min(self.capacity - (write - read) as usize);
        if n == 0 {
            return Ok(Write::Pending);
        }
        let next = write.checked_add(n as u64).ok_or(Fail::Corrupt)?;
        for (i, &byte) in input[..n].iter().enumerate() {
            self.bytes[(write as usize + i) % self.capacity].store(byte, Ordering::Relaxed);
        }
        self.header.write.store(next, Ordering::Release);
        Ok(Write::Bytes(n))
    }
    pub fn ready(&self, direction: Direction) -> Result<bool, Fail> {
        let (read, write) = self.cursors()?;
        Ok(match direction {
            Direction::Read => {
                write != read
                    || self.header.closed(Direction::Write)
                    || self.header.closed(Direction::Read)
            }
            Direction::Write => {
                write - read < self.capacity as u64
                    || self.header.closed(Direction::Read)
                    || self.header.closed(Direction::Write)
            }
        })
    }
    pub fn close(&self, direction: Direction) {
        self.header.close(direction);
    }
}
#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Arc, vec::Vec};
    fn bytes(n: usize) -> Vec<AtomicU8> {
        (0..n).map(|_| AtomicU8::new(0)).collect()
    }
    #[test]
    fn capacity_partial_progress_wrap_and_eof() {
        let h = Header::new(4).unwrap();
        let data = bytes(4);
        let r = Ring::attach(&h, &data, 4).unwrap();
        assert_eq!(r.write(b"abcdef").unwrap(), Write::Bytes(4));
        assert_eq!(r.write(b"z").unwrap(), Write::Pending);
        let mut out = [0; 3];
        assert_eq!(r.read(&mut out).unwrap(), Read::Bytes(3));
        assert_eq!(&out, b"abc");
        assert_eq!(r.write(b"efg").unwrap(), Write::Bytes(3));
        r.close(Direction::Write);
        assert_eq!(r.read(&mut out).unwrap(), Read::Bytes(3));
        assert_eq!(&out, b"def");
        assert_eq!(r.read(&mut out).unwrap(), Read::Bytes(1));
        assert_eq!(out[0], b'g');
        assert_eq!(r.read(&mut out).unwrap(), Read::Eof);
        r.close(Direction::Read);
        r.close(Direction::Read);
        assert_eq!(r.read(&mut out), Err(Fail::Closed));
    }
    #[test]
    fn broken_pipe_corrupt_cursors_and_zero_operations() {
        let h = Header::new(4).unwrap();
        let data = bytes(4);
        let r = Ring::attach(&h, &data, 4).unwrap();
        assert_eq!(r.read(&mut []).unwrap(), Read::Bytes(0));
        assert_eq!(r.write(&[]).unwrap(), Write::Bytes(0));
        h.write.store(5, Ordering::Relaxed);
        assert_eq!(r.read(&mut [0]), Err(Fail::Corrupt));
        h.write.store(0, Ordering::Relaxed);
        h.close(Direction::Read);
        assert_eq!(r.write(&[1]), Err(Fail::BrokenPipe));
        assert!(Header::new(0).is_err());
        assert!(Ring::attach(&h, &data, 5).is_err());
    }
    #[test]
    fn concurrent_spsc_transfers_a_megabyte_without_loss() {
        let data = Arc::new(bytes(127));
        let h = Arc::new(Header::new(127).unwrap());
        let writer_data = data.clone();
        let writer_header = h.clone();
        let thread = std::thread::spawn(move || {
            let ring = Ring::attach(&writer_header, &writer_data, 127).unwrap();
            for i in 0..1024 * 1024 {
                loop {
                    if matches!(ring.write(&[(i % 251) as u8]).unwrap(), Write::Bytes(1)) {
                        break;
                    }
                    std::thread::yield_now();
                }
            }
            ring.close(Direction::Write);
        });
        let ring = Ring::attach(&h, &data, 127).unwrap();
        let mut out = [0; 53];
        let mut received = 0;
        loop {
            match ring.read(&mut out).unwrap() {
                Read::Bytes(n) => {
                    for &b in &out[..n] {
                        assert_eq!(b, (received % 251) as u8);
                        received += 1;
                    }
                }
                Read::Pending => std::thread::yield_now(),
                Read::Eof => break,
            }
        }
        thread.join().unwrap();
        assert_eq!(received, 1024 * 1024);
    }
}
