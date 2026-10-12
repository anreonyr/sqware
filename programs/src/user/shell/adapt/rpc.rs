use alloc::{format, string::String};
use env::{PieToken, TaskId, Wait};
use ipc::{
    rpc::request::{Pending, Sender},
    time::Deadline,
};
use shell_api::Image;
use system_api::{control::construction, loader};

pub const WAIT: Wait = Wait::AtMost(5000);

enum Phase {
    Build(Pending<construction::Call>),
    Inspect(Pending<construction::Call>),
    Cancel(Pending<construction::Call>),
    Claim(Pending<loader::Call>),
    Done,
}

pub struct Loading {
    construction: Sender<construction::Call>,
    loader: Sender<loader::Call>,
    deadline: Deadline,
    phase: Phase,
    image: Option<(TaskId, PieToken)>,
    image_root: Option<PieToken>,
    id: construction::CreateId,
    owner: TaskId,
    subject: system_api::identity::Subject,
    identity_authority: TaskId,
    image_len: u64,
    pub built: Option<loader::Built>,
}

impl Loading {
    pub fn begin(
        construction: PieToken,
        loader: PieToken,
        image: &Image,
        owner: TaskId,
        subject: system_api::identity::Subject,
        identity_authority: TaskId,
        id: construction::CreateId,
    ) -> Result<Self, String> {
        let construction_sender =
            Sender::<construction::Call>::from_raw(construction, construction::BACK)
                .map_err(|e| format!("construction entry: {e:?}"))?;
        let loader_sender = Sender::<loader::Call>::from_raw(loader, loader::BACK)
            .map_err(|e| format!("claim entry: {e:?}"))?;
        let peer = construction_sender.peer();
        let authority =
            resource::raw::inspect(construction).map_err(|_| "construction authority")?;
        if !authority.alive
            || authority.owner != peer
            || authority.vestor != peer
            || authority.kind != env::PieKind::Hole
            || authority.mark != construction::ENTRY
            || !authority.permission.contains(env::Permission::STORE)
        {
            return Err("construction authority provenance".into());
        }
        let deadline = Deadline::new(WAIT);
        let source = resource::raw::inspect(image.seed).map_err(|_| "image source")?;
        if !source.alive
            || source.owner != peer
            || source.vestor != peer
            || source.kind != env::PieKind::Pole
            || source.mark != image.mark
            || !source.permission.contains(env::Permission::FETCH)
            || image.length == 0
            || image.length > 16 * 1024 * 1024
        {
            return Err("image source provenance".into());
        }
        let source_dock = resource::dock::Dock::open(image.seed)
            .map_err(|e| format!("image source mapping: {e:?}"))?;
        let source_view = source_dock.view();
        if image.length > source_view.size() {
            let _ = source_dock.shut();
            return Err("image source bounds".into());
        }
        let size = image.length.div_ceil(env::PAGE_SIZE) * env::PAGE_SIZE;
        let root = env::pie::unseal(env::UnsealArgs::Pole { size, shared: true })
            .map_err(|e| format!("image snapshot allocation: {e:?}"))?;
        let (target, target_size) = match resource::raw::open(root) {
            Ok(mapped) => mapped,
            Err(error) => {
                let _ = resource::raw::release(root);
                let _ = source_dock.shut();
                return Err(format!("image snapshot mapping: {error:?}"));
            }
        };
        if target_size < image.length {
            let _ = env::pie::shut(root);
            let _ = resource::raw::release(root);
            let _ = source_dock.shut();
            return Err("image snapshot bounds".into());
        }
        // SAFETY: both mappings are live and image.length is bounded by their mapped sizes.
        unsafe {
            core::ptr::copy_nonoverlapping(
                source_view.base() as *const u8,
                target as *mut u8,
                image.length,
            );
        }
        if let Err(error) = source_dock.shut() {
            let _ = env::pie::shut(root);
            let _ = resource::raw::release(root);
            return Err(format!("image source unmap: {error:?}"));
        }
        if let Err(error) = env::pie::shut(root) {
            let _ = resource::raw::release(root);
            return Err(format!("image snapshot unmap: {error:?}"));
        }
        if let Err(error) = env::pie::narrow(root, env::Permission::FETCH | env::Permission::VEST) {
            let _ = resource::raw::release(root);
            return Err(format!("image snapshot freeze: {error:?}"));
        }
        let loan = match env::pie::accord(root, peer, env::Permission::FETCH, loader::IMAGE) {
            Ok(loan) => loan,
            Err(error) => {
                let _ = resource::raw::release(root);
                return Err(format!("image grant: {error:?}"));
            }
        };
        let mut args = [0; loader::MAX_ARGS];
        args[0] = owner.get() as u64;
        let pending = match construction_sender.begin(deadline, |back| construction::Request {
            op: construction::BUILD,
            id,
            input: construction::InputOption(Some(construction::Input::Image(
                construction::Image {
                    image: loan,
                    offset: 0,
                    len: image.length as u64,
                    stack: 0,
                    count: 1,
                    args,
                },
            ))),
            owner,
            subject,
            back,
        }) {
            Ok(pending) => pending,
            Err(error) => {
                let _ = env::pie::revoke(peer, loan);
                let _ = resource::raw::release(root);
                return Err(format!("construction request: {error:?}"));
            }
        };
        Ok(Self {
            construction: construction_sender,
            loader: loader_sender,
            deadline,
            phase: Phase::Build(pending),
            image: Some((peer, loan)),
            image_root: Some(root),
            id,
            owner,
            subject,
            identity_authority,
            image_len: image.length as u64,
            built: None,
        })
    }

