//! Product-level logical namespace keys. The Operator mechanism stays layout agnostic.

use crate::operator::NamespaceKey;

pub const SERVICE: NamespaceKey = NamespaceKey::of("service");
pub const DEVICE: NamespaceKey = NamespaceKey::of("device");
pub const IDENTITY: NamespaceKey = NamespaceKey::of("identity");
pub const UNIT: NamespaceKey = NamespaceKey::of("unit");
