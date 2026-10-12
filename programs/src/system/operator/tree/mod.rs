//! 一张按号排的表 ＋ 八条线上原语 ＋ 三条给判据的。

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use env::{Access, PieToken, TaskId};

use ::resource::raw::pies;
use env::pie;
use ipc::session::{alive, opened_by};
use system_api::operator::EntryId;
use system_api::operator::Fail;
use system_api::operator::Kind;
use system_api::operator::PANE_CAP;
use system_api::operator::Permit;
use system_api::operator::Where;
use system_api::operator::path::Path;
use system_api::operator::path::PathBuf;

mod gate;
mod judge;
pub(super) use gate::{Code, verdict};
pub(super) use judge::Facts;

/// **一格**：名字 + 去处。它住在 Operator::slots 里，**下标就是它的号**
/// 名字只是一段（String：不长于 255 字节那一格由**编帧**那一刻判），不是整条路——路那一层只剩
struct Slot {
    name: String,
    node: Node,
    revision: u64,
    authority: Option<super::authority::GrantKey>,
    view: Access,
    /// Caller-scoped retry identity, never a source or authority credential.
    resource_id: u64,
}

/// 去处：一块 Node::Pane（窗格，还能往里走）或一枚 Node::Tile（砖，到头了）
enum Node {
    /// 一块 Pane（窗格）：里面是**孩子的号**（按登记序）。**还能往里走**
    Pane(Vec<EntryId>),
    /// 一枚 Tile（砖）：到头了，就是内核给的那一枚句柄；**「用」与「改」两轴都住在它上面**
    /// **两轴为什么不挂 `Slot`**：① 许可只在 `find` 那一问上被读，而 `find` 只认砖 ⇒ 它本来就
    /// 是"一枚砖的性质"；② 挂 `Slot` 会造出"一块 `Pane` 也有许可 / 也有主人"这两格，而它们
    Tile {
        pie: PieToken,
        permit: Permit,
        owner: Option<TaskId>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Want {
    /// 要一枚 `Tile`（Operator::land）
    Tile,
    /// 要一块 `Pane`（Operator::part）
    Pane,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Key<'a> {
    /// 坐标：那一块 `Pane` + 那一段名字（`land` / `part` 那一问手里有的）
    At(Where, &'a str),
    /// 号：条目自己的号（`find` / `trim` 手里有的）
    Id(EntryId),
}

/// **一次改动**：树真的变了才有它（那一格自己的号 ＋ 它是哪一种 ＋ 归属）。
///
/// **不含"路"那一格**：路能从号现走（[`Operator::road_to`]），而 `trim` 正是"号还在、格子没了"
/// ——那一刻才是唯一需要**先**把路记下来的地方。故这里只记那三格，路在落那条事件时现走。
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Change {
    /// 哪一种：新铸 / 换绑 / 新立一块 / 剪掉
    pub kind: Kind,
    /// 那一格自己的号
    pub id: EntryId,
    /// 那一格的主人（无主 = 零号）
    pub owner: TaskId,
    /// **剪掉那一档才填**：剪完槽就空了，路只能**剪之前**记下来（其余三档由 `publish` 现走）。
    pub road: Option<PathBuf>,
}

/// 一棵命名树：**一个 Operator 管着所有条目**
/// 根 = `root` 那一叠**孩子的号**（Where::Root 指的就是它）；其余每一格住 `slots` 里
/// **号就是下标**。两个容器的容量**互不牵连**：`root` 管根那一层，`slots` 管"一共铸过几格"
pub struct Operator {
    root: Vec<EntryId>,
    authority: super::authority::Authority,
    system: TaskId,
    actor: TaskId,
    identity: Option<system_client::identity::TaskQuery>,
    next_revision: u64,
    pending_changes: Vec<Change>,
    pending_import_release: Vec<PieToken>,
    operations: Vec<OperationRecord>,
    operation_watermarks: Vec<OperationWatermark>,
    /// 一叠格子：**只增不减**（`unlink` 只把那一格置空，不 `pop`）
    /// ⇒ **下标即号、号不重用**：一枚旧号要么还指着原来那一格，要么指着墓碑
    /// （Fail::Unknown），不会悄悄指到后铸的那一格身上。铸号因此**不必另立水位**——水位
    slots: Vec<Option<Slot>>,
}

struct Location {
    at: Where,
    name: String,
}
#[derive(Clone, Copy)]
struct Tile {
    pie: PieToken,
    permit: Permit,
    owner: Option<TaskId>,
}
pub struct PartEntry {
    pub anchor: system_api::operator::Entry,
    pub name: String,
    pub authority: Option<super::authority::GrantKey>,
}
pub struct BindEntry {
    pub anchor: system_api::operator::Entry,
    pub name: String,
    pub pie: PieToken,
    pub resource_id: u64,
    pub permit: Permit,
    pub owner: TaskId,
    pub authority: super::authority::GrantKey,
    pub view: Access,
}
pub struct ReplaceEntry {
    pub binding: system_api::operator::Binding,
    pub pie: PieToken,
    pub resource_id: u64,
    pub permit: Permit,
    pub owner: TaskId,
    pub authority: super::authority::GrantKey,
    pub view: Access,
}

#[derive(Clone, PartialEq, Eq)]
pub struct OperationRequest {
    pub via: PieToken,
    pub caller: TaskId,
    pub id: system_api::operator::OperationId,
    pub op: u8,
    pub anchor: system_api::operator::Entry,
    pub name: String,
    pub resource_id: u64,
    pub permit: Permit,
    pub view: Access,
}

#[derive(Clone, Copy)]
pub struct OperationRef {
    pub via: PieToken,
    pub caller: TaskId,
    pub id: system_api::operator::OperationId,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OperationResult {
    Part {
        entry: system_api::operator::Entry,
        created: bool,
    },
    Bind {
        binding: system_api::operator::Binding,
        created: bool,
    },
    Failed(Fail),
}

struct OperationRecord {
    via: PieToken,
    caller: TaskId,
    id: system_api::operator::OperationId,
    request: Option<OperationRequest>,
    result: Option<OperationResult>,
    closed: bool,
}

#[derive(Clone, Copy)]
struct OperationWatermark {
    via: PieToken,
    caller: TaskId,
    high_water: u64,
}
struct PlacementMeta {
    authority: Option<super::authority::GrantKey>,
    view: Access,
    resource_id: u64,
}
struct Placement {
    location: Location,
    node: Node,
    meta: PlacementMeta,
}
struct Part {
    at: Where,
    name: String,
    authority: Option<super::authority::GrantKey>,
}
struct Land {
    location: Location,
    tile: Tile,
    authority: Option<super::authority::GrantKey>,
    view: Access,
    resource_id: u64,
}

impl Operator {
    pub fn new(system: TaskId, actor: TaskId) -> Operator {
        Operator {
            root: Vec::new(),
            authority: super::authority::Authority::new(system, actor),
            system,
            actor,
            identity: None,
            next_revision: 0,
            pending_changes: Vec::new(),
            pending_import_release: Vec::new(),
            operations: Vec::new(),
            operation_watermarks: Vec::new(),
            slots: Vec::new(),
        }
    }

    pub fn entry(&self, target: Where) -> Result<system_api::operator::Entry, Fail> {
        if let Where::At(id) = target {
            if !self.visible(id) {
                return Err(Fail::Unknown);
            }
        }
        Ok(system_api::operator::Entry {
            operator: self.actor,
            target: match target {
                Where::Root => system_api::operator::EntryTarget::Root,
                Where::At(id) => system_api::operator::EntryTarget::Node(id),
            },
        })
    }

    pub fn part_entry(
        &mut self,
        anchor: system_api::operator::Entry,
        name: String,
    ) -> Result<(system_api::operator::Entry, bool, Option<Change>), Fail> {
        self.part_entry_with_authority(PartEntry {
            anchor,
            name,
            authority: None,
        })
    }

    pub fn part_entry_with_authority(
        &mut self,
        part: PartEntry,
    ) -> Result<(system_api::operator::Entry, bool, Option<Change>), Fail> {
        let at = self.where_for(part.anchor)?;
        let (id, fresh, change) = self.part_at_with_authority(Part {
            at,
            name: part.name,
            authority: part.authority,
        })?;
        Ok((self.entry(Where::At(id))?, fresh, change))
    }

    /// Reserves a stable native mutation request or returns its retained receipt.
    pub fn begin_operation(
        &mut self,
        request: OperationRequest,
    ) -> Result<Option<OperationResult>, Fail> {
        if request.id.0 == 0 {
            return Err(Fail::Denied);
        }
        if let Some(record) = self.operations.iter().find(|record| {
            record.via == request.via && record.caller == request.caller && record.id == request.id
        }) {
            return if !record.closed && record.request.as_ref() == Some(&request) {
                Ok(record.result)
            } else {
                Err(Fail::Denied)
            };
        }
        let watermark = self.operation_watermarks.iter().position(|watermark| {
            watermark.via == request.via && watermark.caller == request.caller
        });
        if watermark.is_some_and(|at| request.id.0 <= self.operation_watermarks[at].high_water) {
            return Err(Fail::Denied);
        }
        let evict = if self.operations.len() >= 256 {
            Some(
                self.operations
                    .iter()
                    .position(|record| record.closed || record.result.is_some())
                    .ok_or(Fail::Full)?,
            )
        } else {
            self.operations.try_reserve(1).map_err(|_| Fail::Full)?;
            None
        };
        if watermark.is_none() && self.operation_watermarks.len() >= 256 {
            return Err(Fail::Full);
        }
        if watermark.is_none() {
            self.operation_watermarks
                .try_reserve(1)
                .map_err(|_| Fail::Full)?;
        }
        if let Some(at) = evict {
            self.operations.remove(at);
        }
        if let Some(at) = watermark {
            self.operation_watermarks[at].high_water = request.id.0;
        } else {
            self.operation_watermarks.push(OperationWatermark {
                via: request.via,
                caller: request.caller,
                high_water: request.id.0,
            });
        }
        self.operations.push(OperationRecord {
            via: request.via,
            caller: request.caller,
            id: request.id,
            request: Some(request),
            result: None,
            closed: false,
        });
        Ok(None)
    }

    pub fn finish_operation(
        &mut self,
        request: &OperationRequest,
        result: OperationResult,
    ) -> Result<(), Fail> {
        let record = self
            .operations
            .iter_mut()
            .find(|record| record.request.as_ref() == Some(request))
            .ok_or(Fail::Unknown)?;
        record.result = Some(result);
        Ok(())
    }

    pub fn operation_state(&self, request: OperationRef) -> system_api::operator::OperationState {
        if let Some(record) = self.operations.iter().find(|record| {
            record.via == request.via && record.caller == request.caller && record.id == request.id
        }) {
            return if record.closed {
                system_api::operator::OperationState::Closed
            } else {
                match record.result {
                    Some(OperationResult::Failed(_)) => {
                        system_api::operator::OperationState::Failed
                    }
                    Some(_) => system_api::operator::OperationState::Applied,
                    None => system_api::operator::OperationState::Pending,
                }
            };
        }
        let high = self
            .operation_watermarks
            .iter()
            .find(|watermark| watermark.via == request.via && watermark.caller == request.caller)
            .map_or(0, |watermark| watermark.high_water);
        if request.id.0 <= high {
            system_api::operator::OperationState::Closed
        } else {
            system_api::operator::OperationState::Unseen
        }
    }

    pub fn forget_operations(&mut self, via: PieToken) {
        self.operations.retain(|record| record.via != via);
        self.operation_watermarks
            .retain(|watermark| watermark.via != via);
    }

    pub fn cancel_operation(
        &mut self,
        request: OperationRef,
    ) -> Result<system_api::operator::OperationState, Fail> {
        let state = self.operation_state(request);
        match state {
            system_api::operator::OperationState::Pending => Err(Fail::Dead),
            system_api::operator::OperationState::Applied
            | system_api::operator::OperationState::Failed
            | system_api::operator::OperationState::Closed => Ok(state),
            system_api::operator::OperationState::Unseen => {
                let watermark = self.operation_watermarks.iter().position(|watermark| {
                    watermark.via == request.via && watermark.caller == request.caller
                });
                if watermark.is_none() && self.operation_watermarks.len() >= 256 {
                    return Err(Fail::Full);
                }
                let evict = if self.operations.len() >= 256 {
                    Some(
                        self.operations
                            .iter()
                            .position(|record| record.closed || record.result.is_some())
                            .ok_or(Fail::Full)?,
                    )
                } else {
                    self.operations.try_reserve(1).map_err(|_| Fail::Full)?;
                    None
                };
                if watermark.is_none() {
                    self.operation_watermarks
                        .try_reserve(1)
                        .map_err(|_| Fail::Full)?;
                }
                if let Some(at) = evict {
                    self.operations.remove(at);
                }
                if let Some(at) = watermark {
                    self.operation_watermarks[at].high_water =
                        self.operation_watermarks[at].high_water.max(request.id.0);
                } else {
                    self.operation_watermarks.push(OperationWatermark {
                        via: request.via,
                        caller: request.caller,
                        high_water: request.id.0,
                    });
                }
                self.operations.push(OperationRecord {
                    via: request.via,
                    caller: request.caller,
                    id: request.id,
                    request: None,
                    result: None,
                    closed: true,
                });
                Ok(system_api::operator::OperationState::Closed)
            }
        }
    }

    pub fn bind_at(
        &mut self,
        bind: BindEntry,
    ) -> Result<(system_api::operator::Binding, Option<Change>), Fail> {
        let at = self.where_for(bind.anchor)?;
        if let Some(id) = self.kid(at, bind.name.as_str())? {
            if matches!(self.slot(id).map(|slot| &slot.node), Some(Node::Tile { permit: old_permit, owner: Some(old_owner), .. }) if *old_permit == bind.permit && *old_owner == bind.owner)
                && self.slot(id).is_some_and(|slot| {
                    slot.authority == Some(bind.authority)
                        && slot.view == bind.view
                        && slot.resource_id == bind.resource_id
                })
            {
                return Ok((self.binding(id)?, None));
            }
            return Err(Fail::Denied);
        }
        let change = self.land_with_authority(Land {
            location: Location {
                at,
                name: bind.name,
            },
            tile: Tile {
                pie: bind.pie,
                permit: bind.permit,
                owner: Some(bind.owner),
            },
            authority: Some(bind.authority),
            view: bind.view,
            resource_id: bind.resource_id,
        })?;
        let binding = self.binding(change.id)?;
        Ok((binding, Some(change)))
    }

    pub fn binding(&self, id: EntryId) -> Result<system_api::operator::Binding, Fail> {
        if !self.visible(id) {
            return Err(Fail::Unknown);
        }
        let slot = self.slot(id).ok_or(Fail::Unknown)?;
        if !matches!(slot.node, Node::Tile { .. }) {
            return Err(Fail::NotATile);
        }
        Ok(system_api::operator::Binding {
            entry: self.entry(Where::At(id))?,
            revision: slot.revision,
        })
    }

    pub fn binding_at(
        &self,
        anchor: system_api::operator::Entry,
        name: &str,
    ) -> Result<
        (
            system_api::operator::Binding,
            PieToken,
            system_api::operator::ResourceView,
        ),
        Fail,
    > {
        let at = self.where_for(anchor)?;
        let id = self.kid(at, name)?.ok_or(Fail::Unknown)?;
        let slot = self.slot(id).ok_or(Fail::Unknown)?;
        let Node::Tile { pie, .. } = &slot.node else {
            return Err(Fail::NotATile);
        };
        let view = system_api::operator::ResourceView::new(slot.view).ok_or(Fail::Denied)?;
        Ok((self.binding(id)?, *pie, view))
    }

    pub fn replace_binding(
        &mut self,
        replacement: ReplaceEntry,
    ) -> Result<(system_api::operator::Binding, Option<Change>), Fail> {
        let ReplaceEntry {
            binding,
            pie,
            resource_id,
            permit,
            owner,
            authority,
            view,
        } = replacement;
        self.check_binding(binding)?;
        if matches!(self.slot(binding.entry_id()).map(|slot| &slot.node), Some(Node::Tile { permit: old_permit, owner: Some(old_owner), .. }) if *old_permit == permit && *old_owner == owner)
        {
            if self.slot(binding.entry_id()).is_some_and(|slot| {
                slot.authority == Some(authority)
                    && slot.view == view
                    && slot.resource_id == resource_id
            }) {
                return Ok((binding, None));
            }
            let revision = self.next_revision.checked_add(1).ok_or(Fail::Full)?;
            let old = match self.slot(binding.entry_id()).map(|slot| &slot.node) {
                Some(Node::Tile { pie, .. }) => *pie,
                _ => return Err(Fail::NotATile),
            };
            if old != pie && pies().any(|held| held.token == old) {
                pie::release(old, env::ReleaseMode::Keep).map_err(|_| Fail::Unknown)?;
            }
            self.next_revision = revision;
            let slot = self
                .slots
                .get_mut(binding.entry_id().get())
                .and_then(Option::as_mut)
                .ok_or(Fail::Unknown)?;
            if let Node::Tile { pie: current, .. } = &mut slot.node {
                *current = pie;
            }
            slot.authority = Some(authority);
            slot.revision = revision;
            slot.view = view;
            slot.resource_id = resource_id;
            let change = Change {
                kind: Kind::Rebound,
                id: binding.entry_id(),
                owner,
                road: None,
            };
            return Ok((
                system_api::operator::Binding {
                    revision,
                    ..binding
                },
                Some(change),
            ));
        }
        let (at, name) = self.locate(binding.entry_id()).ok_or(Fail::Unknown)?;
        let change = self.land_with_authority(Land {
            location: Location { at, name },
            tile: Tile {
                pie,
                permit,
                owner: Some(owner),
            },
            authority: Some(authority),
            view,
            resource_id,
        })?;
        Ok((self.binding(change.id)?, Some(change)))
    }

    pub fn bind_replay(&self, bind: &BindEntry) -> bool {
        self.where_for(bind.anchor).ok().and_then(|at| self.kid(at, &bind.name).ok().flatten())
            .and_then(|id| self.slot(id)).is_some_and(|slot| {
                slot.resource_id == bind.resource_id && slot.authority == Some(bind.authority) && slot.view == bind.view
                    && matches!(slot.node, Node::Tile { permit, owner: Some(owner), .. } if permit == bind.permit && owner == bind.owner)
            })
    }

    pub fn replace_replay(&self, replacement: &ReplaceEntry) -> bool {
        self.where_for(replacement.binding.entry).ok().and_then(|_| self.slot(replacement.binding.entry_id()))
            .is_some_and(|slot| slot.revision == replacement.binding.revision && slot.resource_id == replacement.resource_id
                && slot.authority == Some(replacement.authority) && slot.view == replacement.view
                && matches!(slot.node, Node::Tile { permit, owner: Some(owner), .. } if permit == replacement.permit && owner == replacement.owner))
    }

    pub fn reserve_import_cleanup(&mut self) -> Result<(), Fail> {
        if self.pending_import_release.len() >= 256 {
            return Err(Fail::Full);
        }
        self.pending_import_release
            .try_reserve(1)
            .map_err(|_| Fail::Full)
    }

    pub fn release_import(&mut self, token: PieToken) -> Result<(), Fail> {
        loop {
            match pie::release(token, env::ReleaseMode::Keep) {
                Ok(()) => return Ok(()),
                Err(error) if error.source == env::PieFail::Busy => env::room::starve(),
                Err(_) if ::resource::raw::inspect(token).is_err() => return Ok(()),
                Err(_) => {
                    if self.pending_import_release.len() >= 256 {
                        return Err(Fail::Full);
                    }
                    self.pending_import_release
                        .try_reserve(1)
                        .map_err(|_| Fail::Full)?;
                    self.pending_import_release.push(token);
                    return Err(Fail::Dead);
                }
            }
        }
    }

    pub fn retry_import_cleanup(&mut self) {
        self.pending_import_release
            .retain(|token| pie::release(*token, env::ReleaseMode::Keep).is_err());
    }

    pub fn unbind_binding(
        &mut self,
        binding: system_api::operator::Binding,
    ) -> Result<Option<Change>, Fail> {
        match self.check_binding(binding) {
            Ok(()) => {}
            Err(Fail::Unknown) => return Ok(None),
            Err(fail) => return Err(fail),
        }
        self.trim(binding.entry_id())
    }

    pub fn unpart(&mut self, entry: system_api::operator::Entry) -> Result<Option<Change>, Fail> {
        let Where::At(id) = self.where_for(entry)? else {
            return Err(Fail::Denied);
        };
        if !matches!(self.slot(id).map(|slot| &slot.node), Some(Node::Pane(_))) {
            return Err(Fail::NotAPane);
        }
        self.trim(id)
    }

    pub fn seek_from(
        &self,
        anchor: system_api::operator::Entry,
        relative: &Path,
    ) -> Result<system_api::operator::Entry, Fail> {
        let mut at = self.where_for(anchor)?;
        for part in relative.iter() {
            let id = self.kid(at, part)?.ok_or(Fail::Unknown)?;
            at = match self.slot(id).map(|s| &s.node) {
                Some(Node::Pane(_)) => Where::At(id),
                _ if part == relative.file_name().unwrap_or("") => Where::At(id),
                _ => return Err(Fail::NotAPane),
            };
        }
        self.entry(match at {
            Where::Root => Where::Root,
            Where::At(id) => Where::At(id),
        })
    }

    pub fn relative_to(
        &self,
        anchor: system_api::operator::Entry,
        target: system_api::operator::Entry,
    ) -> Option<PathBuf> {
        self.where_for(anchor).ok()?;
        self.where_for(target).ok()?;
        let base = match anchor.target {
            system_api::operator::EntryTarget::Root => String::new(),
            system_api::operator::EntryTarget::Node(id) => self.road_to(id)?.as_str().to_string(),
        };
        let road = match target.target {
            system_api::operator::EntryTarget::Root => String::new(),
            system_api::operator::EntryTarget::Node(id) => self.road_to(id)?.as_str().to_string(),
        };
        let relative = if base.is_empty() {
            road.as_str()
        } else if road == base {
            ""
        } else {
            road.strip_prefix(&base)?.strip_prefix('/')?
        };
        PathBuf::try_new(relative)
    }

    fn where_for(&self, entry: system_api::operator::Entry) -> Result<Where, Fail> {
        if entry.operator != self.actor {
            return Err(Fail::Denied);
        }
        match entry.target {
            system_api::operator::EntryTarget::Root => Ok(Where::Root),
            system_api::operator::EntryTarget::Node(id) => {
                if !self.visible(id) {
                    return Err(Fail::Unknown);
                }
                Ok(Where::At(id))
            }
        }
    }

    fn check_binding(&self, binding: system_api::operator::Binding) -> Result<(), Fail> {
        self.where_for(binding.entry)?;
        if self.binding(binding.entry_id())?.revision != binding.revision {
            return Err(Fail::Denied);
        }
        Ok(())
    }

    pub fn grant(
        &mut self,
        request: system_api::operator::GrantSubmission,
    ) -> Result<
        (system_api::operator::GrantState, super::authority::GrantKey),
        super::authority::AuthorityError,
    > {
        let Some((_, parent, parent_spec)) = self.authority.grant_at(request.via, request.caller)
        else {
            return Err(super::authority::AuthorityError::Denied);
        };
        if request.parent != Some(parent) {
            return Err(super::authority::AuthorityError::Denied);
        }
        let relative = self
            .relative_to(parent_spec.anchor, request.spec.anchor)
            .ok_or(super::authority::AuthorityError::Denied)?;
        self.authority.grant(request, relative.as_path())
    }

    pub fn bootstrap_root(
        &mut self,
        caller: TaskId,
        entrance: PieToken,
    ) -> Result<(), super::authority::AuthorityError> {
        self.authority.bootstrap_root(super::authority::RootGrant {
            system: caller,
            entrance,
            spec: system_api::operator::GrantSpec {
                id: system_api::operator::GrantId(0),
                holder: caller,
                anchor: system_api::operator::Entry {
                    operator: self.actor,
                    target: system_api::operator::EntryTarget::Root,
                },
                scope: system_api::operator::Scope::Subtree,
                ops: system_api::operator::Ops::ALL,
                permit: None,
                source: None,
                source_mark: None,
            },
        })
    }

    pub fn grant_at(
        &self,
        entrance: PieToken,
        caller: TaskId,
    ) -> Option<(
        super::authority::GrantKey,
        system_api::operator::GrantId,
        &system_api::operator::GrantSpec,
    )> {
        self.authority.grant_at(entrance, caller)
    }

    pub fn authority_id(&self, entrance: PieToken) -> Option<super::authority::GrantKey> {
        self.authority.by_entrance(entrance)
    }

    pub fn grant_at_by_id(
        &self,
        id: super::authority::GrantKey,
        caller: TaskId,
    ) -> Option<(
        system_api::operator::GrantId,
        &system_api::operator::GrantSpec,
    )> {
        self.authority.by_id(id, caller)
    }

    pub fn set_entrance(
        &mut self,
        key: super::authority::GrantRef,
        token: PieToken,
    ) -> Result<(), super::authority::AuthorityError> {
        self.authority.set_entrance(key, token)
    }

    pub fn entrance(&self, key: super::authority::GrantRef) -> Option<PieToken> {
        self.authority.entrance(key)
    }

    pub fn system_task(&self) -> TaskId {
        self.system
    }

    pub fn operator_task(&self) -> TaskId {
        self.actor
    }

    pub fn set_identity_query(&mut self, query: Option<system_client::identity::TaskQuery>) {
        self.identity = query;
    }

    pub fn identity_query(&self) -> Option<&system_client::identity::TaskQuery> {
        self.identity.as_ref()
    }

    pub fn refresh_identity_query(&mut self) {
        if self
            .identity
            .as_ref()
            .is_some_and(|query| !query.available())
        {
            self.identity = None;
        }
    }

    pub fn queue_changes(&mut self, changes: Vec<Change>) {
        if self.pending_changes.try_reserve(changes.len()).is_ok() {
            self.pending_changes.extend(changes);
        }
    }

    pub fn take_changes(&mut self) -> Vec<Change> {
        core::mem::take(&mut self.pending_changes)
    }

    pub fn root_entrance(&self) -> Option<PieToken> {
        self.authority.root_entrance()
    }

    pub fn grant_delivered(
        &mut self,
        key: super::authority::GrantRef,
        remote: PieToken,
    ) -> Result<(), super::authority::AuthorityError> {
        self.authority.delivered(key, remote)
    }

    pub fn grant_remote(&self, key: super::authority::GrantKey) -> Option<PieToken> {
        self.authority.remote(key)
    }

    pub fn inspect_grant(
        &self,
        key: super::authority::GrantRef,
    ) -> Result<system_api::operator::GrantState, super::authority::AuthorityError> {
        self.authority.inspect(key)
    }

    pub fn revoke_grant(
        &mut self,
        key: super::authority::GrantRef,
    ) -> Result<system_api::operator::GrantState, super::authority::AuthorityError> {
        self.authority.revoke(key)
    }

    pub fn revoke_plan(
        &mut self,
        key: super::authority::GrantRef,
    ) -> Result<
        Vec<(
            super::authority::GrantKey,
            PieToken,
            TaskId,
            system_api::operator::GrantId,
            Option<PieToken>,
        )>,
        super::authority::AuthorityError,
    > {
        self.authority.revoke_plan(key)
    }

    pub fn grant_closed(
        &mut self,
        key: super::authority::GrantRef,
    ) -> Result<(), super::authority::AuthorityError> {
        self.authority.closed(key)
    }

    pub fn authority_live(&self, authority: super::authority::GrantKey) -> bool {
        self.authority.live(authority)
    }

    pub fn cleanup_progress(
        &self,
        key: super::authority::GrantKey,
    ) -> Option<(bool, bool, bool, bool)> {
        self.authority.cleanup_progress(key)
    }

    pub fn grant_attached(
        &mut self,
        key: super::authority::GrantKey,
    ) -> Result<(), super::authority::AuthorityError> {
        self.authority.attached(key)
    }

    pub fn cleanup_step(
        &mut self,
        key: super::authority::GrantKey,
        step: u8,
    ) -> Result<(), super::authority::AuthorityError> {
        self.authority.cleanup_step(key, step)
    }

    pub fn cleanup_authority(&mut self, authority: super::authority::GrantKey) -> Result<(), Fail> {
        for id in (0..self.slots.len()).rev().map(EntryId::new) {
            if self.slot(id).is_some_and(|slot| {
                slot.authority == Some(authority) && matches!(slot.node, Node::Tile { .. })
            }) {
                self.trim(id)?;
            }
        }
        for id in (0..self.slots.len()).rev().map(EntryId::new) {
            if self.slot(id).is_some_and(|slot| {
                slot.authority == Some(authority) && matches!(slot.node, Node::Pane(_))
            }) {
                self.trim(id)?;
            }
        }
        Ok(())
    }

    /// **落**：`at` 那一块里给 `name` 贴一枚 `Tile`，并答**这一次真动了什么**。
    ///
    /// 新铸那一档是 [`Kind::Landed`]，换绑那一档是 [`Kind::Rebound`]（**号不动**）——
    /// 树一个字节没变的那一档不存在：`put` 只在"造了一格"或"换掉了那一格的两轴"两种下场里返 `Ok`。
    fn land_with_authority(&mut self, land: Land) -> Result<Change, Fail> {
        let Land {
            location,
            tile,
            authority,
            view,
            resource_id,
        } = land;
        let Location { at, name } = location;
        let Tile { pie, permit, owner } = tile;
        // **归属原样带过去**：`None` = 无主（谁都能接手），`Some(w)` = 有主。**不许折成零号**——
        // `claimable` 读的就是这一格（零号是一个"永远不在场"的主人，与"无主"不是一件事）。
        let who = owner.unwrap_or(TaskId::new(0));
        // **换绑还是新铸**：落之前先只读地问一遍（那一手与 `put` 开头那一趟是同一件事）。
        let was_tile = self.kid(at, name.as_str())?.is_some();
        let id = self.put(Placement {
            location: Location { at, name },
            node: Node::Tile { pie, permit, owner },
            meta: PlacementMeta {
                authority,
                view,
                resource_id,
            },
        })?;
        Ok(Change {
            kind: if was_tile {
                Kind::Rebound
            } else {
                Kind::Landed
            },
            id,
            owner: who,
            road: None,
        })
    }

    /// **分 ＋ 真动了什么**：**真造了一块**才记 [`Kind::Parted`]（已是窗格那一档 `changed = false`
    /// ——树一个字节没变，不该报一条事件）。**两档都答那一格自己的号**（幂等那一档要把它查出来）。
    fn part_at_with_authority(
        &mut self,
        part: Part,
    ) -> Result<(EntryId, bool, Option<Change>), Fail> {
        let Part {
            at,
            name,
            authority,
        } = part;
        if let Some(id) = self.kid(at, name.as_str())? {
            match self.slot(id).map(|slot| &slot.node) {
                Some(Node::Pane(_)) => return Ok((id, false, None)),
                Some(Node::Tile { .. }) => return Err(Fail::NotAPane),
                None => return Err(Fail::Unknown),
            }
        }
        let id = self.put(Placement {
            location: Location { at, name },
            node: Node::Pane(Vec::new()),
            meta: PlacementMeta {
                authority,
                view: Access::NONE,
                resource_id: 0,
            },
        })?;
        Ok((
            id,
            true,
            Some(Change {
                kind: Kind::Parted,
                id,
                owner: TaskId::new(0),
                road: None,
            }),
        ))
    }

    /// `at` 那一块里叫 `name` 的那一格（没有 ⇒ `None`）。`put` 那一趟"先只读地问一遍"
    /// 就是它——故这一只手是那件事的公开发法，不是另一份实现。
    pub fn kid(&self, at: Where, name: &str) -> Result<Option<EntryId>, Fail> {
        Ok(self.kids(at)?.iter().copied().find(|child| {
            self.visible(*child) && self.slot(*child).is_some_and(|slot| slot.name == name)
        }))
    }

    /// **寻**：把那一号后面那一枚 Pie 交出去
    /// - 号不在树上 ⇒ Fail::Unknown（剪掉、剔死、从没铸过长得一样）
    /// - 那一格是一块 `Pane` ⇒ Fail::NotATile
    /// - 是一枚 `Tile`：**先问一句存活**（`establish::alive`）——不在 ⇒ 当场剔掉那一格、
    /// 放下那一份（pie::release），答 Fail::Dead；在 ⇒ 交给 `ship`
    /// `ship` 是"交出去"那一手（**回调**：适配层把那一枚授给调用方）——与"放下"正好是
    /// 一式的两半（交出 / 放下）。它留成参数而不直接叫，是因为它带**去授给谁**那一格
    /// （调用方手里那个号），不是无参动作
    pub fn find(
        &mut self,
        id: EntryId,
        mut ship: impl FnMut(PieToken, Access),
    ) -> Result<(), Fail> {
        if !self.visible(id) {
            return Err(Fail::Unknown);
        }
        // 先只读地问一遍（借用到此为止），再决定要不要动树。
        let (pie, view) = match self.slot(id) {
            None => return Err(Fail::Unknown),
            Some(slot) => match &slot.node {
                Node::Pane(_) => return Err(Fail::NotATile),
                Node::Tile { pie, .. } => (*pie, slot.view),
            },
        };
        // **判的是"这一枚还能不能交出去"，与它是哪种资源无关。** 从前这里借 `vested_by`
        // 代理（那一问的正文是 `Reserve`，而 `Reserve` **只认孔**：对页与铃答 `Denied`）
        // ⇒ 页与铃当门牌时一律被判死，还顺手把那一格剔掉（真机量到：`land=Ok` 而
        // `find=Err(Dead)`，四枚砖全一样）。`Alive` 那一格答的正是这件事实。
        if !alive(pie) {
            let _ = self.unlink(id);
            let _ = pie::release(pie, env::ReleaseMode::Revoke);
            return Err(Fail::Dead);
        }
        ship(pie, view);
        Ok(())
    }

    /// **开**：第 `id` 格**是谁的门牌**（那一枚句柄的开者）
    /// 与 Operator::find **正相反**：这一条什么都不交出去——只答"是谁"，不把那枚句柄递出去
    /// 三格，与 `find` 对同一格答**同一批码**（差别只在"活着的砖"那一档：`find` 交出句柄
    /// - 号不在树上（墓碑 / 从没铸过）⇒ Fail::Unknown
    /// - 那一格是一块 `Pane` ⇒ Fail::NotATile（**没有开者这一说**）
    /// - 是一枚 `Tile`，但开者那扇门封印了 ⇒ Fail::Dead
    pub fn opens(&self, id: EntryId) -> Result<TaskId, Fail> {
        let pie = match self.slot(id) {
            None => return Err(Fail::Unknown),
            Some(slot) => match &slot.node {
                Node::Pane(_) => return Err(Fail::NotATile),
                Node::Tile { pie, .. } => *pie,
            },
        };
        opened_by(pie).ok_or(Fail::Dead)
    }

    /// **剪**：把那一号那一格剪掉
    /// 那一格得存在（否则 Fail::Unknown）；是 `Pane` 的话**必须空着**（否则 Fail::NonEmpty）
    /// 剪掉一枚 `Tile` 时放下本地引用（pie::release(_, ReleaseMode::Keep)），保留已交付的副本——它是资源实体的一份引用，不放下就漏水
    /// **剪掉的那一槽留成墓碑**（`None`），不 `remove`：号是下标，一移后面全错位。故一枚剪过的
    /// 号从此答 Fail::Unknown，而**它不会被重新铸出来**（水位只增）
    /// 答**真动了什么**：剪了"一枚空的 `Pane`"与"一枚 `Tile`"都算真动了树（前者是一格没了，
    /// 后者是一格连资源一起放下）——两档订阅者都该知道"这一号从此不在了"。剪不动的那两格
    /// （号不在 / 那块窗格非空）是 `Err`，与"没变"不是一件事。
    fn trim(&mut self, id: EntryId) -> Result<Option<Change>, Fail> {
        let dropped = match self.slot(id) {
            None => return Err(Fail::Unknown),
            Some(slot) => match &slot.node {
                Node::Pane(inner) if inner.is_empty() => None,
                Node::Pane(_) => return Err(Fail::NonEmpty),
                Node::Tile { pie, .. } => Some(*pie),
            },
        };
        // **路要在剪之前记**：剪完那一槽是碑，`road_to` 再也走不出来。
        let road = self.road_to(id);
        if let Some(pie) = dropped {
            if pies().any(|p| p.token == pie) {
                pie::release(pie, env::ReleaseMode::Keep).map_err(|_| Fail::Unknown)?;
            }
        }
        let _ = self.unlink(id);
        // 剪了"一枚空的 `Pane`"与"一枚 `Tile`"都算真动了树：前者是一格没了，后者是一格连资源
        // 一起放下——两档订阅者都该知道"这一号从此不在了"。**路在落那条事件时现走**（号还在，
        // 槽已空）——故 `Change` 不存路。
        Ok(Some(Change {
            kind: Kind::Trimmed,
            id,
            owner: TaskId::new(0),
            road,
        }))
    }

    /// **列**：看那一块 `Pane` 里有哪些**号**（Where::Root = 根那一层）
    /// 号不在 ⇒ Fail::Unknown；那一号是一枚 `Tile` ⇒ Fail::NotAPane。**不过问死活**
    /// 剔死是 Operator::find 那一路上的事
    /// 答的是号、不是名字（机器用号，人用名——名字另问 Operator::name）
    /// 顺序是**号序**：pane 里本来是登记序，而号单调 ⇒ 两者一致，不必额外排
    pub fn list(&self, at: Where) -> Result<impl Iterator<Item = EntryId> + '_, Fail> {
        if matches!(at, Where::At(id) if !self.visible(id)) {
            return Err(Fail::Unknown);
        }
        Ok(self
            .kids(at)?
            .iter()
            .copied()
            .filter(|id| self.visible(*id)))
    }

    pub fn seek(&self, road: &Path) -> Result<EntryId, Fail> {
        let prefix = road.parent().ok_or(Fail::Unknown)?;
        let last = road.file_name().ok_or(Fail::Unknown)?;
        let mut level: &[EntryId] = &self.root;
        for step in prefix.iter() {
            let child = self.child(level, step).ok_or(Fail::Unknown)?;
            level = match &self.slot(child).ok_or(Fail::Unknown)?.node {
                Node::Pane(inner) => inner,
                Node::Tile { .. } => return Err(Fail::NotAPane),
            };
        }
        self.child(level, last).ok_or(Fail::Unknown)
    }

    /// **名**：这枚号此刻叫什么
    pub fn name(&self, id: EntryId) -> Result<String, Fail> {
        if !self.visible(id) {
            return Err(Fail::Unknown);
        }
        self.slot(id)
            .map(|slot| slot.name.clone())
            .ok_or(Fail::Unknown)
    }

    pub fn reference(&self, id: EntryId) -> Option<PieToken> {
        match &self.slot(id)?.node {
            Node::Tile { pie, .. } => Some(*pie),
            Node::Pane(_) => None,
        }
    }
    pub fn permit(&self, id: EntryId) -> Permit {
        match self.slot(id) {
            Some(Slot {
                node: Node::Tile { permit, .. },
                ..
            }) => *permit,
            _ => Permit::Public,
        }
    }

    /// **这一格此刻能不能重落**：无主 ⇒ 能；有主 ⇒ 只有主人自己，或**那一枚已经不在了**
    /// （`establish::alive`：不在表里 / 已封印）才轮得到别人。
    ///
    /// **判据与 `find` 同一格**（"这一枚还能不能交出去"与"这一格能不能换人"是同一件事）：
    /// 从前这里借 `vested_by`（孔那一套的父手），于是**页与铃那两格恒"可重落"**——任何域都能
    /// 把别人的门牌换绑成自己的。
    pub fn claimable(&self, key: Key<'_>, who: TaskId) -> bool {
        let Some(Slot {
            node: Node::Tile { pie, owner, .. },
            ..
        }) = self.slot_at(key)
        else {
            return true;
        };
        match owner {
            None => true,
            Some(owner) => *owner == who || !alive(*pie),
        }
    }

    /// 落 / 分共用的那一手：在 `at` 那一块 `Pane` 里给 `name` 立一格（或换绑那一格）
    /// **答那一格自己的号**：换绑不动号，故只有"真铸了一格"才动水位。**不递归**——
    /// `at` 那一块 `Pane` 就是一次 Operator::kids
    /// **先只读地问一遍**（那一格叫什么号），再动手：这样动手那一段只需要一次
    /// `slots[i]` 的可变借用，不必在 children 里穿一层 `&mut`（那正是老一版递归的由头）
    fn put(&mut self, placement: Placement) -> Result<EntryId, Fail> {
        let Placement {
            location,
            node,
            meta,
        } = placement;
        let PlacementMeta {
            authority,
            view,
            resource_id,
        } = meta;
        let Location { at, name } = location;
        let want = if matches!(node, Node::Tile { .. }) {
            Want::Tile
        } else {
            Want::Pane
        };
        let existing = self
            .kids(at)?
            .iter()
            .copied()
            .find(|child| self.slot(*child).is_some_and(|slot| slot.name == name));
        match existing {
            Some(id) => {
                if !self.visible(id) {
                    return Err(Fail::Denied);
                }
                // 已占。**换绑不动号**：那一格还是同一格——答的就是它那个号。
                let Some(slot) = self.slot(id) else {
                    return Err(Fail::Unknown);
                };
                if want == Want::Pane && matches!(slot.node, Node::Pane(_)) {
                    return Ok(id);
                }
                let old = match &slot.node {
                    Node::Tile { pie: old, .. } => Some(*old),
                    Node::Pane(inner) if inner.is_empty() => None,
                    Node::Pane(_) => return Err(Fail::NonEmpty),
                };
                if let Some(old) = old {
                    if pies().any(|p| p.token == old) {
                        pie::release(old, env::ReleaseMode::Keep).map_err(|_| Fail::Unknown)?;
                    }
                }
                let revision = self.next_revision.checked_add(1).ok_or(Fail::Full)?;
                self.next_revision = revision;
                let slot = self
                    .slots
                    .get_mut(id.get())
                    .and_then(Option::as_mut)
                    .ok_or(Fail::Unknown)?;
                slot.node = node;
                slot.revision = revision;
                slot.authority = authority;
                slot.view = view;
                Ok(id)
            }
            None => {
                if self.kids(at)?.len() >= PANE_CAP {
                    return Err(Fail::Full);
                }
                // **先要位、再落格**：条数那一闸管的是`PANE_CAP`，这两行管**内存**。
                // 少了它们，分配失败走的是 `handle_alloc_error`（abort）——而同一句"备不下就
                // 如实报"在仓里另外一处是 `try_reserve → Full`：Desk::admit
                // （`programs/src/system/common/face/desk.rs`）。**同一句话，两处一个纪律**
                self.slots.try_reserve(1).map_err(|_| Fail::Full)?;
                self.kids_mut(at)?.try_reserve(1).map_err(|_| Fail::Full)?;
                let fresh = EntryId::new(self.slots.len());
                let revision = self.next_revision.checked_add(1).ok_or(Fail::Full)?;
                self.next_revision = revision;
                self.slots.push(Some(Slot {
                    name,
                    node,
                    revision,
                    authority,
                    view,
                    resource_id,
                }));
                self.kids_mut(at)?.push(fresh);
                Ok(fresh)
            }
        }
    }

    /// 这一块 `Pane` 的孩子（Where::Root = 根那一层；`At(id)` = 那一格，得是 `Pane`）
    /// 三格判据与老一版的走法逐条对齐：号不在 ⇒ Fail::Unknown；那一格是 `Tile` ⇒
    fn kids(&self, at: Where) -> Result<&[EntryId], Fail> {
        match at {
            Where::Root => Ok(&self.root),
            Where::At(id) => match self.slot(id) {
                None => Err(Fail::Unknown),
                Some(slot) => match &slot.node {
                    Node::Pane(inner) => Ok(inner),
                    Node::Tile { .. } => Err(Fail::NotAPane),
                },
            },
        }
    }

    fn kids_mut(&mut self, at: Where) -> Result<&mut Vec<EntryId>, Fail> {
        match at {
            Where::Root => Ok(&mut self.root),
            Where::At(id) => match self.slots.get_mut(id.get()).and_then(Option::as_mut) {
                None => Err(Fail::Unknown),
                Some(slot) => match &mut slot.node {
                    Node::Pane(inner) => Ok(inner),
                    Node::Tile { .. } => Err(Fail::NotAPane),
                },
            },
        }
    }

    /// 按号取那一格（只读）。号不在 / 是墓碑 ⇒ `None`
    fn slot(&self, id: EntryId) -> Option<&Slot> {
        self.slots.get(id.get()).and_then(Option::as_ref)
    }

    /// 按 Key 那两把钥匙取那一格（只读）。两种寻址**打的是同一格**（见 Key）
    fn slot_at(&self, key: Key<'_>) -> Option<&Slot> {
        match key {
            Key::Id(id) => self.slot(id),
            Key::At(at, name) => self.slot(self.child(self.kids(at).ok()?, name)?),
        }
    }

    /// 在这一块 `Pane` 的孩子里按名字找那个号（只读一趟扫，最多 `PANE_CAP` 次查表）
    fn child(&self, level: &[EntryId], name: &str) -> Option<EntryId> {
        level
            .iter()
            .copied()
            .find(|id| self.visible(*id) && self.slot(*id).is_some_and(|slot| slot.name == name))
    }

    fn visible(&self, id: EntryId) -> bool {
        self.slot(id).is_some_and(|slot| {
            slot.authority
                .is_none_or(|authority| self.authority.live(authority))
        })
    }

    /// 按号把那**一格拿走**：槽标成墓碑（`None`），再把它从**某一个**父的 children 里摘掉
    fn unlink(&mut self, id: EntryId) -> Option<Slot> {
        let taken = self.slots.get_mut(id.get())?.take()?;
        if let Some(at) = self.root.iter().position(|child| *child == id) {
            self.root.remove(at);
            return Some(taken);
        }
        for i in 0..self.slots.len() {
            let Some(Some(one)) = self.slots.get_mut(i) else {
                continue;
            };
            if let Node::Pane(inner) = &mut one.node
                && let Some(at) = inner.iter().position(|child| *child == id)
            {
                inner.remove(at);
                break;
            }
        }
        Some(taken)
    }

    /// **从根写起的那条路**：`id` 那一格自己那一段 ＋ 一路向上的每一段。
    ///
    /// 表里每一格只记**自己那一段名**（路那一层不在槽上，见 `Slot` 的注），故要整条路只能
    /// 从根走下来找它——`PANE_CAP` 与 `Path::MAX` 都是小常数，这条走法每层最多扫一趟。
    ///
    /// 返 `None` = 这一号不在树上（碑 / 从没铸过）：**事件里那条路拼不出来就不发**，
    /// 比发一条路是假的强。
    pub fn road_to(&self, id: EntryId) -> Option<PathBuf> {
        let mut segs: Vec<String> = Vec::new();
        let mut want = id;
        loop {
            // 找"哪一层的孩子里有 `want`"，把那一层的**父**继续往上带。
            let (parent, name) = self.locate(want)?;
            segs.try_reserve(1).ok()?;
            segs.push(name);
            match parent {
                Where::Root => break,
                Where::At(up) => want = up,
            }
        }
        // 从根写起 ⇒ 反着拼。
        let mut road = PathBuf::from(Path::ROOT);
        for seg in segs.iter().rev() {
            let next = road.try_join(seg.as_str())?;
            road = next;
        }
        Some(road)
    }

    /// `id` 那一格住在哪：**父那一块 `Pane`（根那一层报 `Where::Root`）＋ 它自己那一段名**。
    fn locate(&self, id: EntryId) -> Option<(Where, String)> {
        if self.root.contains(&id) {
            let name = self.slot(id)?.name.clone();
            return Some((Where::Root, name));
        }
        for i in 0..self.slots.len() {
            let Some(Some(one)) = self.slots.get(i) else {
                continue;
            };
            let Node::Pane(inner) = &one.node else {
                continue;
            };
            if inner.contains(&id) {
                let name = self.slot(id)?.name.clone();
                return Some((Where::At(EntryId::new(i)), name));
            }
        }
        None
    }
}

// 真正的对照表只有一份（system_api::operator::frame）；`gate` 为了"不带载体"自己
// 看得见 `gate` 与 `frame`。
const _: () = {
    use system_api::operator::frame;
    assert!(gate::WIRE_OK == frame::OK);
    assert!(gate::WIRE_DENIED == frame::DENIED);
    assert!(gate::WIRE_UNJUDGED == frame::UNJUDGED);
};