    pub fn source(&self) -> Option<env::Source> {
        match &self.phase {
            Phase::Build(p) | Phase::Inspect(p) => Some(p.source()),
            Phase::Cancel(p) => Some(p.source()),
            Phase::Claim(p) => Some(p.source()),
            Phase::Done => None,
        }
    }

    pub fn remaining(&self) -> Wait {
        self.deadline.remaining()
    }
    pub fn done(&self) -> bool {
        matches!(&self.phase, Phase::Done)
    }

    pub fn cancel(&mut self) -> Result<(), String> {
        if matches!(&self.phase, Phase::Done | Phase::Cancel(_)) {
            return Ok(());
        }
        self.phase = Phase::Cancel(
            self.construction
                .begin(self.deadline, |back| construction::Request {
                    op: construction::CANCEL,
                    id: self.id,
                    input: construction::InputOption(None),
                    owner: self.owner,
                    subject: self.subject,
                    back,
                })
                .map_err(|e| format!("construction cancel: {e:?}"))?,
        );
        Ok(())
    }

    pub fn poll(&mut self) -> Result<Option<loader::Built>, String> {
        match &mut self.phase {
            Phase::Build(pending) => {
                let result = match pending.poll() {
                    Ok(Some(reply)) => reply,
                    Ok(None) => return Ok(None),
                    Err(_) => {
                        self.phase = Phase::Inspect(
                            self.construction
                                .begin(self.deadline, |back| construction::Request {
                                    op: construction::INSPECT,
                                    id: self.id,
                                    input: construction::InputOption(None),
                                    owner: self.owner,
                                    subject: self.subject,
                                    back,
                                })
                                .map_err(|e| format!("construction inspect: {e:?}"))?,
                        );
                        return Ok(None);
                    }
                };
                self.accept_creation(result)
            }
            Phase::Inspect(pending) => {
                let result = match pending.poll() {
                    Ok(Some(reply)) => reply,
                    Ok(None) => return Ok(None),
                    Err(_) => {
                        self.phase = Phase::Inspect(
                            self.construction
                                .begin(self.deadline, |back| construction::Request {
                                    op: construction::INSPECT,
                                    id: self.id,
                                    input: construction::InputOption(None),
                                    owner: self.owner,
                                    subject: self.subject,
                                    back,
                                })
                                .map_err(|e| format!("construction inspect retry: {e:?}"))?,
                        );
                        return Ok(None);
                    }
                };
                if result.state == construction::UNSEEN {
                    let (peer, old_image) = self
                        .image
                        .take()
                        .ok_or("construction image lease missing")?;
                    let _ = env::pie::revoke(peer, old_image);
                    let root = self.image_root.ok_or("construction image root missing")?;
                    let image = env::pie::accord(root, peer, env::Permission::FETCH, loader::IMAGE)
                        .map_err(|e| format!("construction image regrant: {e:?}"))?;
                    self.image = Some((peer, image));
                    let image_len = self.image_len;
                    let mut args = [0; loader::MAX_ARGS];
                    args[0] = self.owner.get() as u64;
                    self.phase = Phase::Build(
                        self.construction
                            .begin(self.deadline, |back| construction::Request {
                                op: construction::BUILD,
                                id: self.id,
                                input: construction::InputOption(Some(construction::Input::Image(
                                    construction::Image {
                                        image,
                                        offset: 0,
                                        len: image_len,
                                        stack: 0,
                                        count: 1,
                                        args,
                                    },
                                ))),
                                owner: self.owner,
                                subject: self.subject,
                                back,
                            })
                            .map_err(|e| {
                                let _ = env::pie::revoke(peer, image);
                                self.image = None;
                                format!("construction retry: {e:?}")
                            })?,
                    );
                    return Ok(None);
                }
                self.accept_creation(result)
            }
            Phase::Claim(pending) => {
                let Some(reply) = pending
                    .poll()
                    .map_err(|error| format!("claim transport: {error:?}"))?
                else {
                    return Ok(None);
                };
                let built = self.built.ok_or("claim has no instance")?;
                if reply.status != 0
                    || reply.task != built.task
                    || reply.team != built.team.get() as u64
                {
                    return Err("claim mismatch".into());
                }
                self.phase = Phase::Done;
                Ok(Some(built))
            }
            Phase::Cancel(pending) => {
                let reply = match pending.poll() {
                    Ok(Some(reply)) => reply,
                    Ok(None) => return Ok(None),
                    Err(_) => {
                        self.phase = Phase::Cancel(
                            self.construction
                                .begin(self.deadline, |back| construction::Request {
                                    op: construction::CANCEL,
                                    id: self.id,
                                    input: construction::InputOption(None),
                                    owner: self.owner,
                                    subject: self.subject,
                                    back,
                                })
                                .map_err(|e| format!("construction cancel retry: {e:?}"))?,
                        );
                        return Ok(None);
                    }
                };
                if matches!(
                    reply.state,
                    construction::CANCELLED | construction::FAILED | construction::CLOSED
                ) {
                    if let Some((peer, token)) = self.image.take() {
                        let _ = env::pie::revoke(peer, token);
                    }
                    self.release_image_root();
                    self.phase = Phase::Done;
                    Ok(None)
                } else {
                    self.phase = Phase::Cancel(
                        self.construction
                            .begin(self.deadline, |back| construction::Request {
                                op: construction::CANCEL,
                                id: self.id,
                                input: construction::InputOption(None),
                                owner: self.owner,
                                subject: self.subject,
                                back,
                            })
                            .map_err(|e| format!("construction cancel retry: {e:?}"))?,
                    );
                    Ok(None)
                }
            }
            Phase::Done => Ok(self.built),
        }
    }

