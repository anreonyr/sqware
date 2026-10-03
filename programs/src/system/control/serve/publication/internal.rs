use crate::system::control::core::publication::{
    Address, Installation, Publications, Record, Source,
};
use crate::system::operator::core::Tile;
use crate::system::operator::serve::install::Tree;
use crate::system::operator::serve::plate::Placement;

use super::Internal;
impl Publications {
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
