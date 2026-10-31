//! Immutable index sharing; live style details and all frontend identities stay session-local.

use rake_protocol::inspection::{Snapshot, TreeRevision};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) struct Captures(Mutex<VecDeque<(uuid::Uuid, Arc<Snapshot>, usize)>>);

impl Captures {
    pub fn find(&self, runtime: uuid::Uuid, revision: &TreeRevision) -> Option<Arc<Snapshot>> {
        self.0
            .lock()
            .ok()?
            .iter()
            .find(|(owner, snapshot, _)| {
                *owner == runtime
                    && snapshot.capture == revision.capture
                    && snapshot.status == revision.status
            })
            .map(|(_, snapshot, _)| Arc::clone(snapshot))
    }

    pub fn retain(&self, runtime: uuid::Uuid, snapshot: Arc<Snapshot>) {
        let Ok(bytes) = serde_json::to_vec(snapshot.as_ref()).map(|bytes| bytes.len()) else {
            return;
        };
        const BUDGET: usize = rake_protocol::inspection::MAX_CAPTURE_BYTES;
        if bytes > BUDGET {
            return;
        }
        let Ok(mut captures) = self.0.lock() else {
            return;
        };
        captures.retain(|(owner, _, _)| *owner != runtime);
        while captures.len() >= 4
            || captures.iter().map(|(_, _, size)| size).sum::<usize>() + bytes > BUDGET
        {
            captures.pop_front();
        }
        captures.push_back((runtime, snapshot, bytes));
    }
}
