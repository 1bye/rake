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
        // Preserve the biggest byte contributors when the call-tree budget fills.
        let mut priority: Vec<_> = capture.samples.iter().collect();

        priority.sort_by_key(|sample| {
            (
                std::cmp::Reverse(sample.estimated_bytes),
                std::cmp::Reverse(sample.live),
                sample.ordinal,
            )
        });

        for sample in priority {
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

        samples.sort_by_key(|sample| sample["ordinal"].as_u64().unwrap_or(0));
        self.metadata = json!({"generation":capture.generation,"recording":capture.recording,"interval":capture.interval,"omitted":omitted,
            "limits":{"liveSamples":MAX_LIVE_SAMPLES,"freedSamples":MAX_FREED_SAMPLES,"sites":MAX_SITES,"stackDepth":STACK_DEPTH,"treeNodes":768,"stackIdentities":4096,"treeBudgetBytes":384*1024,"snapshotBytes":512*1024},
            "freedHistoryOmitted":capture.freed_omitted,
            "liveSamples":capture.samples.iter().filter(|s|s.live).count(),"freedSamples":capture.samples.iter().filter(|s|!s.live).count(),
            "sampledLiveBytes":capture.samples.iter().filter(|s|s.live).map(|s|s.size).sum::<u64>(),
            "estimatedLiveBytes":capture.samples.iter().filter(|s|s.live).map(|s|s.estimated_bytes).sum::<u64>(),
            "measurement":"sampled Rust allocator blocks; estimates, not process RSS or an ownership graph"});
        let mut profile = json!({"head":tree(1,&nodes),"samples":samples});

        profile["rake"] = self.metadata.clone();

        Ok(profile)
    }

    pub fn heap_snapshot(&mut self, capture: &Snapshot, frames: &[Value]) -> Result<String, Error> {
        let _ = self.profile(capture, frames)?;

        let live: Vec<_> = capture
            .samples
            .iter()
            .filter(|sample| sample.live)
            .collect();

        let title = format!(
            "Rake sampled Rust allocations · generation {} · estimated bytes · {} omissions · no ownership graph",
            capture.generation, self.metadata["omitted"]
        );

        let mut strings = vec![title];
        let mut names = BTreeMap::new();

        let mut nodes = vec![
            json!(9),
            json!(0),
            json!(1),
            json!(0),
            json!(live.len()),
            json!(0),
            json!(0),
        ];

        let mut edges = Vec::new();

        for (index, sample) in live.iter().enumerate() {
            let frame = sample
                .stack
                .iter()
                .filter_map(|index| frames.get(*index as usize))
                .find(|frame| {
                    frame["url"].as_str().is_some_and(|url| !url.is_empty())
                        && !frame["functionName"]
                            .as_str()
                            .unwrap_or("")
                            .contains("__rust_alloc")
                });

            let name = frame.map_or_else(
                || "Rust allocation · unresolved native caller".to_owned(),
                |frame| {
                    format!(
                        "Rust allocation · {}",
                        frame["functionName"].as_str().unwrap_or("unknown")
                    )
                },
            );

            let name = *names.entry(name.clone()).or_insert_with(|| {
                strings.push(name);

                strings.len() - 1
            });

            let id = sample
                .ordinal
                .checked_mul(2)
                .and_then(|id| id.checked_add(1))
                .filter(|id| *id < (1u64 << 53))
                .ok_or_else(|| {
                    Error::new(
                        -32000,
                        "Native allocation identity exceeds Chrome's numeric range",
                    )
                })?;

            nodes.extend([
                json!(8),
                json!(name),
                json!(id),
                json!(sample.estimated_bytes),
                json!(0),
                json!(0),
                json!(0),
            ]);
            edges.extend([json!(1), json!(index), json!((index + 1) * 7)]);
        }

        let header = json!({"meta":{
            "node_fields":["type","name","id","self_size","edge_count","trace_node_id","detachedness"],
            "node_types":[["hidden","array","string","object","code","closure","regexp","number","native","synthetic","concatenated string","sliced string","symbol","bigint"],"string","number","number","number","number","number"],
            "edge_fields":["type","name_or_index","to_node"],"edge_types":[["context","element","property","internal","hidden","shortcut","weak"],"string_or_number","node"],
            "trace_function_info_fields":[],"trace_node_fields":[],"sample_fields":[],"location_fields":[]},
            "node_count":live.len()+1,"edge_count":live.len(),"trace_function_count":0});

        // Chrome streams fields in V8 order and requires strings to be the final array.
        let text = serde_json::to_string(&HeapFile {
            snapshot: header,
            nodes,
            edges,
            trace_function_infos: [],
            trace_tree: [],
            samples: [],
            locations: [],
            rake: &self.metadata,
            strings,
        })
        .map_err(|error| Error::new(-32000, error.to_string()))?;

        if text.len() > 512 * 1024 {
            return Err(Error::new(
                -32000,
                "Native heap snapshot exceeds the export budget",
            ));
        }

        Ok(text)
    }
}

