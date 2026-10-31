//! Immutable index sharing; live style details and all frontend identities stay session-local.

use rake_protocol::inspection::{Snapshot, TreeRevision};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(super) struct Captures(Mutex<VecDeque<Entry>>);

struct Entry {
    runtime: uuid::Uuid,
    revision: TreeRevision,
    snapshot: Arc<Snapshot>,
    bytes: usize,
}

impl Captures {
    pub fn find(&self, runtime: uuid::Uuid, revision: &TreeRevision) -> Option<Arc<Snapshot>> {
        self.0
            .lock()
            .ok()?
            .iter()
            .find(|entry| entry.runtime == runtime && &entry.revision == revision)
            .map(|entry| Arc::clone(&entry.snapshot))
    }

    pub fn retain(&self, runtime: uuid::Uuid, revision: &TreeRevision, snapshot: Arc<Snapshot>) {
        let Ok(bytes) = super::wire::serialized_len(snapshot.as_ref()) else {
            return;
        };
        const BUDGET: usize = rake_protocol::inspection::MAX_CAPTURE_BYTES;
        if bytes > BUDGET {
            return;
        }
        let Ok(mut captures) = self.0.lock() else {
            return;
        };
        captures.retain(|entry| entry.runtime != runtime);
        while captures.len() >= 4
            || captures.iter().map(|entry| entry.bytes).sum::<usize>() + bytes > BUDGET
        {
            captures.pop_front();
        }
        captures.push_back(Entry {
            runtime,
            revision: revision.clone(),
            snapshot,
            bytes,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rake_protocol::inspection::Status;

    fn snapshot(capture: &str, bytes: usize) -> Arc<Snapshot> {
        Arc::new(Snapshot {
            status: Status::Captured,
            capture: capture.into(),
            router_revision: 0,
            omitted: 0,
            coverage: "x".repeat(bytes),
            elements: vec![],
        })
    }

    #[test]
    fn shared_cache_isolates_runtimes_replaces_revisions_and_evicts_by_count_and_bytes() {
        let cache = Captures::default();
        let owners: Vec<_> = (0..5).map(|_| uuid::Uuid::new_v4()).collect();
        let revision = TreeRevision {
            metadata: None,
            capture: "one".into(),
            status: Status::Captured,
        };
        let first = snapshot("one", 0);
        cache.retain(owners[0], &revision, Arc::clone(&first));
        assert!(Arc::ptr_eq(
            &cache.find(owners[0], &revision).unwrap(),
            &first
        ));
        assert!(cache.find(owners[1], &revision).is_none());
        for &owner in &owners[1..] {
            cache.retain(owner, &revision, snapshot("one", 0));
        }
        assert!(cache.find(owners[0], &revision).is_none());
        cache.retain(
            owners[4],
            &TreeRevision {
                capture: "two".into(),
                ..revision.clone()
            },
            snapshot("two", 0),
        );
        assert!(cache.find(owners[4], &revision).is_none());

        let size = rake_protocol::inspection::MAX_CAPTURE_BYTES / 2;
        cache.retain(owners[0], &revision, snapshot("one", size));
        cache.retain(owners[1], &revision, snapshot("one", size));
        assert!(
            cache.find(owners[0], &revision).is_none(),
            "serialized metadata exceeds the byte budget"
        );
        assert!(cache.find(owners[1], &revision).is_some());
        cache.retain(owners[2], &revision, snapshot("one", size * 2));
        assert!(
            cache.find(owners[2], &revision).is_none(),
            "oversized entries are never retained"
        );
    }
}
