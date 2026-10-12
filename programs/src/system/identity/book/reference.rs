use system_api::identity::{Fail, IdentityName, IdentityObject, OperatorEntryRef};

use super::IdentityBook;

pub struct Association {
    pub object: IdentityObject,
    pub name: IdentityName,
    pub entry: OperatorEntryRef,
}

pub struct ReferenceQuery {
    pub object: IdentityObject,
    pub name: IdentityName,
}

impl IdentityBook {
    pub fn associate(&mut self, from: env::TaskId, request: Association) -> Result<(), Fail> {
        if from != self.installer || request.entry.operator.get() == 0 {
            return Err(Fail::Denied);
        }
        self.validate_object(request.object)?;
        if let Some(row) = self
            .references
            .iter_mut()
            .find(|row| row.object == request.object && row.name == request.name)
        {
            row.entry = request.entry;
            return Ok(());
        }
        if self.references.len() >= 8192 {
            return Err(Fail::Full);
        }
        self.references.try_reserve(1).map_err(|_| Fail::Full)?;
        self.references.push(super::IdentityReference {
            object: request.object,
            name: request.name,
            entry: request.entry,
        });
        Ok(())
    }

    pub fn reference(&self, query: ReferenceQuery) -> Result<Option<OperatorEntryRef>, Fail> {
        self.validate_object(query.object)?;
        Ok(self
            .references
            .iter()
            .find(|row| row.object == query.object && row.name == query.name)
            .map(|row| row.entry))
    }

    fn validate_object(&self, object: IdentityObject) -> Result<(), Fail> {
        match object {
            IdentityObject::Principal(principal) => self.principal(principal).map(|_| ()),
            IdentityObject::Coalition(coalition) => self.coalition(coalition).map(|_| ()),
        }
    }
}
