use crate::system::operator::core::Tile;
use crate::system::operator::client::Tree;
use crate::system::operator::Placement;
use crate::system::run::publication::book::{Address, Installation, Publications, Record, Source};

use super::Internal;
impl Publications {
    pub(crate) fn withdraw_internal(
        &mut self,
        tree: &mut Tree,
        entry: env::PieToken,
    ) -> Result<(), &'static str> {
        let mut at = 0;
        while at < self.records.len() {
            let record = &self.records[at];
            if record.address.target.is_none()
                && record.source.publisher == env::unit::self_id()
                && record.source.entry == entry
            {
                self.remove(tree, at)?;
            } else {
                at += 1;
            }
        }
        Ok(())
    }
    pub fn internal(
        &mut self,
        tree: &mut Tree,
        publication: &Internal,
    ) -> Result<(), &'static str> {
        let road = &publication.road;
        let entry = publication.entry;
        let (permit, publisher) = publication.access;
        let previous = self
            .records
            .iter()
            .position(|r| r.address.road.as_str() == road.as_str());
        if let Some(at) = previous {
            let r = &self.records[at];
            if r.installation.mount.is_none() {
                return Err("publication cleanup pending");
            }
            if r.source.publisher != publisher {
                return Err("internal publication requires retirement");
            }
            if r.source.entry != entry {
                return Err("internal publication conflict");
            }
            if r.source.permit == permit {
                return Ok(());
            }
            let mount = tree.mount(&Placement {
                road: (road).to_path_buf(),
                tile: Tile {
                    pie: entry,
                    permit,
                    owner: Some(publisher),
                },
                replace: true,
            })?;
            self.records[at].installation.mount = Some(mount);
            self.records[at].source.permit = permit;
            return Ok(());
        }
        self.records
            .try_reserve(1)
            .map_err(|_| "publication capacity")?;
        let mount = tree.mount(&Placement {
            road: (road).to_path_buf(),
            tile: Tile {
                pie: entry,
                permit,
                owner: Some(publisher),
            },
            replace: false,
        })?;
        self.records.push(Record {
            address: Address {
                road: road.to_path_buf(),
                target: None,
            },
            source: Source {
                publisher,
                entry,
                permit,
            },
            installation: Installation {
                owner: publisher,
                mount: Some(mount),
            },
        });
        Ok(())
    }
}
