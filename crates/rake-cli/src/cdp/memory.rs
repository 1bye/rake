//! Bounded Chrome allocation profiles with stable native stack identities.

use super::wire::Error;
use rake_protocol::{control::Command, inspection::memory::Snapshot};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

#[derive(Default)]
pub(super) struct Memory {
    pub owner: Option<String>,
    pub active: bool,
    pub interval: u64,
    pub include_freed: bool,
    pub renewed: Option<Instant>,
    ids: BTreeMap<Vec<String>, u32>,
    generation: Option<u64>,
    pub metadata: Value,
}

impl Memory {
    pub fn begin(&mut self, params: &Value) -> Result<Command, Error> {
        if self.active {
            return Err(Error::new(
                -32000,
                "Native memory recording is already active",
            ));
        }
        let interval = params
            .get("samplingInterval")
            .map_or(Some(32768), Value::as_u64)
            .filter(|interval| (256..=16 * 1024 * 1024).contains(interval))
            .ok_or_else(|| {
                Error::new(
                    -32602,
                    "samplingInterval must be an integer from 256 to 16777216",
                )
            })?;
        let mut include_freed = false;
        for name in [
            "includeObjectsCollectedByMajorGC",
            "includeObjectsCollectedByMinorGC",
        ] {
            if let Some(value) = params.get(name) {
                include_freed |= value
                    .as_bool()
                    .ok_or_else(|| Error::new(-32602, format!("{name} must be boolean")))?;
            }
        }
        if let Some(depth) = params.get("stackDepth")
            && depth.as_u64().is_none_or(|depth| depth == 0 || depth > 24)
        {
            return Err(Error::new(-32602, "Native stackDepth must be 1–24"));
        }
        *self = Self {
            owner: Some(uuid::Uuid::new_v4().to_string()),
            interval,
            include_freed,
            ..Self::default()
        };
        Ok(self.command(true))
    }

    pub fn command(&self, enabled: bool) -> Command {
        Command::RecordMemory {
            owner: self.owner.clone().unwrap_or_default(),
            enabled,
            interval: self.interval,
            lease_ms: if enabled { 5000 } else { 0 },
        }
    }