    fn accept_creation(
        &mut self,
        reply: system_api::loader::Said,
    ) -> Result<Option<loader::Built>, String> {
        match reply.state {
            construction::PENDING => {
                self.phase = Phase::Inspect(
                    self.construction
                        .begin(self.deadline, |back| construction::Request {
                            op: construction::INSPECT,
                            id: self.id,
                            input: construction::InputOption(None),
                            owner: self.owner,
                            subject: self.subject,
                            back,
                        })
                        .map_err(|e| format!("construction inspect: {e:?}"))?,
                );
                Ok(None)
            }
            construction::FAILED => Err(format!("construction rejected: {}", reply.status)),
            construction::CANCELLED | construction::CLOSED => Err("construction cancelled".into()),
            construction::COMPLETED
                if reply.status == 0 && reply.task.get() != 0 && reply.team != 0 =>
            {
                let built = loader::Built {
                    task: reply.task,
                    team: env::TeamId::new(reply.team as usize),
                };
                system_client::identity::forward_resolve(
                    reply.startup.resolve,
                    self.identity_authority,
                    self.construction.peer(),
                    built.task,
                )
                .map_err(|error| format!("instance Resolve handoff: {error:?}"))?;
                self.built = Some(built);
                if let Some((peer, token)) = self.image.take() {
                    let _ = env::pie::revoke(peer, token);
                }
                self.release_image_root();
                self.phase = Phase::Claim(
                    self.loader
                        .begin(self.deadline, |back| {
                            loader::Wire::Claim(loader::Claim {
                                op: loader::CLAIM,
                                task: built.task,
                                back,
                            })
                        })
                        .map_err(|e| format!("claim request: {e:?}"))?,
                );
                Ok(None)
            }
            _ => Err("invalid construction result".into()),
        }
    }
}

impl Drop for Loading {
    fn drop(&mut self) {
        if let Some((peer, token)) = self.image.take() {
            let _ = env::pie::revoke(peer, token);
        }
        self.release_image_root();
    }
}

impl Loading {
    fn release_image_root(&mut self) {
        if let Some(root) = self.image_root.take() {
            let _ = env::pie::release(root, env::ReleaseMode::Revoke);
        }
    }
}
