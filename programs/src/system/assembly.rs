//! Construction and installation work shared by System entry points.
use crate::system::{
    control::unit::Control,
    loader::{Image, Spawn},
};
use ::schedule::{BuildError, Plan, Progress, Res, ResMut, Schedule};
use alloc::vec::Vec;
use core::num::NonZeroU64;
use env::TaskId;
use ipc::rpc::reply::Sender;
use system_api::control::construction::{CANCELLED, CreateId, PENDING, UNSEEN};
use system_api::{control::Fail, identity::Install, loader::Said};

fn sha256(input: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    fn compress(h: &mut [u32; 8], block: &[u8; 64], k: &[u32; 64]) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes(word.try_into().unwrap());
        }
        for i in 16..64 {
            let x = w[i - 15];
            let y = w[i - 2];
            let s0 = x.rotate_right(7) ^ x.rotate_right(18) ^ (x >> 3);
            let s1 = y.rotate_right(17) ^ y.rotate_right(19) ^ (y >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut z] = *h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = z
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(k[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            z = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (state, value) in h.iter_mut().zip([a, b, c, d, e, f, g, z]) {
            *state = state.wrapping_add(value);
        }
    }
    for chunk in input.chunks_exact(64) {
        compress(&mut h, chunk.try_into().unwrap(), &K);
    }
    let rem = input.chunks_exact(64).remainder();
    let mut tail = [0u8; 128];
    tail[..rem.len()].copy_from_slice(rem);
    tail[rem.len()] = 0x80;
    let padded = if rem.len() < 56 { 64 } else { 128 };
    tail[padded - 8..padded].copy_from_slice(&(input.len() as u64).wrapping_mul(8).to_be_bytes());
    for block in tail[..padded].chunks_exact(64) {
        compress(&mut h, block.try_into().unwrap(), &K);
    }
    let mut digest = [0u8; 32];
    for (out, value) in digest.chunks_exact_mut(4).zip(h) {
        out.copy_from_slice(&value.to_be_bytes());
    }
    digest
}

#[cfg(test)]
mod fingerprint_tests {
    use super::sha256;

    #[test]
    fn sha256_matches_standard_vectors() {
        assert_eq!(
            sha256(b""),
            [
                0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
                0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
                0x78, 0x52, 0xb8, 0x55,
            ]
        );
        assert_eq!(
            sha256(b"abc"),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad,
            ]
        );
        for (len, expected) in [
            (
                55,
                "2900465fcb533e05a158fd2b3be0e5e3b03740d83060aa3580e0d98a96bf2384",
            ),
            (
                56,
                "31454ff48ef36af2f08fd511bdc37d9d5855ac23e992e5ff5445cb6b7674a674",
            ),
            (
                63,
                "5f6401b96532c36de4e65beec0409b69b1d181864c8009b7a04f43e5d56350d1",
            ),
            (
                64,
                "94eb5de4943613fd048dc93393ab06877405faa39c11f53e9386083339833e7e",
            ),
            (
                65,
                "fc518669b6eb4b4dd91827ecacef86689c725bd5bab888fd3b26dbb196eec954",
            ),
            (
                4096,
                "4e441a3533bb2c10cd5649981d395744213e09a336746b5a3458fee4057205ec",
            ),
        ] {
            let input: alloc::vec::Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let expected: [u8; 32] = core::array::from_fn(|i| {
                u8::from_str_radix(&expected[2 * i..2 * i + 2], 16).unwrap()
            });
            assert_eq!(sha256(&input), expected, "length {len}");
        }
        let pattern: alloc::vec::Vec<u8> = (0..256).map(|i| (i * 37 + 11) as u8).collect();
        let mut maximum = alloc::vec::Vec::with_capacity(16 * 1024 * 1024);
        for _ in 0..(16 * 1024 * 1024 / pattern.len()) {
            maximum.extend_from_slice(&pattern);
        }
        let expected = [
            0x83, 0xf8, 0xf2, 0x38, 0x90, 0x35, 0xd0, 0x70, 0x5d, 0x74, 0xff, 0xf3, 0x95, 0xa7,
            0x16, 0x27, 0x03, 0x3e, 0x24, 0x57, 0xb9, 0xf5, 0xdd, 0xbe, 0xa7, 0xb7, 0xf6, 0xfd,
            0x87, 0x4a, 0xf6, 0x6c,
        ];
        assert_eq!(sha256(&maximum), expected, "maximum image snapshot");
    }
}