// Field order is part of Chrome's streaming snapshot format.
#[derive(serde::Serialize)]
struct HeapFile<'a> {
    snapshot: Value,
    nodes: Vec<Value>,
    edges: Vec<Value>,
    trace_function_infos: [Value; 0],
    trace_tree: [Value; 0],
    samples: [Value; 0],
    locations: [Value; 0],
    rake: &'a Value,
    strings: Vec<String>,
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
    fn large_allocations_survive_a_full_call_tree_budget() {
        let sites: Vec<_> = (0..256)
            .map(|index| Site {
                address: format!("0x{index:x}"),
                function: if index == 255 {
                    "large_probe".into()
                } else {
                    format!("small_{index}")
                },
                file: None,
                line: None,
                column: None,
            })
            .collect();

        let frames:Vec<_>=sites.iter().map(|site|json!({"functionName":site.function,"scriptId":"","url":"","lineNumber":-1,"columnNumber":-1})).collect();

        let mut samples: Vec<_> = (0..512)
            .map(|index| Sample {
                ordinal: index + 1,
                size: 64,
                estimated_bytes: 32768,
                live: false,
                stack: vec![
                    (index % 255) as u16,
                    ((index / 255) % 255) as u16,
                    ((index + 37) % 255) as u16,
                ],
            })
            .collect();

        samples.push(Sample {
            ordinal: 1000,
            size: 8 * 1024 * 1024,
            estimated_bytes: 8 * 1024 * 1024,
            live: true,
            stack: vec![255],
        });
        let capture = Snapshot {
            generation: 1,
            recording: true,
            interval: 32768,
            omitted: 0,
            freed_omitted: 0,
            sites,
            samples,
        };

        let profile = Memory::default().profile(&capture, &frames).unwrap();

        assert!(profile["rake"]["omitted"].as_u64().unwrap() > 0);
        assert!(
            profile["head"]["children"][0]["children"]
                .as_array()
                .unwrap()
                .iter()
                .any(|node| node["callFrame"]["functionName"] == "large_probe"
                    && node["selfSize"] == 8 * 1024 * 1024)
        );
    }

    #[test]
    fn heap_projections_preserve_live_identities_for_comparison() {
        let mut memory = Memory::default();

        let mut capture = Snapshot {
            generation: 1,
            recording: true,
            interval: 32768,
            omitted: 0,
            freed_omitted: 0,
            sites: vec![Site {
                address: "0x42".into(),
                function: "probe".into(),
                file: None,
                line: None,
                column: None,
            }],
            samples: vec![Sample {
                ordinal: 123,
                size: 1024,
                estimated_bytes: 32768,
                live: true,
                stack: vec![0],
            }],
        };

        let frames = vec![
            json!({"functionName":"probe","scriptId":"","url":"http://localhost/probe.rs","lineNumber":1,"columnNumber":0}),
        ];

        let text = memory.heap_snapshot(&capture, &frames).unwrap();
        let mut cursor = 0;

        for key in [
            "snapshot",
            "nodes",
            "edges",
            "samples",
            "locations",
            "strings",
        ] {
            cursor += text[cursor..]
                .find(&format!("\"{key}\":"))
                .expect("Chrome streaming field order");
        }

        assert!(text.ends_with("]}"), "strings must be the final array");
        let first: Value = serde_json::from_str(&text).unwrap();

        let second: Value =
            serde_json::from_str(&memory.heap_snapshot(&capture, &frames).unwrap()).unwrap();

        assert_eq!(first["nodes"][9], 247);
        assert_eq!(first["nodes"], second["nodes"]);
        assert_eq!(first["edges"], json!([1, 0, 7]));
        capture.samples[0].live = false;
        let freed: Value =
            serde_json::from_str(&memory.heap_snapshot(&capture, &frames).unwrap()).unwrap();

        assert_eq!(freed["snapshot"]["node_count"], 1);
        assert_eq!(freed["nodes"].as_array().unwrap().len(), 7);
    }

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
            freed_omitted: 0,
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