    pub fn profile(&mut self, capture: &Snapshot, frames: &[Value]) -> Result<Value, Error> {
        use rake_protocol::inspection::memory::*;
        if capture.samples.len() > MAX_LIVE_SAMPLES + MAX_FREED_SAMPLES
            || capture.sites.len() > MAX_SITES
            || frames.len() != capture.sites.len()
        {
            return Err(Error::new(
                -32000,
                "Native memory capture exceeds its bounds",
            ));
        }
        if self
            .generation
            .is_some_and(|generation| generation != capture.generation)
        {
            return Err(Error::new(
                -32000,
                "Native memory recording generation changed",
            ));
        }
        self.generation = Some(capture.generation);
        let mut nodes = BTreeMap::from([
            (
                1,
                Node::new(
                    json!({"functionName":"(root)","scriptId":"","url":"","lineNumber":-1,"columnNumber":-1}),
                ),
            ),
            (
                2,
                Node::new(
                    json!({"functionName":"Rust allocator (estimated bytes)","scriptId":"","url":"","lineNumber":-1,"columnNumber":-1}),
                ),
            ),
            (
                3,
                Node::new(
                    json!({"functionName":"(native frames omitted; partial capture)","scriptId":"","url":"","lineNumber":-1,"columnNumber":-1}),
                ),
            ),
        ]);
        nodes.get_mut(&1).expect("root").children.insert(2);
        let mut bytes = 2048;
        let mut omitted = capture.omitted;
        let mut samples = Vec::new();
        for sample in &capture.samples {
            if sample.stack.len() > STACK_DEPTH {
                return Err(Error::new(-32000, "Native memory stack exceeds its bounds"));
            }
            let mut parent = 2;
            let mut path = Vec::new();
            for &index in sample.stack.iter().rev() {
                let site = capture
                    .sites
                    .get(index as usize)
                    .ok_or_else(|| Error::new(-32000, "Invalid native memory frame index"))?;
                let frame = &frames[index as usize];
                path.push(site.address.clone());
                let cost = frame.to_string().len() + 128;
                let id = self.ids.get(&path).copied();
                if id.is_none() && self.ids.len() >= 4096
                    || nodes.len() >= 768
                    || bytes + cost >= 384 * 1024
                {
                    parent = 3;
                    omitted += 1;
                    nodes.get_mut(&2).expect("group").children.insert(3);
                    break;
                }
                // IDs survive branches being freed between profiles of this recording.
                let id = if let Some(id) = id {
                    id
                } else {
                    let id = self.ids.len() as u32 + 4;
                    self.ids.insert(path.clone(), id);
                    id
                };
                if let std::collections::btree_map::Entry::Vacant(entry) = nodes.entry(id) {
                    entry.insert(Node::new(frame.clone()));
                    bytes += cost;
                }
                nodes.get_mut(&parent).expect("parent").children.insert(id);
                parent = id;
            }
            nodes.get_mut(&parent).expect("sample node").size =
                nodes[&parent].size.saturating_add(sample.estimated_bytes);
            samples.push(
                json!({"size":sample.estimated_bytes,"nodeId":parent,"ordinal":sample.ordinal}),
            );
        }
        self.metadata = json!({"generation":capture.generation,"recording":capture.recording,"interval":capture.interval,"omitted":omitted,
            "liveSamples":capture.samples.iter().filter(|s|s.live).count(),"freedSamples":capture.samples.iter().filter(|s|!s.live).count(),
            "sampledLiveBytes":capture.samples.iter().filter(|s|s.live).map(|s|s.size).sum::<u64>(),
            "estimatedLiveBytes":capture.samples.iter().filter(|s|s.live).map(|s|s.estimated_bytes).sum::<u64>(),
            "measurement":"sampled Rust allocator blocks; estimates, not process RSS or an ownership graph"});
        let mut profile = json!({"head":tree(1,&nodes),"samples":samples});
        profile["rake"] = self.metadata.clone();
        Ok(profile)
    }
}

struct Node {
    frame: Value,
    size: u64,
    children: BTreeSet<u32>,
}
impl Node {
    fn new(frame: Value) -> Self {
        Self {
            frame,
            size: 0,
            children: BTreeSet::new(),
        }
    }
}
fn tree(id: u32, nodes: &BTreeMap<u32, Node>) -> Value {
    let node = &nodes[&id];
    json!({"id":id,"callFrame":node.frame,"selfSize":node.size,"children":node.children.iter().map(|id|tree(*id,nodes)).collect::<Vec<_>>()})
}

#[cfg(test)]
mod tests {
    use super::*;
    use rake_protocol::inspection::memory::{Sample, Site};
    #[test]
    fn profiles_keep_stack_ids_across_live_changes_and_reject_invalid_captures() {
        let mut memory = Memory::default();
        assert!(memory.begin(&json!({"samplingInterval":1})).is_err());
        memory.begin(&json!({})).unwrap();
        let mut capture = Snapshot {
            generation: 1,
            recording: true,
            interval: 32768,
            omitted: 0,
            sites: vec![Site {
                address: "0x42".into(),
                function: "probe".into(),
                file: None,
                line: None,
                column: None,
            }],
            samples: vec![Sample {
                ordinal: 1,
                size: 42,
                estimated_bytes: 32768,
                live: true,
                stack: vec![0],
            }],
        };
        let frames = vec![
            json!({"functionName":"probe","scriptId":"","url":"","lineNumber":-1,"columnNumber":-1}),
        ];
        let first = memory.profile(&capture, &frames).unwrap();
        assert_eq!(
            first["head"]["children"][0]["children"][0]["selfSize"],
            32768
        );
        capture.samples[0].ordinal = 2;
        let second = memory.profile(&capture, &frames).unwrap();
        assert_eq!(
            first["samples"][0]["nodeId"],
            second["samples"][0]["nodeId"]
        );
        capture.samples[0].stack = vec![2];
        assert!(memory.profile(&capture, &frames).is_err());
    }
}
