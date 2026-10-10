use super::super::core::JobId;
use alloc::{collections::VecDeque, vec::Vec};
use pipe_client::{Direction, Port, Read, Write};
pub enum Buffer {
    Read { bytes: Vec<u8>, at: usize },
    Write { bytes: Vec<u8> },
    Terminal(Direction),
}
pub struct LocalPort {
    pub buffer: Buffer,
    pub closed: bool,
    pub bound: Option<JobId>,
}
impl LocalPort {
    pub fn direction(&self) -> Direction {
        match self.buffer {
            Buffer::Read { .. } => Direction::Read,
            Buffer::Write { .. } => Direction::Write,
            Buffer::Terminal(direction) => direction,
        }
    }
    pub fn bytes(&self) -> Option<&[u8]> {
        match &self.buffer {
            Buffer::Read { bytes, .. } | Buffer::Write { bytes } => Some(bytes),
            _ => None,
        }
    }
    pub fn read(
        &mut self,
        n: usize,
        input: &mut VecDeque<u8>,
        eof: bool,
    ) -> Result<Option<Vec<u8>>, &'static str> {
        if self.closed {
            return Err("port closed");
        }
        match &mut self.buffer {
            Buffer::Read { bytes, at } => {
                let end = at.saturating_add(n).min(bytes.len());
                let data = bytes[*at..end].to_vec();
                *at = end;
                Ok(Some(data))
            }
            Buffer::Terminal(Direction::Read) => {
                if input.is_empty() {
                    return if eof { Ok(Some(Vec::new())) } else { Ok(None) };
                }
                Ok(Some(
                    (0..n.min(input.len()))
                        .filter_map(|_| input.pop_front())
                        .collect(),
                ))
            }
            _ => Err("port is not readable"),
        }
    }
    pub fn write(
        &mut self,
        bytes: &[u8],
        output: &mut VecDeque<u8>,
        available: usize,
    ) -> Result<usize, &'static str> {
        if self.closed {
            return Err("port closed");
        }
        match &mut self.buffer {
            Buffer::Write { bytes: buffer } => {
                if bytes.len() > available {
                    return Err("buffer limit exceeded");
                }
                buffer
                    .try_reserve(bytes.len())
                    .map_err(|_| "buffer allocation")?;
                buffer.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            Buffer::Terminal(Direction::Write) => {
                if bytes.len() > available {
                    return Err("output limit exceeded");
                }
                output.extend(bytes);
                Ok(bytes.len())
            }
            _ => Err("port is not writable"),
        }
    }
}
pub struct Pump {
    pub local: u64,
    pub port: Port,
    pub done: bool,
    pub pending: Vec<u8>,
    pub at: usize,
}
impl Pump {
    pub fn source(&self) -> bool {
        self.port.direction() == Direction::Write
    }
    pub fn demand(&self) -> bool {
        matches!(
            env::mail::wait(
                self.port.token(),
                env::MailCondition::Signal(pipe_api::DEMAND_BIT),
                env::Wait::POLL
            ),
            Ok(true)
        )
    }
    pub fn hush_demand(&self) {
        let _ = env::mail::hush(self.port.token(), pipe_api::DEMAND_BIT);
    }
    pub fn source_step(
        &mut self,
        local: &mut LocalPort,
        input: &mut VecDeque<u8>,
        eof: bool,
    ) -> Result<(), &'static str> {
        if self.done {
            return Ok(());
        }
        if self.pending.is_empty() {
            if local.closed {
                let _ = self.port.close();
                self.done = true;
                return Ok(());
            }
            let Some(bytes) = local.read(256, input, eof)? else {
                return Ok(());
            };
            if bytes.is_empty() {
                let _ = self.port.close();
                self.done = true;
                return Ok(());
            }
            self.pending = bytes;
            self.at = 0;
        }
        match self.port.write(&self.pending[self.at..], env::Wait::POLL) {
            Ok(Write::Bytes(n)) => {
                self.at += n;
                if self.at == self.pending.len() {
                    self.pending.clear();
                    self.at = 0;
                }
            }
            Ok(Write::Pending) => {}
            Err(pipe_api::Fail::BrokenPipe | pipe_api::Fail::Closed | pipe_api::Fail::Dead) => {
                self.done = true;
                let _ = self.port.close();
            }
            Err(_) => return Err("pipe source failed"),
        }
        Ok(())
    }
    pub fn sink_step(
        &mut self,
        local: &mut LocalPort,
        output: &mut VecDeque<u8>,
        available: usize,
    ) -> Result<usize, &'static str> {
        if self.done {
            return Ok(0);
        }
        if local.closed {
            let _ = self.port.close();
            self.done = true;
            return Ok(0);
        }
        let mut bytes = [0; 256];
        match self.port.read(&mut bytes, env::Wait::POLL) {
            Ok(Read::Bytes(n)) => local.write(&bytes[..n], output, available),
            Ok(Read::Eof) | Err(pipe_api::Fail::Dead | pipe_api::Fail::Closed) => {
                self.done = true;
                let _ = self.port.close();
                Ok(0)
            }
            Ok(Read::Pending) => Ok(0),
            Err(_) => Err("pipe sink failed"),
        }
    }
}
