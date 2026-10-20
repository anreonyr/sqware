use super::super::{
    core::{Endpoint, Job, JobId, JobStatus, PauseReason, Plan},
    native::objects::Objects,
};
use super::{
    manifest::Manifest,
    ports::{Buffer, Pump},
    rpc::{Loading, WAIT},
    waiting::Interests,
};
use alloc::{collections::VecDeque, format, string::String, vec::Vec};
use env::{PieToken, TaskId};
use ipc::rpc::request::Pending;
use lisp::RootValue;
use pipe_client::{Client, Creating, Direction, Pipe};
use shell_api::{Binding, Catalogue, Launch};
use system_api::{control::construction, identity::Subject, loader};
const MEMORY: usize = 4 * 1024 * 1024;
struct Member {
    built: Option<loader::Built>,
    bindings: Vec<Binding>,
    manifest: Option<Manifest>,
    ran: bool,
    dead: bool,
    reason: Option<usize>,
    stopping: bool,
}
enum Work {
    Load {
        index: usize,
        pending: Loading,
    },
    Create {
        pending: Creating,
    },
    Bind {
        direction: Direction,
        pending: Pending<pipe_api::Call>,
    },
    Release {
        pending: Pending<pipe_api::Call>,
    },
}
pub struct RuntimeJob {
    pub model: Job,
    pub error: Option<String>,
    plan: Plan,
    _root: RootValue,
    members: Vec<Member>,
    pipes: Vec<Pipe>,
    pumps: Vec<Pump>,
    work: Option<Work>,
    build_index: usize,
    link_index: usize,
    manifest_index: usize,
    binding: Option<Direction>,
    pump_index: usize,
    pub input: VecDeque<u8>,
    pub output: VecDeque<u8>,
    pub eof: bool,
    released: bool,
}
pub struct Controller {
    pub catalogue: Catalogue,
    pipe: Client,
    construction: PieToken,
    loader: PieToken,
    subject: Subject,
    identity_authority: TaskId,
    pub foreground: Option<JobId>,
    jobs: Vec<RuntimeJob>,
    next: u64,
    cursor: usize,
    output_cursor: usize,
    next_create: u64,
}
impl Controller {
    pub fn new(
        catalogue: Catalogue,
        pipe: Client,
        construction: PieToken,
        loader: PieToken,
        subject: Subject,
        identity_authority: TaskId,
    ) -> Self {
        Self {
            catalogue,
            pipe,
            construction,
            loader,
            subject,
            identity_authority,
            foreground: None,
            jobs: Vec::new(),
            next: 0,
            cursor: 0,
            output_cursor: 0,
            next_create: 0,
        }
    }
    pub fn job(&self, id: JobId) -> Option<&RuntimeJob> {
        self.jobs.iter().find(|job| job.model.id == id)
    }
    fn job_mut(&mut self, id: JobId) -> Result<&mut RuntimeJob, &'static str> {
        self.jobs
            .iter_mut()
            .find(|job| job.model.id == id)
            .ok_or("unknown job")
    }
    pub fn jobs(&self) -> impl Iterator<Item = JobId> + '_ {
        self.jobs.iter().map(|job| job.model.id)
    }
    pub fn prepare(
        &mut self,
        plan: Plan,
        root: RootValue,
        objects: &mut Objects,
    ) -> Result<JobId, &'static str> {
        if plan.weight()
            > MEMORY.saturating_sub(self.jobs.iter().map(|job| job.plan.weight()).sum())
        {
            return Err("job plan memory limit exceeded");
        }
        if self.jobs.len() >= 128 {
            return Err("job limit exceeded");
        }
        for link in &plan.links {
            for end in [&link.source, &link.sink] {
                if let Endpoint::Port { id } = end {
                    let port = objects.port(*id).ok_or("missing port")?;
                    if port.closed || port.bound.is_some() {
                        return Err("port is closed or already bound");
                    }
                }
            }
        }
        self.jobs.try_reserve(1).map_err(|_| "job allocation")?;
        self.next = self
            .next
            .checked_add(1)
            .ok_or("job identifiers exhausted")?;
        let id = JobId(self.next);
        let model = Job::prepare(id, plan.commands.len());
        let members = plan
            .commands
            .iter()
            .map(|_| Member {
                built: None,
                bindings: Vec::new(),
                manifest: None,
                ran: false,
                dead: false,
                reason: None,
                stopping: false,
            })
            .collect();
        for link in &plan.links {
            for end in [&link.source, &link.sink] {
                if let Endpoint::Port { id: key } = end {
                    objects.port_mut(*key).ok_or("missing port")?.bound = Some(id);
                }
            }
        }
        self.jobs.push(RuntimeJob {
            model,
            error: None,
            plan,
            _root: root,
            members,
            pipes: Vec::new(),
            pumps: Vec::new(),
            work: None,
            build_index: 0,
            link_index: 0,
            manifest_index: 0,
            binding: None,
            pump_index: 0,
            input: VecDeque::new(),
            output: VecDeque::new(),
            eof: false,
            released: false,
        });
        Ok(id)
    }
    pub fn start(&mut self, id: JobId) -> Result<(), &'static str> {
        self.job_mut(id)?
            .model
            .start()
            .map_err(|_| "job is not prepared")
    }
    pub fn pause(&mut self, id: JobId, reason: PauseReason) -> Result<(), &'static str> {
        self.job_mut(id)?
            .model
            .pause(reason)
            .map_err(|_| "job cannot pause")
    }
    pub fn resume(&mut self, id: JobId) -> Result<(), &'static str> {
        self.job_mut(id)?
            .model
            .resume()
            .map_err(|_| "job is not paused")
    }
    pub fn cancel(&mut self, id: JobId) -> Result<(), &'static str> {
        let job = self.job_mut(id)?;
        if job.model.status.terminal() {
            return Ok(());
        }
        if let Some(Work::Load { index, pending }) = &mut job.work {
            pending.cancel().map_err(|_| "construction cancel")?;
            if let Some(built) = pending.built {
                job.members[*index].built = Some(built);
            }
        } else {
            job.work = None;
        }
        job.model.cancel();
        Ok(())
    }
    pub fn cancel_all(&mut self) {
        let ids: Vec<_> = self.jobs().collect();
        for id in ids {
            let _ = self.cancel(id);
        }
    }
    pub fn settled(&self) -> bool {
        self.jobs.iter().all(|job| job.model.status.terminal())
    }
    pub fn foreground(&mut self, id: JobId) -> Result<(), &'static str> {
        if self.job(id).is_none_or(|job| job.model.status.terminal()) {
            return Err("job is not live");
        }
        for job in &mut self.jobs {
            job.model.foreground = job.model.id == id;
        }
        self.foreground = Some(id);
        Ok(())
    }
    pub fn background(&mut self, id: JobId) -> Result<(), &'static str> {
        self.job_mut(id)?.model.foreground = false;
        if self.foreground == Some(id) {
            self.foreground = None;
        }
        Ok(())
    }
    pub fn input(&mut self, bytes: &[u8], eof: bool) {
        if let Some(id) = self.foreground {
            if let Ok(job) = self.job_mut(id) {
                if job.input.len() + bytes.len() <= 64 * 1024 {
                    job.input.extend(bytes);
                }
                job.eof |= eof;
            }
        }
    }
    pub fn output(&self) -> Option<(JobId, &[u8])> {
        for next in 0..self.jobs.len() {
            let job = &self.jobs[(self.output_cursor + next) % self.jobs.len()];
            let (first, second) = job.output.as_slices();
            let bytes = if first.is_empty() { second } else { first };
            if !bytes.is_empty() {
                return Some((job.model.id, &bytes[..bytes.len().min(256)]));
            }
        }
        None
    }
    pub fn consume_output(&mut self, id: JobId, n: usize) {
        if let Some(index) = self.jobs.iter().position(|job| job.model.id == id) {
            for _ in 0..n {
                self.jobs[index].output.pop_front();
            }
            self.output_cursor = index.wrapping_add(1);
        }
    }
    pub fn reclaim(&mut self, objects: &Objects) {
        self.jobs
            .retain(|job| !job.model.status.terminal() || objects.has_job(job.model.id));
    }
    pub fn interests(&self, objects: &Objects, interests: &mut Interests) {
        for job in &self.jobs {
            if job.model.status.terminal() {
                continue;
            }
            if let Some(work) = &job.work {
                let (source, budget) = match work {
                    Work::Load { pending, .. } => (pending.source(), pending.remaining()),
                    Work::Create { pending } => (Some(pending.source()), pending.remaining()),
                    Work::Bind { pending, .. } | Work::Release { pending } => {
                        (Some(pending.source()), pending.remaining())
                    }
                };
                if let Some(source) = source {
                    interests.source(source);
                } else {
                    interests.runnable = true;
                }
                interests.budget(budget);
                continue;
            }
            match job.model.status {
                JobStatus::Preparing => interests.runnable = true,
                JobStatus::Starting
                | JobStatus::Pausing
                | JobStatus::Resuming
                | JobStatus::Cancelling => {
                    interests.budget(env::Wait::AtMost(1));
                }
                _ => {}
            }
            for member in &job.members {
                if let Some(built) = member.built.filter(|_| !member.dead) {
                    interests.source(env::ToleEventSource::Join {
                        target: built.team.into(),
                    });
                }
            }
            let all_dead = job.model.results.iter().all(Option::is_some);
            let drained = job.pumps.iter().all(|pump| pump.done) && job.output.is_empty();
            if all_dead && (drained || job.model.status == JobStatus::Cancelling) {
                interests.runnable = true;
            }
            if !matches!(
                job.model.status,
                JobStatus::Starting | JobStatus::Running | JobStatus::Resuming
            ) {
                continue;
            }
            for pump in &job.pumps {
                if pump.done {
                    continue;
                }
                let Some(local) = objects.port(pump.local) else {
                    interests.runnable = true;
                    continue;
                };
                if local.closed {
                    interests.runnable = true;
                    continue;
                }
                if pump.source() && pump.pending.is_empty() {
                    if matches!(local.buffer, Buffer::Terminal(Direction::Read)) {
                        if !job.model.foreground {
                            interests.source(env::ToleEventSource::Pole {
                                token: pump.port.token(),
                                bit: pipe_api::DEMAND_BIT,
                            });
                            continue;
                        }
                        if job.input.is_empty() && !job.eof {
                            continue;
                        }
                    }
                }
                match pump.port.prepare_wait() {
                    Ok(false) => interests.source(pump.port.source()),
                    _ => interests.runnable = true,
                }
            }
        }
    }
    pub fn advance(&mut self, objects: &mut Objects) {
        if self.jobs.is_empty() {
            return;
        }
        let index = self.cursor % self.jobs.len();
        self.cursor = self.cursor.wrapping_add(1);
        let output_used: usize = self.jobs.iter().map(|job| job.output.len()).sum();
        let available = MEMORY
            .saturating_sub(objects.used())
            .saturating_sub(output_used);
        let construction = self.construction;
        let loader = self.loader;
        let subject = self.subject;
        let identity_authority = self.identity_authority;
        let next_create = &mut self.next_create;
        let job = &mut self.jobs[index];
        if let Err(error) = advance(
            job,
            objects,
            &self.catalogue,
            &self.pipe,
            construction,
            loader,
            subject,
            identity_authority,
            next_create,
            available,
        ) {
            if let Some(Work::Load { index, pending }) = &job.work {
                if let Some(built) = pending.built {
                    job.members[*index].built = Some(built);
                }
            }
            job.work = None;
            job.error.get_or_insert(error);
            job.model.fail();
        }
        if self.foreground == Some(job.model.id) && !job.model.foreground {
            self.foreground = None;
        }
    }
}
fn target(job: &RuntimeJob, end: &Endpoint) -> Result<TaskId, String> {
    match end {
        Endpoint::Port { .. } => Ok(env::unit::self_id()),
        Endpoint::Command { id, .. } => {
            let index = job
                .plan
                .commands
                .iter()
                .position(|command| command.id == *id)
                .ok_or("unknown command")?;
            job.members[index]
                .built
                .map(|built| built.task)
                .ok_or("command is not built".into())
        }
    }
}
fn advance(
    job: &mut RuntimeJob,
    objects: &mut Objects,
    catalogue: &Catalogue,
    pipe: &Client,
    construction_entry: PieToken,
    loader_entry: PieToken,
    subject: Subject,
    identity_authority: TaskId,
    next_create: &mut u64,
    available: usize,
) -> Result<(), String> {
    if job.model.status.terminal() {
        if let Some(Work::Load { pending, .. }) = &mut job.work {
            let _ = pending.poll();
            if pending.done() {
                job.work = None;
            }
        }
        return Ok(());
    }
    if let Some(mut work) = job.work.take() {
        let pending = match &mut work {
            Work::Load { index, pending } => {
                let result = pending.poll();
                if let Some(built) = pending.built {
                    job.members[*index].built = Some(built);
                }
                match result? {
                    None => true,
                    Some(built) => {
                        job.members[*index].built = Some(built);
                        job.build_index = *index + 1;
                        false
                    }
                }
            }
            Work::Create { pending } => {
                match pending.poll().map_err(|e| format!("pipe create: {e:?}"))? {
                    None => true,
                    Some(created) => {
                        job.pipes.push(created);
                        job.binding = Some(Direction::Write);
                        false
                    }
                }
            }
            Work::Bind { direction, pending } => {
                match pending.poll().map_err(|e| format!("pipe bind: {e:?}"))? {
                    None => true,
                    Some(reply) => {
                        if reply.status != 0 {
                            return Err(format!("pipe bind rejected: {}", reply.status));
                        }
                        let root = &job.pipes[job.link_index];
                        if reply.id != root.id || reply.capacity != root.capacity as u64 {
                            return Err("pipe binding mismatch".into());
                        }
                        let endpoint = pipe_api::Endpoint {
                            id: root.id,
                            capacity: root.capacity,
                            seed: reply.seed,
                            direction: *direction,
                        };
                        let link = &job.plan.links[job.link_index];
                        let end = match direction {
                            Direction::Write => &link.source,
                            Direction::Read => &link.sink,
                        };
                        match end {
                            Endpoint::Command { id, port } => {
                                let index = job
                                    .plan
                                    .commands
                                    .iter()
                                    .position(|command| command.id == *id)
                                    .ok_or("missing bound command")?;
                                job.members[index].bindings.push(Binding {
                                    name: port.clone(),
                                    endpoint,
                                });
                            }
                            Endpoint::Port { id } => {
                                let local = objects.port_mut(*id).ok_or("missing external port")?;
                                local.bound = Some(job.model.id);
                                let port = pipe
                                    .import(endpoint, WAIT)
                                    .map_err(|e| format!("pipe mapping: {e:?}"))?;
                                job.pumps.push(Pump {
                                    local: *id,
                                    port,
                                    done: false,
                                    pending: Vec::new(),
                                    at: 0,
                                });
                            }
                        }
                        if *direction == Direction::Write {
                            job.binding = Some(Direction::Read);
                        } else {
                            job.link_index += 1;
                            job.binding = None;
                        }
                        false
                    }
                }
            }
            Work::Release { pending } => {
                match pending.poll().map_err(|e| format!("pipe release: {e:?}"))? {
                    None => true,
                    Some(reply) => {
                        if reply.status != 0 && reply.status != pipe_api::Fail::Dead.code() {
                            return Err("pipe release rejected".into());
                        }
                        job.pipes.pop();
                        false
                    }
                }
            }
        };
        if pending {
            job.work = Some(work);
        }
        return Ok(());
    }
    if job.model.status == JobStatus::Preparing {
        if job.build_index < job.members.len() {
            let index = job.build_index;
            let command = &job.plan.commands[index];
            let image = catalogue
                .images
                .iter()
                .find(|image| image.name == command.image)
                .ok_or("image unavailable")?;
            job.work = Some(Work::Load {
                index,
                pending: {
                    *next_create = next_create
                        .checked_add(1)
                        .ok_or("create identifiers exhausted")?;
                    Loading::begin(
                        construction_entry,
                        loader_entry,
                        image,
                        env::unit::self_id(),
                        subject,
                        identity_authority,
                        construction::CreateId {
                            value: *next_create,
                        },
                    )?
                },
            });
            return Ok(());
        }
        if job.link_index < job.plan.links.len() {
            if let Some(direction) = job.binding {
                let link = &job.plan.links[job.link_index];
                let end = match direction {
                    Direction::Write => &link.source,
                    Direction::Read => &link.sink,
                };
                job.work = Some(Work::Bind {
                    direction,
                    pending: pipe
                        .begin_bind(
                            &job.pipes[job.link_index],
                            direction,
                            target(job, end)?,
                            WAIT,
                        )
                        .map_err(|e| format!("pipe bind: {e:?}"))?,
                });
            } else {
                job.work = Some(Work::Create {
                    pending: pipe
                        .begin_create(pipe_api::DEFAULT_CAPACITY, WAIT)
                        .map_err(|e| format!("pipe create: {e:?}"))?,
                });
            }
            return Ok(());
        }
        if job.manifest_index < job.members.len() {
            let index = job.manifest_index;
            let member = &mut job.members[index];
            let launch = Launch {
                args: job.plan.commands[index].args.clone(),
                ports: core::mem::take(&mut member.bindings),
            };
            member.manifest = Some(
                Manifest::install(
                    &launch,
                    member.built.ok_or("missing prepared instance")?.task,
                )
                .map_err(String::from)?,
            );
            job.manifest_index += 1;
            return Ok(());
        }
        job.model.prepared().map_err(|_| "invalid prepare state")?;
        return Ok(());
    }
    for (index, member) in job.members.iter_mut().enumerate() {
        let Some(built) = member.built.filter(|_| !member.dead) else {
            continue;
        };
        let mut drained = false;
        for _ in 0..64 {
            match env::unit::join(built.team.into(), env::Wait::POLL, true)
                .map_err(|e| format!("job result: {e:?}"))?
            {
                env::JoinReply::Reaped(exit) => {
                    if exit.task == built.task {
                        if exit.reason != 0 {
                            member.reason = Some(exit.reason);
                        } else {
                            member.reason.get_or_insert(0);
                        }
                    } else if exit.cause == env::ExitCause::Fault
                        || exit.cause == env::ExitCause::Reap && exit.reason != 0
                    {
                        if member.reason.unwrap_or(0) == 0 {
                            member.reason = Some(exit.reason);
                        }
                    } else {
                        continue;
                    }
                    if !member.stopping {
                        env::unit::slay_team(built.team).map_err(|e| format!("job stop: {e:?}"))?;
                        member.stopping = true;
                    }
                }
                _ => {
                    drained = true;
                    break;
                }
            }
        }
        if !drained {
            continue;
        }
        if matches!(
            env::unit::join(built.team.into(), env::Wait::POLL, false),
            Ok(env::JoinReply::Complete)
        ) {
            match env::unit::oust(built.team) {
                Ok(()) => {
                    member.dead = true;
                    job.model
                        .complete(index, member.reason.unwrap_or(0))
                        .map_err(|_| "missing result")?;
                }
                Err(error) if error.source == env::UnitFail::Busy => {}
                Err(error) => return Err(format!("job reclaim: {error:?}")),
            }
        }
    }
    if matches!(
        job.model.status,
        JobStatus::Starting | JobStatus::Running | JobStatus::Resuming
    ) && !job.pumps.is_empty()
    {
        let index = job.pump_index % job.pumps.len();
        job.pump_index = job.pump_index.wrapping_add(1);
        let pump = &mut job.pumps[index];
        let local = objects
            .port_mut(pump.local)
            .ok_or("connected port disappeared")?;
        if pump.source() {
            let terminal = matches!(local.buffer, Buffer::Terminal(Direction::Read));
            if terminal && !job.model.foreground {
                if pump.demand() {
                    pump.hush_demand();
                    job.model
                        .pause(PauseReason::BackgroundRead)
                        .map_err(|_| "background read pause failed")?;
                }
            } else {
                pump.source_step(local, &mut job.input, job.eof)
                    .map_err(String::from)?;
            }
        } else {
            pump.sink_step(local, &mut job.output, available)
                .map_err(String::from)?;
        }
    }
    if job.model.status == JobStatus::Cancelling {
        for (index, member) in job.members.iter().enumerate() {
            if member.built.is_none() {
                job.model
                    .skip(index)
                    .map_err(|_| "missing cancelled member")?;
            }
        }
    }
    let all_dead = job.model.results.iter().all(Option::is_some);
    let drained = job.pumps.iter().all(|pump| pump.done) && job.output.is_empty();
    if all_dead && (drained || job.model.status == JobStatus::Cancelling) {
        for link in &job.plan.links {
            for endpoint in [&link.source, &link.sink] {
                if let Endpoint::Port { id } = endpoint {
                    if let Some(local) = objects.port_mut(*id) {
                        if local.bound == Some(job.model.id) {
                            local.bound = None;
                        }
                    }
                }
            }
        }
        job.pumps.clear();
        for member in &mut job.members {
            member.manifest = None;
        }
        if let Some(root) = job.pipes.last() {
            job.work = Some(Work::Release {
                pending: pipe
                    .begin_release(root, WAIT)
                    .map_err(|e| format!("pipe release: {e:?}"))?,
            });
            return Ok(());
        }
        job.released = true;
        job.model.drained();
        return Ok(());
    }
    if matches!(
        job.model.status,
        JobStatus::Starting | JobStatus::Pausing | JobStatus::Resuming
    ) {
        if let Some(index) = job.model.pending_member() {
            let member = &mut job.members[index];
            let built = member.built.ok_or("missing running member")?;
            let result = match job.model.status {
                JobStatus::Pausing if !member.ran => Ok(()),
                JobStatus::Pausing => env::unit::debark_team(built.team),
                JobStatus::Resuming => env::unit::embark_team(built.team),
                _ => env::unit::embark_task(built.task),
            };
            match result {
                Ok(()) => {
                    if job.model.status != JobStatus::Pausing {
                        member.ran = true;
                    }
                    job.model.acknowledge(index).map_err(|_| "missing member")?;
                }
                Err(error) if error.source == env::UnitFail::Busy => {}
                Err(error) => return Err(format!("job transition: {error:?}")),
            }
            return Ok(());
        }
    }
    if job.model.status == JobStatus::Cancelling {
        for member in &mut job.members {
            if let Some(built) = member.built.filter(|_| !member.dead && !member.stopping) {
                env::unit::slay_team(built.team).map_err(|e| format!("job cancel: {e:?}"))?;
                member.stopping = true;
            }
        }
    }
    job.model.advance();
    Ok(())
}