pub(crate) struct Jobs {
    catalog: Option<crate::boot::Catalog<'static>>,
    work: WorkQueue<Work>,
    launches: Vec<Launch>,
    creates: Vec<CreateRecord>,
    create_watermarks: Vec<CreateWatermark>,
    closed_create_scopes: Vec<ClosedCreateScope>,
    hook_instance: Option<InstanceRef>,
    hook_job: Option<JobId>,
    hook_next: usize,
}
impl Default for Jobs {
    fn default() -> Self {
        Self {
            catalog: None,
            work: WorkQueue::default(),
            launches: Vec::new(),
            creates: Vec::new(),
            create_watermarks: Vec::new(),
            closed_create_scopes: Vec::new(),
            hook_instance: None,
            hook_job: None,
            hook_next: 0,
        }
    }
}
pub(crate) struct Launch {
    pub instance: Launched,
    pub installation: InstanceInstall,
}
pub(crate) struct InstanceInstall {
    pub delivery: Delivery,
    pub service: Option<crate::system::control::unit::Service>,
    pub identity: Option<system_api::identity::InstallId>,
    pub startup: system_api::loader::StartupMetadata,
    pub bootstrap: Option<bootstrap::Carrier>,
    pub retirement: Vec<retirement::Participant>,
}
pub(crate) struct Launched {
    pub reference: InstanceRef,
    pub task: TaskId,
    pub create: Option<CreateKey>,
}
pub(crate) struct Delivery {
    pub owner: TaskId,
    pub identity: Install,
    pub program: Option<&'static crate::unit::UnitFile>,
    pub back: Option<Sender<Said>>,
}
pub(crate) struct Request {
    pub image: ImageRequest,
    pub delivery: Delivery,
    pub create: Option<CreateKey>,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct JobId(NonZeroU64);
impl JobId {
    pub(crate) fn new(raw: u64) -> Option<Self> {
        NonZeroU64::new(raw).map(Self)
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct InstanceRef {
    pub job: JobId,
    pub task: TaskId,
}
pub(crate) struct HookSelection {
    pub(crate) reference: InstanceRef,
    pub(crate) job: JobId,
    pub(crate) next: usize,
}
enum Work {
    Construct { id: JobId, request: Request },
    Lifecycle(crate::system::control::lifecycle::Tracked),
    Phase(Phase),
    Command(Command),
    Completed(JobCompletion),
}
pub(crate) enum JobOutcome {
    Construct(InstanceRef),
    Prepare(InstanceRef),
    Activate(InstanceRef),
    Retire(InstanceRef),
    Release(InstanceRef),
    Debark(InstanceRef),
    StaticLifecycle(crate::system::control::lifecycle::Action, Option<TaskId>),
    Command(system_api::control::frame::Said),
    CreateReplay(Said),
    Cancelled,
}
pub(crate) enum JobFailure {
    Construction(Fail),
    Control(crate::system::control::unit::verdict::Fail),
    Installation(&'static str),
    Command(system_api::control::Fail),
}
pub(crate) enum JobReply {
    None,
    Construction(Option<Sender<Said>>),
    Lifecycle {
        back: Option<Sender<system_api::control::frame::Said>>,
        waiters: Vec<Sender<system_api::control::frame::Said>>,
    },
    Command {
        reply: Option<Sender<system_api::control::frame::Said>>,
        waiters: Vec<Sender<system_api::control::frame::Said>>,
    },
}
pub(crate) struct JobResult {
    pub(crate) outcome: Result<JobOutcome, JobFailure>,
    pub(crate) reply: JobReply,
}
struct ConstructFinish {
    id: JobId,
    result: Result<JobOutcome, JobFailure>,
    back: Option<Sender<Said>>,
    key: Option<CreateKey>,
}
pub(crate) type JobCompletion = work::Completed<JobResult>;
#[path = "assembly/work.rs"]
mod work;
mod bootstrap;
use work::WorkQueue;
impl work::JobIdentity for Work {
    fn completed_id(&self) -> Option<JobId> {
        match self {
            Work::Completed(completion) => work::JobIdentity::completed_id(completion),
            _ => None,
        }
    }
}
struct Phase {
    id: JobId,
    reference: InstanceRef,
    key: crate::system::control::instance::hook::Key,
    cursor: ::schedule::Cursor,
}
pub(crate) struct Command {
    pub id: JobId,
    pub reference: InstanceRef,
    pub wire: system_api::control::frame::Wire,
    pub from: TaskId,
    pub reply: Option<Sender<system_api::control::frame::Said>>,
    pub waiters: Vec<Sender<system_api::control::frame::Said>>,
    pub activation_deadline: Option<u64>,
}
#[path = "assembly/input.rs"]
mod input;
pub(crate) use input::Source as ImageRequest;
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct CreateKey {
    pub entry: env::PieToken,
    pub caller: TaskId,
    pub id: CreateId,
}
struct CreateRecord {
    key: CreateKey,
    construct: Option<JobId>,
    prepare: Option<JobId>,
    prepared: bool,
    owner: TaskId,
    subject: Option<system_api::identity::Subject>,
    input: Option<system_api::control::construction::Input>,
    fingerprint: Option<[u8; 32]>,
    said: Said,
    cancelled: bool,
}
struct CreateWatermark {
    entry: env::PieToken,
    caller: TaskId,
    highest: u64,
}
struct ClosedCreateScope {
    entry: env::PieToken,
    caller: TaskId,
}
fn same_input(
    old: &system_api::control::construction::Input,
    new: &system_api::control::construction::Input,
) -> bool {
    use system_api::control::construction::Input;
    match (old, new) {
        (Input::Unit(old), Input::Unit(new)) => old == new,
        (Input::Image(old), Input::Image(new)) => {
            old.offset == new.offset
                && old.len == new.len
                && old.stack == new.stack
                && old.count == new.count
                && old.args == new.args
        }
        _ => false,
    }
}
fn retained_input(
    input: &system_api::control::construction::Input,
) -> Result<system_api::control::construction::Input, Fail> {
    use system_api::control::construction::{Image, Input, UnitRef};
    Ok(match input {
        Input::Image(image) => Input::Image(Image {
            image: env::PieToken::NONE,
            ..*image
        }),
        Input::Unit(unit) => {
            let mut name = alloc::string::String::new();
            name.try_reserve_exact(unit.name.len())
                .map_err(|_| Fail::Full)?;
            name.push_str(&unit.name);
            Input::Unit(UnitRef {
                name,
                stack: unit.stack,
                count: unit.count,
                args: unit.args,
            })
        }
    })
}

impl Jobs {
    fn complete(&mut self, completion: JobCompletion) {
        self.work.push_back(Work::Completed(completion));
    }
    pub(crate) fn take(&mut self, id: JobId) -> Option<JobCompletion> {
        match self.work.take(id)? {
            Work::Completed(completion) => Some(completion),
            _ => None,
        }
    }
    pub(crate) fn complete_lifecycle(
        &mut self,
        tracked: crate::system::control::lifecycle::Tracked,
    ) {
        let operation = tracked.operation;
        let id = operation.key.id;
        let action = operation.key.request.action;
        let task = operation.progress.execution.task;
        let result = match operation.progress.failure {
            Some(fail) => Err(JobFailure::Control(fail)),
            None => Ok(JobOutcome::StaticLifecycle(action, task)),
        };
        let reply = JobReply::Lifecycle {
            back: operation.key.request.back,
            waiters: operation.key.waiters,
        };
        self.complete(JobCompletion {
            id,
            result: JobResult {
                outcome: result,
                reply,
            },
        });
    }
    pub(crate) fn complete_phase(
        &mut self,
        id: JobId,
        result: Result<JobOutcome, JobFailure>,
    ) -> bool {
        let Some(at) = self.work.iter().position(|work| {
            matches!(work,
            Work::Phase(phase) if phase.id == id)
        }) else {
            return false;
        };
        self.work.remove(at);
        self.complete(JobCompletion {
            id,
            result: JobResult {
                outcome: result,
                reply: JobReply::None,
            },
        });
        true
    }
    pub(crate) fn complete_command(
        &mut self,
        command: Command,
        result: Result<JobOutcome, JobFailure>,
    ) {
        self.complete(JobCompletion {
            id: command.id,
            result: JobResult {
                outcome: result,
                reply: JobReply::Command {
                    reply: command.reply,
                    waiters: command.waiters,
                },
            },
        });
    }
    fn complete_construct(&mut self, finish: ConstructFinish) {
        self.complete(JobCompletion {
            id: finish.id,
            result: JobResult {
                outcome: finish.result,
                reply: JobReply::Construction(finish.back),
            },
        });
    }
    fn allocate_job(&mut self) -> Result<JobId, Fail> {
        self.work.reserve_job(272).ok_or(Fail::Full)
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.work.is_empty()
    }
    pub(crate) fn len(&self) -> usize {
        self.work.len()
    }
    pub(crate) fn lifecycle_jobs(
        &self,
    ) -> impl Iterator<Item = &crate::system::control::lifecycle::Tracked> {
        self.work.iter().filter_map(|work| match work {
            Work::Lifecycle(tracked) => Some(tracked),
            _ => None,
        })
    }
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.work.iter().any(|work| {
            matches!(work,
            Work::Lifecycle(tracked) if tracked.operation.key.request.name == name)
        })
    }
    pub(crate) fn enqueue(&mut self, request: Request) -> Result<JobId, Fail> {
        if self.work.len() >= 272 || self.work.try_reserve(1).is_err() {
            if let Some(key) = request.create {
                if let Some(record) = self.creates.iter_mut().find(|record| record.key == key) {
                    record.said = Said::from_result(Err(Fail::Full));
                }
            }
            request.image.release();
            if let Some(back) = request.delivery.back {
                crate::system::launch::reply(back, Err(Fail::Full));
            }
            return Err(Fail::Full);
        }
        let id = match self.allocate_job() {
            Ok(id) => id,
            Err(fail) => {
                if let Some(key) = request.create {
                    if let Some(record) = self.creates.iter_mut().find(|record| record.key == key) {
                        record.said = Said::from_result(Err(fail));
                    }
                }
                request.image.release();
                if let Some(back) = request.delivery.back {
                    crate::system::launch::reply(back, Err(fail));
                }
                return Err(fail);
            }
        };
        self.work.push_back(Work::Construct { id, request });
        if let Some(key) = self.work.iter().find_map(|work| match work {
            Work::Construct {
                id: queued,
                request,
            } if *queued == id => request.create,
            _ => None,
        }) {
            if let Some(record) = self.creates.iter_mut().find(|record| record.key == key) {
                record.construct = Some(id);
            }
        }
        Ok(id)
    }
    pub(crate) fn push_lifecycle(
        &mut self,
        mut request: crate::system::control::lifecycle::Request,
    ) -> Result<
        JobId,
        (
            crate::system::control::unit::verdict::Fail,
            crate::system::control::lifecycle::Request,
        ),
    > {
        for work in self.work.iter_mut() {
            let Work::Lifecycle(tracked) = work else {
                continue;
            };
            if tracked.operation.key.request.name != request.name {
                continue;
            }
            if matches!(
                (&tracked.operation.key.request.action, &request.action),
                (
                    crate::system::control::lifecycle::Action::Ruin,
                    crate::system::control::lifecycle::Action::Ruin
                )
            ) {
                if let Some(back) = request.back.take() {
                    if tracked.operation.key.waiters.try_reserve(1).is_err() {
                        request.back = Some(back);
                        return Err((crate::system::control::unit::verdict::Fail::Full, request));
                    }
                    tracked.operation.key.waiters.push(back);
                }
                return Ok(tracked.operation.key.id);
            }
            return Err((
                crate::system::control::unit::verdict::Fail::NotReady,
                request,
            ));
        }
        let Some(id) = self.work.reserve_job(272) else {
            return Err((crate::system::control::unit::verdict::Fail::Full, request));
        };
        self.work.push_back(Work::Lifecycle(
            crate::system::control::lifecycle::Tracked::new(id, request),
        ));
        Ok(id)
    }
    pub(crate) fn submit_lifecycle(
        &mut self,
        name: alloc::string::String,
        action: crate::system::control::lifecycle::Action,
    ) -> Result<JobId, crate::system::control::unit::verdict::Fail> {
        self.push_lifecycle(crate::system::control::lifecycle::Request {
            name,
            action,
            back: None,
        })
        .map_err(|(fail, _)| fail)
    }
    pub(crate) fn phase(
        &mut self,
        reference: InstanceRef,
        key: crate::system::control::instance::hook::Key,
    ) -> Result<JobId, crate::system::control::unit::verdict::Fail> {
        if let Some(phase) = self.work.iter().find_map(|work| match work {
            Work::Phase(phase) if phase.reference == reference && phase.key == key => Some(phase),
            _ => None,
        }) {
            return Ok(phase.id);
        }
        let id = self
            .work
            .reserve_job(272)
            .ok_or(crate::system::control::unit::verdict::Fail::Full)?;
        self.work.push_back(Work::Phase(Phase {
            id,
            reference,
            key,
            cursor: Default::default(),
        }));
        if key == crate::system::control::instance::hook::Key::Prepare {
            if let Some(create) = self
                .launches
                .iter()
                .find(|launch| launch.instance.reference == reference)
                .and_then(|launch| launch.instance.create)
            {
                if let Some(record) = self.creates.iter_mut().find(|record| record.key == create) {
                    record.prepare = Some(id);
                }
            }
        }
        Ok(id)
    }
    pub(crate) fn phase_cursor(&mut self, id: JobId) -> Option<&mut ::schedule::Cursor> {
        self.work.iter_mut().find_map(|work| match work {
            Work::Phase(phase) if phase.id == id => Some(&mut phase.cursor),
            _ => None,
        })
    }
    pub(crate) fn complete_create_prepare(&mut self, id: JobId, reference: InstanceRef) -> bool {
        let Some(key) = self
            .launches
            .iter()
            .find(|launch| launch.instance.reference == reference)
            .and_then(|launch| launch.instance.create)
        else {
            return true;
        };
        let Some(record) = self.creates.iter_mut().find(|record| record.key == key) else {
            return false;
        };
        if record.prepare != Some(id) {
            return false;
        }
        record.prepared = true;
        true
    }
    pub(crate) fn submit_command(
        &mut self,
        incoming: crate::system::control::endpoint::request::Incoming,
        reference: InstanceRef,
    ) -> Result<
        JobId,
        (
            crate::system::control::unit::verdict::Fail,
            crate::system::control::endpoint::request::Incoming,
        ),
    > {
        let mut incoming = Some(incoming);
        for work in self.work.iter_mut() {
            let Work::Command(command) = work else {
                continue;
            };
            if command.reference != reference {
                continue;
            }
            let Some(request) = incoming.as_ref() else {
                continue;
            };
            if command.from != request.from { continue; }
            if matches!(
                request.wire,
                system_api::control::frame::Wire::StateInstance(_)
            ) || matches!(
                command.wire,
                system_api::control::frame::Wire::StateInstance(_)
            ) {
                continue;
            }
            if matches!(
                (&command.wire, &request.wire),
                (system_api::control::frame::Wire::EmbarkInstance(old), system_api::control::frame::Wire::EmbarkInstance(new)) if old == new
            ) {
                if command.waiters.try_reserve(1).is_err() {
                    return Err((
                        crate::system::control::unit::verdict::Fail::Full,
                        incoming.take().unwrap(),
                    ));
                }
                command.waiters.push(incoming.take().unwrap().reply);
                return Ok(command.id);
            }
            if matches!((&command.wire, &request.wire),
                (system_api::control::frame::Wire::RuinInstance(old), system_api::control::frame::Wire::RuinInstance(new))
                | (system_api::control::frame::Wire::ReleaseInstance(old), system_api::control::frame::Wire::ReleaseInstance(new)) if old == new)
            {
                if command.waiters.try_reserve(1).is_err() {
                    return Err((
                        crate::system::control::unit::verdict::Fail::Full,
                        incoming.take().unwrap(),
                    ));
                }
                command.waiters.push(incoming.take().unwrap().reply);
                return Ok(command.id);
            }
            return Err((
                crate::system::control::unit::verdict::Fail::NotReady,
                incoming.take().unwrap(),
            ));
        }
        let incoming = incoming.take().expect("unmatched incoming request");
        let Some(id) = self.work.reserve_job(272) else {
            return Err((crate::system::control::unit::verdict::Fail::Full, incoming));
        };
        self.work.push_back(Work::Command(Command {
            id,
            reference,
            wire: incoming.wire,
            from: incoming.from,
            reply: Some(incoming.reply),
            waiters: Vec::new(),
            activation_deadline: None,
        }));
        Ok(id)
    }
    pub(crate) fn poll_instance_ready(
        &mut self,
        reference: InstanceRef,
    ) -> Result<bool, crate::system::control::unit::verdict::Fail> {
        let launch = self
            .launches
            .iter_mut()
            .find(|launch| {
                launch.instance.reference == reference && launch.instance.task == reference.task
            })
            .ok_or(crate::system::control::unit::verdict::Fail::Unknown)?;
        let ready = launch
            .installation
            .delivery
            .program
            .map_or(crate::unit::Ready::OnSpawn, |program| program.control.ready);
        launch
            .installation
            .service
            .as_mut()
            .ok_or(crate::system::control::unit::verdict::Fail::NotReady)?
            .ready_for(ready)
    }
    pub(crate) fn take_commands(&mut self, limit: usize) -> Vec<Command> {
        let mut commands = Vec::new();
        let count = self.work.len();
        for _ in 0..count {
            if commands.len() >= limit {
                break;
            }
            match self.work.pop_front() {
                Some(Work::Command(command)) => commands.push(command),
                Some(work) => self.work.push_back(work),
                None => break,
            }
        }
        commands
    }
    pub(crate) fn requeue_command(&mut self, command: Command) {
        self.work.push_back(Work::Command(command));
    }
    pub(crate) fn take_local(
        &mut self,
        id: JobId,
    ) -> Option<
        Result<
            (crate::system::control::lifecycle::Action, Option<TaskId>),
            crate::system::control::unit::verdict::Fail,
        >,
    > {
        let completion = self.take(id)?;
        Some(match completion.result.outcome {
            Err(JobFailure::Control(fail)) => Err(fail),
            Ok(JobOutcome::StaticLifecycle(action, task)) => Ok((action, task)),
            _ => return None,
        })
    }
    pub(crate) fn select_lifecycle(
        &mut self,
    ) -> Option<crate::system::control::lifecycle::Tracked> {
        let rounds = self.work.len();
        match self.work.pop_matching(rounds, |work| {
            matches!(
                work, Work::Lifecycle(tracked) if !tracked.complete
            )
        })? {
            Work::Lifecycle(tracked) => Some(tracked),
            _ => None,
        }
    }
    pub(crate) fn requeue_lifecycle(
        &mut self,
        tracked: crate::system::control::lifecycle::Tracked,
    ) {
        self.work.push_back(Work::Lifecycle(tracked));
    }
    pub(crate) fn reply_completed(&mut self) {
        let ids: Vec<_> = self
            .work
            .iter()
            .filter_map(|work| match work {
                Work::Completed(completion)
                    if matches!(&completion.result.reply,
                JobReply::Lifecycle { back, waiters } if back.is_some() || !waiters.is_empty()) =>
                {
                    Some(completion.id)
                }
                _ => None,
            })
            .collect();
        for id in ids {
            let Some(completion) = self.take(id) else {
                continue;
            };
            let JobReply::Lifecycle { back, waiters } = completion.result.reply else {
                continue;
            };
            let said = match completion.result.outcome {
                Err(JobFailure::Control(fail)) => {
                    let code = match fail {
                        crate::system::control::unit::verdict::Fail::Unknown => {
                            system_api::control::Fail::Unknown
                        }
                        crate::system::control::unit::verdict::Fail::BadImage => {
                            system_api::control::Fail::BadImage
                        }
                        crate::system::control::unit::verdict::Fail::Full => {
                            system_api::control::Fail::Full
                        }
                        crate::system::control::unit::verdict::Fail::NotReady => {
                            system_api::control::Fail::NotReady
                        }
                    };
                    system_api::control::frame::said_status(
                        system_api::control::frame::fail_to_code(Some(code)),
                    )
                }
                Ok(JobOutcome::StaticLifecycle(
                    crate::system::control::lifecycle::Action::Embark { .. },
                    Some(task),
                )) => system_api::control::frame::said_task(task),
                Ok(_) => system_api::control::frame::said_status(system_api::control::frame::OK),
                Err(_) => system_api::control::frame::said_status(system_api::control::frame::FULL),
            };
            if let Some(back) = back {
                crate::system::control::endpoint::request::reply(back, said);
            }
            for waiter in waiters {
                crate::system::control::endpoint::request::reply(waiter, said);
            }
        }
    }
    pub(crate) fn retire_local_completed(&mut self) {
        let ids: Vec<_> = self.work.iter().filter_map(|work| match work {
            Work::Completed(completion) if matches!(&completion.result.reply, JobReply::Lifecycle { back: None, waiters } if waiters.is_empty()) => Some(completion.id),
            _ => None,
        }).collect();
        for id in ids {
            let _ = self.take(id);
        }
    }
    pub(crate) fn take_construct_batch(&mut self) -> Vec<(JobId, Request)> {
        let count = self.work.len();
        let mut requests = Vec::new();
        for _ in 0..count {
            match self.work.pop_front() {
                Some(Work::Construct { id, request }) => requests.push((id, request)),
                Some(work) => self.work.push_back(work),
                None => break,
            }
        }
        requests
    }
    pub(crate) fn startup(&self, task: TaskId) -> Option<system_api::loader::StartupMetadata> {
        self.launches
            .iter()
            .find(|launch| launch.instance.task == task)
            .map(|launch| launch.installation.startup)
    }
    pub(crate) fn admit_create(
        &mut self,
        key: CreateKey,
        request: &system_api::control::construction::Request,
    ) -> Result<CreateAdmission, Fail> {
        if self
            .closed_create_scopes
            .iter()
            .any(|scope| scope.entry == key.entry && scope.caller == key.caller)
        {
            return Err(Fail::Conflict);
        }
        if let Some(record) = self.creates.iter().find(|record| record.key == key) {
            if record.cancelled {
                return Ok(CreateAdmission::Cancelled);
            }
            let same = record.owner == request.owner
                && record.subject == Some(request.subject)
                && record
                    .input
                    .as_ref()
                    .zip(request.input.0.as_ref())
                    .is_some_and(|(old, new)| same_input(old, new));
            return if same {
                Ok(CreateAdmission::Duplicate)
            } else {
                Err(Fail::Conflict)
            };
        }
        self.prune_creates(key);
        let watermark = self
            .create_watermarks
            .iter()
            .position(|mark| mark.entry == key.entry && mark.caller == key.caller);
        if watermark.is_some_and(|at| key.id.value <= self.create_watermarks[at].highest) {
            return Err(Fail::Conflict);
        }
        if self.creates.len() >= 256 || self.creates.try_reserve(1).is_err() {
            return Err(Fail::Full);
        }
        if watermark.is_none() {
            if self.create_watermarks.len() >= 256 || self.create_watermarks.try_reserve(1).is_err()
            {
                return Err(Fail::Full);
            }
        }
        let input = retained_input(request.input.0.as_ref().ok_or(Fail::Bad)?)?;
        if let Some(at) = watermark {
            self.create_watermarks[at].highest = key.id.value;
        } else {
            self.create_watermarks.push(CreateWatermark {
                entry: key.entry,
                caller: key.caller,
                highest: key.id.value,
            });
        }
        self.creates.push(CreateRecord {
            key,
            construct: None,
            prepare: None,
            prepared: false,
            owner: request.owner,
            subject: Some(request.subject),
            input: Some(input),
            fingerprint: None,
            said: Said {
                status: wire::OK,
                state: PENDING,
                team: 0,
                task: TaskId::new(0),
                startup: system_api::loader::StartupMetadata {
                    resolve: env::PieToken::NONE,
                },
            },
            cancelled: false,
        });
        Ok(CreateAdmission::New)
    }
    pub(crate) fn create_reply(&self, key: CreateKey) -> Said {
        self.creates
            .iter()
            .find(|record| record.key == key)
            .map(|record| record.said)
            .unwrap_or_else(|| {
                if self.create_watermarks.iter().any(|mark| {
                    mark.entry == key.entry
                        && mark.caller == key.caller
                        && key.id.value <= mark.highest
                }) {
                    Said::from_result(Err(Fail::Conflict))
                } else {
                    Said {
                        status: wire::OK,
                        state: UNSEEN,
                        team: 0,
                        task: TaskId::new(0),
                        startup: system_api::loader::StartupMetadata {
                            resolve: env::PieToken::NONE,
                        },
                    }
                }
            })
    }
    pub(crate) fn retire_create_scope(
        &mut self,
        entry: env::PieToken,
        caller: TaskId,
    ) -> Result<(), Fail> {
        let pending = self.creates.iter().any(|record| {
            record.key.entry == entry && record.key.caller == caller && record.said.state == PENDING
        });
        if pending
            && !self
                .closed_create_scopes
                .iter()
                .any(|scope| scope.entry == entry && scope.caller == caller)
        {
            if self.closed_create_scopes.len() >= 256
                || self.closed_create_scopes.try_reserve(1).is_err()
            {
                return Err(Fail::Full);
            }
            self.closed_create_scopes
                .push(ClosedCreateScope { entry, caller });
        }
        self.sweep_closed_create_scopes();
        if self
            .closed_create_scopes
            .iter()
            .any(|scope| scope.entry == entry && scope.caller == caller)
        {
            Err(Fail::NotReady)
        } else {
            Ok(())
        }
    }
    fn sweep_closed_create_scopes(&mut self) {
        let mut at = 0;
        while at < self.closed_create_scopes.len() {
            let scope = &self.closed_create_scopes[at];
            let pending = self.creates.iter().any(|record| {
                record.key.entry == scope.entry
                    && record.key.caller == scope.caller
                    && record.said.state == PENDING
            });
            if pending {
                at += 1;
                continue;
            }
            let scope = self.closed_create_scopes.remove(at);
            self.creates.retain(|record| {
                record.key.entry != scope.entry || record.key.caller != scope.caller
            });
            self.create_watermarks
                .retain(|mark| mark.entry != scope.entry || mark.caller != scope.caller);
        }
    }
    fn prune_creates(&mut self, key: CreateKey) {
        let cutoff = key.id.value.saturating_sub(64);
        self.creates.retain(|record| {
            if record.key.entry != key.entry
                || record.key.caller != key.caller
                || record.key.id.value > cutoff
            {
                return true;
            }
            record.said.state == PENDING
        });
    }
    pub(crate) fn cancel_create(&mut self, key: CreateKey, control: &mut Control) -> Said {
        let Some(at) = self.creates.iter().position(|record| record.key == key) else {
            self.prune_creates(key);
            let watermark = self
                .create_watermarks
                .iter()
                .position(|mark| mark.entry == key.entry && mark.caller == key.caller);
            if watermark.is_some_and(|i| key.id.value <= self.create_watermarks[i].highest) {
                return Said::from_result(Err(Fail::Conflict));
            }
            if self.creates.len() >= 256 || self.creates.try_reserve(1).is_err() {
                return Said::from_result(Err(Fail::Full));
            }
            if watermark.is_none()
                && (self.create_watermarks.len() >= 256
                    || self.create_watermarks.try_reserve(1).is_err())
            {
                return Said::from_result(Err(Fail::Full));
            }
            if let Some(at) = watermark {
                self.create_watermarks[at].highest = key.id.value;
            } else {
                self.create_watermarks.push(CreateWatermark {
                    entry: key.entry,
                    caller: key.caller,
                    highest: key.id.value,
                });
            }
            self.creates.push(CreateRecord {
                key,
                construct: None,
                prepare: None,
                prepared: false,
                owner: key.caller,
                subject: None,
                input: None,
                fingerprint: None,
                said: Said {
                    status: wire::OK,
                    state: CANCELLED,
                    team: 0,
                    task: TaskId::new(0),
                    startup: system_api::loader::StartupMetadata {
                        resolve: env::PieToken::NONE,
                    },
                },
                cancelled: true,
            });
            return self.creates.last().unwrap().said;
        };
        self.creates[at].cancelled = true;
        if let Some(launch) = self
            .launches
            .iter()
            .find(|launch| launch.instance.create == Some(key))
        {
            control.stop_instance(launch.instance.task);
            self.creates[at].said.state = PENDING;
            return self.creates[at].said;
        }
        let construct = self.creates[at].construct;
        if let Some(at) = self.work.iter().position(|work| {
            matches!(work,
            Work::Construct { id, request } if Some(*id) == construct && request.create == Some(key))
        }) {
            let Work::Construct { id, request } = self.work.remove(at).expect("work index") else {
                unreachable!()
            };
            request.image.release();
            consume_construct(
                self,
                ConstructFinish {
                    id,
                    result: Ok(JobOutcome::Cancelled),
                    back: request.delivery.back,
                    key: Some(key),
                },
            );
            return self.creates[at].said;
        }
        self.creates[at].said = cancelled_said();
        self.creates[at].said
    }
    pub(crate) fn instance_ref(&self, task: TaskId) -> Option<InstanceRef> {
        self.launches
            .iter()
            .find(|launch| launch.instance.task == task)
            .map(|launch| launch.instance.reference)
    }
    pub(crate) fn active_hook_reference(&self) -> Option<InstanceRef> {
        self.hook_instance
    }
    pub(crate) fn active_hook_task(&self) -> Option<TaskId> {
        self.hook_instance.map(|instance| instance.task)
    }
    pub(crate) fn select_hook(&mut self, selection: HookSelection) {
        self.hook_instance = Some(selection.reference);
        self.hook_job = Some(selection.job);
        self.hook_next = selection.next;
    }
    pub(crate) fn active_hook_job(&self) -> Option<JobId> {
        self.hook_job
    }
    pub(crate) fn advance_hook(&mut self, next: usize) {
        self.hook_next = next;
    }
    pub(crate) fn next_hook(&self) -> usize {
        self.hook_next
    }
    pub(crate) fn clear_hook(&mut self) {
        self.hook_instance = None;
        self.hook_job = None;
    }
    pub(crate) fn reset_hook_scan(&mut self) {
        self.hook_instance = None;
        self.hook_job = None;
        self.hook_next = 0;
    }
}
pub(crate) enum CreateAdmission {
    New,
    Duplicate,
    Cancelled,
}

pub(crate) fn install(
    resources: &mut ::schedule::Resources<'static>,
) -> Result<(), ::schedule::resource::AccessError> {
    let catalog = *resources.read::<crate::boot::Catalog<'static>>()?;
    resources.insert(Jobs {
        catalog: Some(catalog),
        ..Jobs::default()
    })?;
    Ok(())
}

pub(crate) fn frame() -> Result<Plan<crate::system::app::Fault>, BuildError> {
    let mut frame = Schedule::sequence();
    frame.system("settle", settle)?;
    frame.system("construct", construct)?;
    frame.build()
}
fn settle(
    settle: Res<crate::system::app::policy::Settle>,
    mut jobs: ResMut<Jobs>,
) -> Result<Progress, crate::system::app::Fault> {
    if settle.flow.settling || settle.closing() {
        let requests = jobs.take_construct_batch();
        for (id, request) in requests {
            request.image.release();
            let result = if request.create.is_some_and(|key| {
                jobs.creates
                    .iter()
                    .any(|record| record.key == key && record.cancelled)
            }) {
                Ok(JobOutcome::Cancelled)
            } else {
                Err(JobFailure::Construction(Fail::NotReady))
            };
            consume_construct(
                &mut jobs,
                ConstructFinish {
                    id,
                    result,
                    back: request.delivery.back,
                    key: request.create,
                },
            );
        }
    }
    Ok(Progress::Done)
}
fn construct(
    mut loader: ResMut<crate::system::loader::Loader>,
    mut control: ResMut<Control>,
    mut jobs: ResMut<Jobs>,
) -> Result<Progress, crate::system::app::Fault> {
    let requests = jobs.take_construct_batch();
    let mut context = Construct {
        loader: &mut loader,
        control: &mut control,
    };
    for (job_id, mut request) in requests {
        let bytes = match request.image.snapshot(jobs.catalog) {
            Ok(bytes) => bytes,
            Err(fail) => {
                request.image.release();
                consume_construct(
                    &mut jobs,
                    ConstructFinish {
                        id: job_id,
                        result: Err(JobFailure::Construction(fail.into())),
                        back: request.delivery.back,
                        key: request.create,
                    },
                );
                continue;
            }
        };
        request.image.release();
        request.delivery.program = request.image.program();
        if let Some(key) = request.create {
            let fingerprint = sha256(&bytes);
            let existing = jobs
                .creates
                .iter()
                .find(|record| record.key == key)
                .map(|record| (record.cancelled, record.fingerprint, record.said));
            if let Some((cancelled, Some(previous), said)) = existing {
                let back = request.delivery.back.take();
                let result = if cancelled {
                    Ok(JobOutcome::Cancelled)
                } else if previous != fingerprint {
                    Err(JobFailure::Construction(Fail::Conflict))
                } else {
                    Ok(JobOutcome::CreateReplay(said))
                };
                consume_construct(
                    &mut jobs,
                    ConstructFinish {
                        id: job_id,
                        result,
                        back,
                        key: Some(key),
                    },
                );
                continue;
            }
            if let Some(record) = jobs.creates.iter_mut().find(|record| record.key == key) {
                if record.cancelled {
                    let back = request.delivery.back.take();
                    consume_construct(
                        &mut jobs,
                        ConstructFinish {
                            id: job_id,
                            result: Ok(JobOutcome::Cancelled),
                            back,
                            key: Some(key),
                        },
                    );
                    continue;
                }
                record.fingerprint = Some(fingerprint);
            }
        }
        if jobs.launches.try_reserve(1).is_err() {
            consume_construct(
                &mut jobs,
                ConstructFinish {
                    id: job_id,
                    result: Err(JobFailure::Construction(Fail::Full)),
                    back: request.delivery.back,
                    key: request.create,
                },
            );
            continue;
        }
        let create = request.create;
        match construct_one(
            &mut context,
            ConstructRequest {
                request,
                bytes,
                job_id,
            },
        ) {
            Ok(mut launch) => {
                launch.instance.create = create;
                let reference = launch.instance.reference;
                if let Some(done) = consume_construct(
                    &mut jobs,
                    ConstructFinish {
                        id: job_id,
                        result: Ok(JobOutcome::Construct(reference)),
                        back: None,
                        key: create,
                    },
                ) {
                    if done == reference {
                        jobs.launches.push(launch);
                    }
                }
            }
            Err((fail, delivery)) => {
                consume_construct(
                    &mut jobs,
                    ConstructFinish {
                        id: job_id,
                        result: Err(JobFailure::Construction(fail)),
                        back: delivery.back,
                        key: create,
                    },
                );
            }
        }
    }
    Ok(Progress::Done)
}
struct Construct<'a> {
    loader: &'a mut crate::system::loader::Loader,
    control: &'a mut Control,
}
struct ConstructRequest {
    request: Request,
    bytes: Vec<u8>,
    job_id: JobId,
}
fn construct_one(
    context: &mut Construct<'_>,
    input: ConstructRequest,
) -> Result<Launch, (Fail, Delivery)> {
    let ConstructRequest {
        request,
        bytes,
        job_id,
    } = input;
    let Request {
        image, delivery, ..
    } = request;
    let (count, values, stack) = image.parameters();
    let result = context.control.create_instance(
        crate::system::control::instance::create::Admission {
            owner: delivery.owner,
            job: job_id,
        },
        || {
            let count = count as usize;
            if count > system_api::loader::MAX_ARGS {
                return Err(system_api::loader::Fail::Bad.into());
            }
            let mut args = [0; system_api::loader::MAX_ARGS];
            for (to, from) in args.iter_mut().zip(&values[..count]) {
                *to = *from as usize;
            }
            context
                .loader
                .construct(
                    Image {
                        bytes: &bytes,
                        kind: delivery
                            .program
                            .map_or(env::ProgramKind::User, |program| program.loader.space),
                    },
                    Spawn {
                        sire: delivery.owner,
                        args: &args[..count],
                        stack: stack as usize,
                    },
                )
                .map_err(Into::into)
        },
    );
    match result {
        Ok(built) => {
            let mut service = crate::system::control::unit::Service::new(built.task);
            if let Some(program) = delivery.program {
                if service.connect(program).is_err() {
                    context.control.stop_instance(built.task);
                    return Err((Fail::NotReady, delivery));
                }
            }
            let retirement = delivery
                .program
                .map(|program| {
                    program
                        .control
                        .retirement
                        .iter()
                        .map(|declaration| {
                            retirement::Participant::new(*declaration, built.task)
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(Launch {
                instance: Launched {
                    reference: InstanceRef {
                        job: job_id,
                        task: built.task,
                    },
                    task: built.task,
                    create: None,
                },
                installation: InstanceInstall {
                    delivery,
                    service: Some(service),
                    identity: None,
                    startup: system_api::loader::StartupMetadata {
                        resolve: env::PieToken::NONE,
                    },
                    bootstrap: None,
                    retirement,
                },
            })
        }
        Err(fail) => Err((fail, delivery)),
    }
}

fn consume_construct(jobs: &mut Jobs, finish: ConstructFinish) -> Option<InstanceRef> {
    let id = finish.id;
    let key = finish.key;
    jobs.complete_construct(finish);
    let completion = jobs.take(id)?;
    let reference = match completion.result.outcome {
        Ok(JobOutcome::Construct(reference)) => Some(reference),
        Ok(JobOutcome::CreateReplay(said)) => {
            if let JobReply::Construction(Some(back)) = completion.result.reply {
                let _ = back.send(said);
            }
            None
        }
        Ok(JobOutcome::Cancelled) => {
            if let Some(key) = key {
                if let Some(record) = jobs.creates.iter_mut().find(|record| record.key == key) {
                    record.said = cancelled_said();
                }
            }
            if let JobReply::Construction(Some(back)) = completion.result.reply {
                let _ = back.send(cancelled_said());
            }
            None
        }
        Err(JobFailure::Construction(fail)) => {
            if let Some(key) = key {
                if let Some(record) = jobs.creates.iter_mut().find(|record| record.key == key) {
                    if !record.cancelled && record.said.state == PENDING {
                        record.said = Said::from_result(Err(fail));
                    }
                }
            }
            if let JobReply::Construction(Some(back)) = completion.result.reply {
                crate::system::launch::reply(back, Err(fail));
            }
            None
        }
        _ => None,
    };
    jobs.sweep_closed_create_scopes();
    reference
}
fn cancelled_said() -> Said {
    Said {
        status: wire::OK,
        state: CANCELLED,
        team: 0,
        task: TaskId::new(0),
        startup: system_api::loader::StartupMetadata {
            resolve: env::PieToken::NONE,
        },
    }
}
pub(crate) fn completed(
    mut jobs: ResMut<Jobs>,
    control: ResMut<Control>,
) -> Result<Progress, crate::system::app::Fault> {
    let mut index = 0;
    while index < jobs.launches.len() {
        let task = jobs.launches[index].instance.task;
        if control.instance(task).is_none_or(|item| {
            item.state == crate::system::control::unit::table::State::Dead && item.team().is_none()
        }) {
            let launch = jobs.launches.remove(index);
            if let Some(key) = launch.instance.create {
                if let Some(record) = jobs.creates.iter_mut().find(|record| record.key == key) {
                    if record.cancelled {
                        record.said = Said {
                            status: wire::OK,
                            state: CANCELLED,
                            team: 0,
                            task: TaskId::new(0),
                            startup: system_api::loader::StartupMetadata {
                                resolve: env::PieToken::NONE,
                            },
                        };
                    }
                }
            }
            continue;
        }
        if jobs.launches[index].installation.delivery.back.is_none() {
            index += 1;
            continue;
        }
        let Some(result) = control.instance_result(task)? else {
            index += 1;
            continue;
        };
        if result.is_ok()
            && jobs.launches[index].instance.create.is_some_and(|key| {
                jobs.creates
                    .iter()
                    .find(|record| record.key == key)
                    .is_some_and(|record| record.prepare.is_some() && !record.prepared)
            })
        {
            index += 1;
            continue;
        }
        let mut said = Said::from_result(result);
        if result.is_ok() {
            said.startup = jobs.launches[index].installation.startup;
        }
        if let Some(key) = jobs.launches[index].instance.create {
            if let Some(record) = jobs.creates.iter_mut().find(|record| record.key == key) {
                if !record.cancelled {
                    record.said = said;
                }
            }
        }
        // A lost reply does not invalidate an instance that remains queryable.
        if let Some(back) = jobs.launches[index].installation.delivery.back.take() {
            let _ = back.send(said);
        }
        index += 1;
    }
    jobs.sweep_closed_create_scopes();
    Ok(Progress::Done)
}

#[path = "assembly/hooks.rs"]
pub(crate) mod hooks;
#[path = "assembly/static.rs"]
pub(crate) mod static_plan;

#[path = "assembly/namespace.rs"]
pub(crate) mod namespace;
#[path = "assembly/retirement.rs"]
pub(crate) mod retirement;
#[path = "assembly/supply.rs"]
pub(crate) mod supply;
