//! Trusted Login requests construction under a configured account identity.
use super::Fail;
use crate::common::path::Path;
use crate::communication::session::establish;
use crate::system::{
    loader::{Built, frame::Said},
    operator::Face,
};
use env::wire::Span as _;
use env::{PieToken, TaskId, TeamId, Wait, pie};
use ::resource::raw::{HolePie, reserve};
pub use super::marks::ACCOUNT_ENTRY as ENTRY;
pub use super::marks::ACCOUNT_BACK as BACK;
pub const DIR: &Path = Path::new("/svc/sys/control/account");
pub mod frame;
pub use frame::Request;
pub struct Client {
    entry: PieToken,
    host: TaskId,
}
impl Client {
    pub fn find(tree: &Face, wait: Wait) -> Result<Self, Fail> {
        Self::of(
            tree.tile(DIR, wait)
                .and_then(|tile| tile.token(wait))
                .map_err(|_| Fail::Bad)?,
        )
    }
    pub fn of(entry: PieToken) -> Result<Self, Fail> {
        let host = match reserve(entry) {
            Ok((_, host, mark)) if mark == ENTRY => host,
            _ => {
                let _ = pie::release(entry);
                return Err(Fail::Bad);
            }
        };
        Ok(Self { entry, host })
    }
    pub fn create(&self, account: &str, wait: Wait) -> Result<Built, Fail> {
        let (back, token) = establish::lend_out(self.entry, BACK).map_err(|_| Fail::Bad)?;
        let result = (|| {
            let request = Request {
                account: account.into(),
                back: token,
            };
            let mut bytes = [0; Request::LEN];
            let n = request.store_at(&mut bytes, 0).ok_or(Fail::Bad)?;
            HolePie::from_token(self.entry)
                .push(&bytes[..n], wait)
                .map_err(|_| Fail::Bad)?;
            let mut bytes = [0; Said::LEN];
            let (n, from) = HolePie::from_token(back)
                .pull(&mut bytes, wait)
                .map_err(|_| Fail::Bad)?;
            let (reply, end) = Said::fetch_at(&bytes[..n], 0).ok_or(Fail::Bad)?;
            if from != self.host || n != end {
                return Err(Fail::Bad);
            }
            if reply.status != super::frame::OK {
                return Err(super::frame::code_to_fail(reply.status).unwrap_or(Fail::Bad));
            }
            if reply.task.get() == 0 || reply.team == 0 {
                return Err(Fail::Bad);
            }
            Ok(Built {
                task: reply.task,
                team: TeamId::new(reply.team as usize),
            })
        })();
        let _ = pie::seal(back);
        let _ = pie::release(back);
        result
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = pie::release(self.entry);
    }
}
