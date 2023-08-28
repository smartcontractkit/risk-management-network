use crate::afn_contract::TaggedRoot;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
struct InflightRootCacheWithCustomTime {
    inflight_time: Duration,
    inflight_roots: HashMap<TaggedRoot, Instant>,
}

impl InflightRootCacheWithCustomTime {
    pub fn new(inflight_time: Duration) -> Self {
        Self {
            inflight_time,
            inflight_roots: HashMap::new(),
        }
    }

    fn prune(&mut self, now: Instant) {
        self.inflight_roots
            .retain(|_root, &mut entry_time| now.duration_since(entry_time) <= self.inflight_time);
    }

    pub fn add(&mut self, now: Instant, root: TaggedRoot) {
        self.inflight_roots.insert(root, now);
    }

    pub fn contains(&mut self, now: Instant, tagged_root: TaggedRoot) -> bool {
        self.prune(now);
        self.inflight_roots.contains_key(&tagged_root)
    }
}

pub struct InflightRootCache(InflightRootCacheWithCustomTime);
impl InflightRootCache {
    pub fn new(inflight_time: Duration) -> Self {
        Self(InflightRootCacheWithCustomTime::new(inflight_time))
    }

    pub fn add(&mut self, root: TaggedRoot) {
        self.0.add(Instant::now(), root);
    }

    pub fn contains(&mut self, tagged_root: TaggedRoot) -> bool {
        self.0.contains(Instant::now(), tagged_root)
    }
}
