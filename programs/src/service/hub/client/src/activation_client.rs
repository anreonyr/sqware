use env::{PieToken, TaskId, Wait, pie, unit};
use hub_api::{
    self as hub,
    activation::{self, Activate},
    frame::Said,
};
use ipc::{hand::Sender, session::establish};
use resource::raw::{Hole, reserve};
use system_api::identity::CoalitionId;
use wire::Span as _;

/// Ask the kernel-Sire-owned face injected at launch, not a matching public mark.
/// **一趟报一批**（见 [`activation::Activate`]）：枚数写进帧里，`BACK` 那一条回话只有一个状态。
pub fn activate(task: TaskId, coalitions: &[CoalitionId]) -> Result<(), ()> {
    let sire = unit::sire();
    let entry = establish::find(sire, activation::ENTRY).ok_or(())?;
    if !matches!(reserve(entry), Ok((vestor, owner, mark))
        if vestor == sire && owner == sire && mark == activation::ENTRY)
    {
        return Err(());
    }
    let (back, seed) = establish::lend_out(entry, activation::BACK)?;
    struct Back(PieToken);
    impl Drop for Back {
        fn drop(&mut self) {
            let _ = pie::seal(self.0);
            let _ = pie::release(self.0);
        }
    }
    let _back = Back(back);
    let frame = Activate::of(task, coalitions, seed).ok_or(())?;
    let mut request = Sender::<Activate>::from_raw(entry);
    request
        .send_within(frame, Wait::AtMost(5000))
        .map_err(|_| ())?;
    let mut bytes = [0; Said::LEN];
    let (n, from) = Hole::from_raw(back)
        .pull(&mut bytes, Wait::AtMost(5000))
        .map_err(|_| ())?;
    let said = Said::fetch_at(&bytes[..n], 0).map(|one| one.0).ok_or(())?;
    (from == sire && said.status == hub::OK)
        .then_some(())
        .ok_or(())
}
