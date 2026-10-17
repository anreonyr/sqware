use env::FlatReference;

use super::RetentionId;
use super::exposure::Exposure;

pub(super) struct Memory {
    pub id: RetentionId,
    pub reference: FlatReference,
    pub exposure: Exposure,
}
