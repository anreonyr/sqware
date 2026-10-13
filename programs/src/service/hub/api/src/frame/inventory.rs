use env::wire::Field as _;
use env::{Mark, Name, Permission, PieKind, TaskId};

/// Maximum machine leaves plus the two boot resources under one bounded lease.
pub const INVENTORY_MAX: usize = 64;
const SLOT_BASE: u64 = 0x4855_4200_0000_0000;
const ITEM_LEN: usize = 16 + 8 + 1 + 4;

/// Stable target-local slot for an item in the machine's canonical inventory order.
pub const fn inventory_slot(index: usize) -> Option<Mark> {
    if index < INVENTORY_MAX {
        Some(Mark::new(SLOT_BASE + index as u64))
    } else {
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InventoryItem {
    pub resource: Name,
    pub owner: TaskId,
    pub kind: PieKind,
    pub permission: Permission,
}

impl InventoryItem {
    const EMPTY: Self = Self {
        resource: Name::Page(env::Page::Dtb),
        owner: TaskId::new(0),
        kind: PieKind::Pole,
        permission: Permission::empty(),
    };
}

/// Read-only inventory; item order is DTB, supervisor interrupt, sorted FDT devices.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Inventory {
    count: u8,
    items: [InventoryItem; INVENTORY_MAX],
}

impl Inventory {
    pub const fn header(count: usize) -> Option<u8> {
        if count <= INVENTORY_MAX {
            Some(count as u8)
        } else {
            None
        }
    }

    pub const fn len(self) -> usize {
        self.count as usize
    }

    pub fn of(items: &[InventoryItem]) -> Option<Self> {
        if items.len() > INVENTORY_MAX
            || items.iter().enumerate().any(|(at, item)| {
                item.resource.kind() != item.kind
                    || item.permission.is_empty()
                    || items[..at]
                        .iter()
                        .any(|prior| prior.resource == item.resource)
            })
        {
            return None;
        }
        let mut stored = [InventoryItem::EMPTY; INVENTORY_MAX];
        stored.get_mut(..items.len())?.copy_from_slice(items);
        Some(Self {
            count: items.len() as u8,
            items: stored,
        })
    }

    pub fn item(self, index: usize) -> Option<InventoryItem> {
        (index < self.len()).then(|| self.items[index])
    }

    pub const fn encoded_len_for(count: usize) -> Option<usize> {
        if count <= INVENTORY_MAX {
            Some(1 + count * ITEM_LEN)
        } else {
            None
        }
    }

    pub const fn encoded_len(self) -> usize {
        1 + self.len() * ITEM_LEN
    }

    pub fn store(self, out: &mut [u8]) -> Option<usize> {
        let len = self.encoded_len();
        let target = out.get_mut(..len)?;
        target[0] = self.count;
        let mut at = 1;
        for item in &self.items[..self.len()] {
            item.resource.store(&mut target[at..at + 16]);
            at += 16;
            target[at..at + 8].copy_from_slice(&(item.owner.get() as u64).to_le_bytes());
            at += 8;
            target[at] = item.kind as u8;
            at += 1;
            target[at..at + 4].copy_from_slice(&item.permission.bits().to_le_bytes());
            at += 4;
        }
        Some(len)
    }

    pub fn fetch(bytes: &[u8]) -> Option<Self> {
        let count = usize::from(*bytes.first()?);
        if count > INVENTORY_MAX || bytes.len() != 1 + count * ITEM_LEN {
            return None;
        }
        let mut items = [InventoryItem::EMPTY; INVENTORY_MAX];
        let mut at = 1;
        for item in &mut items[..count] {
            item.resource = Name::fetch(bytes.get(at..at + 16)?)?;
            at += 16;
            let owner =
                usize::try_from(u64::from_le_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
                    .ok()?;
            item.owner = TaskId::new(owner);
            at += 8;
            item.kind = PieKind::of(*bytes.get(at)?)?;
            at += 1;
            item.permission =
                Permission::from_bits(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))?;
            at += 4;
        }
        let inventory = Self {
            count: count as u8,
            items,
        };
        (Self::of(&inventory.items[..count]) == Some(inventory)).then_some(inventory)
    }
}
