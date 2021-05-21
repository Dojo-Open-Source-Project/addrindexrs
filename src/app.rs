use bitcoin::hash_types::BlockHash;
use std::sync::{Arc, Mutex};

use crate::{daemon, errors::*, index, signal::Waiter, store};

//
// Application
//
pub struct App {
    store: store::DBStore,
    index: index::Index,
    daemon: daemon::Daemon,
    tip: Mutex<BlockHash>,
}

impl App {
    pub fn new(
        store: store::DBStore,
        index: index::Index,
        daemon: daemon::Daemon
    ) -> Result<Arc<App>> {
        Ok(Arc::new(App {
            store,
            index,
            daemon: daemon.reconnect()?,
            tip: Mutex::new(BlockHash::default()),
        }))
    }

    fn write_store(&self) -> &impl store::WriteStore {
        &self.store
    }

    // TODO: use index for queries.
    pub fn read_store(&self) -> &dyn store::ReadStore {
        &self.store
    }

    pub fn index(&self) -> &index::Index {
        &self.index
    }

    pub fn daemon(&self) -> &daemon::Daemon {
        &self.daemon
    }

    pub fn update(&self, signal: &Waiter) -> Result<bool> {
        let mut tip = self.tip.lock().expect("failed to lock tip");
        let new_block = *tip != self.daemon().getbestblockhash()?;
        if new_block {
            *tip = self.index().update(self.write_store(), &signal)?;
        }
        Ok(new_block)
    }
}
