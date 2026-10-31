//! Bounded frame trace recording and UTF-8 IO streams for Chrome Performance.

use super::wire::Error;
use rake_protocol::inspection::Frames;
use serde_json::{Value, json};

const MAX_TRACE_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Trace {
    pub active: bool,
    pub cursor: u64,
    pub loss: bool,
    stream_mode: bool,
    events: Vec<Value>,
    bytes: usize,
    stream: Option<(String, usize)>,
    initial_dropped: Option<u64>,
    generation: u64,
    handle: String,
    started: Option<std::time::Instant>,
    analysis: Analysis,
    pub source_prefix: Option<String>,
    pub causal: super::causal::Causal,
}

impl Trace {
    pub fn begin(&mut self, params: &Value) -> Result<(), Error> {
        if self.active {
            return Err(Error::new(-32000, "Frame tracing is already active"));
        }

        let mode = params["transferMode"].as_str().unwrap_or("ReportEvents");

        if params
            .get("streamFormat")
            .is_some_and(|format| format != "json")
            || !matches!(mode, "ReportEvents" | "ReturnAsStream")
            || params.get("streamCompression").is_some_and(|v| v != "none")
        {
            return Err(Error::new(
                -32602,
                "Only uncompressed JSON frame traces are supported",
            ));
        }

        let generation = self.generation + 1;

        *self = Self {
            generation,
            handle: format!("rake-frames-{generation}"),
            started: Some(std::time::Instant::now()),
            active: true,
            stream_mode: mode == "ReturnAsStream",
            ..Self::default()
        };
        self.push(vec![
            json!({"name":"process_name","ph":"M","pid":1,"tid":1,"args":{"name":"Rake GPUI (observed wall time)"}}),
            json!({"name":"thread_name","ph":"M","pid":1,"tid":1,"args":{"name":"CrRendererMain"}}),
            json!({"name":"TracingStartedInBrowser","cat":"disabled-by-default-devtools.timeline","ph":"I","s":"t","pid":1,"tid":1,"ts":0,
                "args":{"data":{"persistentIds":true,"frames":[{"frame":"rake","processId":1,"url":"rake://runtime","name":"Rake GPUI"}]}}}),
            json!({"name":"Rake recording started","cat":"rake.recording","ph":"i","s":"g","pid":1,"tid":1,"ts":0}),
        ]);

        Ok(())
    }

    fn push(&mut self, events: Vec<Value>) {
        for event in events {
            let bytes = serde_json::to_vec(&event).map_or(MAX_TRACE_BYTES, |bytes| bytes.len()) + 1;

            if self.bytes + bytes < MAX_TRACE_BYTES - 256 * 1024 {
                self.bytes += bytes;
                self.events.push(event);
            } else {
                self.loss = true;
            }
        }
    }

    pub fn report(&mut self) -> Vec<Value> {
        if self.stream_mode {
            Vec::new()
        } else {
            self.bytes = 0;

            std::mem::take(&mut self.events)
        }
    }

    pub fn has_stream(&self) -> bool {
        self.stream.is_some()
    }

    pub fn collect(&mut self, frames: &Frames) -> Vec<Value> {
        let baseline = *self.initial_dropped.get_or_insert(frames.dropped);

        self.loss |= frames.gap || frames.dropped > baseline;
        self.cursor = frames.cursor;
        self.analysis.observe(frames);
        self.push(events(frames, self.source_prefix.as_deref()));

        self.report()
    }

    pub fn end(&mut self) -> Value {
        self.active = false;
        let elapsed = self
            .started
            .map_or(0, |start| start.elapsed().as_micros() as u64);

        let incomplete = self.causal.finish(elapsed);

        self.push(incomplete);
        self.loss |= self.causal.loss;
        self.push(self.analysis.events(elapsed));
        self.push(vec![json!({"name":"Rake recording stopped","cat":"rake.recording","ph":"i","s":"g","pid":1,"tid":1,"ts":elapsed})]);

        if self.stream_mode {
            let document = json!({"traceEvents":std::mem::take(&mut self.events),"displayTimeUnit":"ms",
                "metadata":{"clock":"recording monotonic microseconds","measurement":"GPUI draw and inclusive boundary CPU wall time; no GPU execution or CPU samples","dataLossOccurred":self.loss,"frames":self.summary(),"causal":self.causal.metadata()}});

            self.stream = Some((document.to_string(), 0));

            json!({"dataLossOccurred":self.loss,"stream":self.handle,"traceFormat":"json","streamCompression":"none"})
        } else {
            json!({"dataLossOccurred":self.loss})
        }
    }

    pub fn diagnostics(
        &mut self,
        snapshot: &rake_diagnostics::Snapshot,
        sources: &std::collections::BTreeMap<u64, String>,
    ) -> Vec<Value> {
        let events = self.causal.observe(snapshot, sources);

        self.loss |= self.causal.loss;
        self.push(events);

        self.report()
    }

    pub fn summary(&self) -> Value {
        self.analysis.summary()
    }

    pub fn read(&mut self, params: &Value) -> Result<Value, Error> {
        if params["handle"] != self.handle {
            return Err(Error::new(-32602, "Unknown trace stream"));
        }

        let (text, cursor) = self
            .stream
            .as_mut()
            .ok_or_else(|| Error::new(-32000, "Trace stream is closed"))?;

        if let Some(offset) = params.get("offset") {
            *cursor = offset
                .as_u64()
                .and_then(|v| usize::try_from(v).ok())
                .filter(|v| *v <= text.len() && text.is_char_boundary(*v))
                .ok_or_else(|| Error::new(-32602, "Invalid trace stream offset"))?;
        }

        let size = params
            .get("size")
            .map_or(Some(65536), |v| v.as_u64())
            .filter(|v| *v > 0)
            .ok_or_else(|| Error::new(-32602, "Invalid stream size"))?
            .min(65536) as usize;

        let mut end = cursor.saturating_add(size).min(text.len());

        while end > *cursor && !text.is_char_boundary(end) {
            end -= 1;
        }

        if end == *cursor && end < text.len() {
            end += text[end..]
                .chars()
                .next()
                .expect("nonempty UTF-8 tail")
                .len_utf8();
        }

        let data = text[*cursor..end].to_owned();

        *cursor = end;

        Ok(json!({"base64Encoded":false,"data":data,"eof":end == text.len()}))
    }

    pub fn close(&mut self, params: &Value) -> Result<(), Error> {
        if params["handle"] != self.handle {
            return Err(Error::new(-32000, "Trace stream expired"));
        }

        self.stream = None;

        Ok(())
    }
}

fn events(frames: &Frames, prefix: Option<&str>) -> Vec<Value> {
    let mut events = Vec::new();

    for frame in &frames.frames {
        events.push(json!({"name":"thread_name","ph":"M","pid":1,"tid":frame.window.0,"args":{"name":if frame.window.0 == 1 { "CrRendererMain".to_owned() } else { format!("GPUI window {} (logical)",frame.window.0) }}}));
        events.push(json!({"name":"RunTask","cat":"devtools.timeline","ph":"X","pid":1,"tid":frame.window.0,
            "ts":frame.elapsed_us,"dur":frame.draw_us,"args":{"data":{"kind":"GPUI window draw","frame":frame.sequence,"invalidations":frame.invalidations,"dirtyToDrawUs":frame.dirty_to_draw_us,"omittedNodeTimings":frame.omitted_nodes}}}));
        let properties = vec![
            json!(["Frame", frame.sequence.to_string()]),
            json!(["Draw (µs)", frame.draw_us.to_string()]),
            json!([
                "Dirty to draw end (µs)",
                frame
                    .dirty_to_draw_us
                    .map_or_else(|| "unavailable".into(), |us| us.to_string())
            ]),
            json!(["Invalidations", frame.invalidations.to_string()]),
            json!(["Omitted callback timings", frame.omitted_nodes.to_string()]),
            json!([
                "Reference budget (µs)",
                "16667 (60 Hz reference; not a measured deadline)"
            ]),
        ];

        events.extend(track(
            &format!("Draw #{}", frame.sequence),
            frame.elapsed_us,
            frame.draw_us,
            &format!("Window {} / Draws", frame.window.0),
            if frame.draw_us > 16_667 {
                "error"
            } else {
                "primary"
            },
            properties,
        ));

        if let Some(latency) = frame.dirty_to_draw_us {
            let end = frame.elapsed_us.saturating_add(frame.draw_us);
            let start = end.saturating_sub(latency);

            events.extend(track(
                "Dirty → draw end",
                start,
                end.saturating_sub(start),
                &format!("Window {} / Redraw latency", frame.window.0),
                "secondary",
                vec![
                    json!(["Observed latency (µs)", latency.to_string()]),
                    json!(["Start clipped to recording", (latency > end).to_string()]),
                ],
            ));
        }

        for (index, node) in frame.nodes.iter().enumerate() {
            let phase = phase(node.phase);
            let url = source_url(prefix, frame.sequence, index, node);
            let name = format!("{phase} / {}", node.name);

            events.push(json!({"name":name,"cat":"rake.callbacks","ph":"X","pid":1,"tid":frame.window.0,"ts":node.elapsed_us,"dur":node.duration_us,
                "args":{"data":{"boundary":node.name,"stableId":node.stable_id,"url":url,"lineNumber":node.source.line.saturating_sub(1),"columnNumber":node.source.column.saturating_sub(1),"inclusive":true,
                    "stackTrace":[{"functionName":format!("Rust construction: {}",node.name),"scriptId":"","url":url,"lineNumber":node.source.line.saturating_sub(1),"columnNumber":node.source.column.saturating_sub(1)}]}}}));
            events.extend(track(
                &name,
                node.elapsed_us,
                node.duration_us,
                &format!("Window {} / {phase}", frame.window.0),
                "tertiary-light",
                vec![
                    json!(["Boundary", node.name]),
                    json!([
                        "Stable identity",
                        node.stable_id.as_deref().unwrap_or("unavailable")
                    ]),
                    json!([
                        "Rust source",
                        format!(
                            "{}:{}:{}",
                            node.source.file, node.source.line, node.source.column
                        )
                    ]),
                    json!(["Source URL", url]),
                    json!([
                        "Measurement",
                        "inclusive wall time; overlaps descendant callbacks"
                    ]),
                    json!(["Frame", frame.sequence.to_string()]),
                ],
            ));
        }
    }

    events
}

fn phase(phase: rake_protocol::inspection::FramePhase) -> &'static str {
    match phase {
        rake_protocol::inspection::FramePhase::Layout => "GPUI request-layout",
        rake_protocol::inspection::FramePhase::Prepaint => "GPUI prepaint",
        rake_protocol::inspection::FramePhase::Paint => "GPUI paint",
    }
}

pub(super) fn source_url(
    prefix: Option<&str>,
    sequence: u64,
    index: usize,
    node: &rake_protocol::inspection::NodeTiming,
) -> String {
    match prefix {
        Some(prefix) => {
            let file = std::path::Path::new(&node.source.file)
                .file_name()
                .and_then(|file| file.to_str())
                .unwrap_or("source.rs");

            let file: String = file
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();

            format!("{prefix}/{sequence}/{index}/{file}")
        }
        None => super::console::source_url(&rake_diagnostics::Source {
            file: node.source.file.clone(),
            line: node.source.line,
        }),
    }
}

// Chrome's extension payload presents native observations without pretending to be browser work.
pub(super) fn track(
    name: &str,
    start: u64,
    duration: u64,
    lane: &str,
    color: &str,
    properties: Vec<Value>,
) -> Vec<Value> {
    let id = uuid::Uuid::new_v4().to_string();

    let detail = json!({"devtools":{"dataType":"track-entry","trackGroup":"Rake / observed wall time","track":lane,
        "color":color,"properties":properties,"tooltipText":name}}).to_string();

    vec![
        json!({"name":name,"cat":"blink.user_timing","ph":"b","id":id,"pid":1,"tid":1,"ts":start,"args":{"detail":detail}}),
        json!({"name":name,"cat":"blink.user_timing","ph":"e","id":id,"pid":1,"tid":1,"ts":start.saturating_add(duration),"args":{}}),
    ]
}

#[derive(Default)]
struct Analysis {
    windows: std::collections::BTreeMap<u64, Draws>,
    omitted_windows: bool,
    hotspots: std::collections::BTreeMap<String, Hotspot>,
    hotspot_bytes: usize,
    omitted_hotspots: bool,
}

#[derive(Default)]
struct Draws {
    count: u64,
    total: u64,
    max: u64,
    slow: u64,
    invalidations: u64,
    omitted: u64,
}

struct Hotspot {
    node: rake_protocol::inspection::NodeTiming,
    count: u64,
    total: u64,
    max: u64,
}

impl Analysis {
    fn observe(&mut self, frames: &Frames) {
        for frame in &frames.frames {
            for node in &frame.nodes {
                let key =
                    json!([node.stable_id, node.name, node.source, phase(node.phase)]).to_string();

                if !self.hotspots.contains_key(&key)
                    && (self.hotspots.len() >= 128 || self.hotspot_bytes + key.len() + 256 > 65536)
                {
                    self.omitted_hotspots = true;
                    continue;
                }

                let hotspot = self.hotspots.entry(key.clone()).or_insert_with(|| {
                    self.hotspot_bytes += key.len() + 256;

                    Hotspot {
                        node: node.clone(),
                        count: 0,
                        total: 0,
                        max: 0,
                    }
                });

                hotspot.count = hotspot.count.saturating_add(1);
                hotspot.total = hotspot.total.saturating_add(node.duration_us);
                hotspot.max = hotspot.max.max(node.duration_us);
            }

            if self.windows.len() == 64 && !self.windows.contains_key(&frame.window.0) {
                self.omitted_windows = true;
                continue;
            }

            let window = self.windows.entry(frame.window.0).or_default();

            window.count = window.count.saturating_add(1);
            window.total = window.total.saturating_add(frame.draw_us);
            window.max = window.max.max(frame.draw_us);
            window.slow += u64::from(frame.draw_us > 16_667);
            window.invalidations = window.invalidations.saturating_add(frame.invalidations);
            window.omitted = window.omitted.saturating_add(frame.omitted_nodes as u64);
        }
    }

    fn summary(&self) -> Value {
        let windows: Vec<Value> = self.windows.iter().map(|(id,draws)| json!({"windowId":id,"drawCount":draws.count,
            "averageDrawUs":draws.total / draws.count.max(1),"maxDrawUs":draws.max,"slowDrawCount":draws.slow,
            "invalidations":draws.invalidations,"omittedCallbackTimings":draws.omitted})).collect();

        let mut hotspots: Vec<_> = self.hotspots.values().collect();

        hotspots.sort_by_key(|sample| std::cmp::Reverse(sample.max));
        let hotspots: Vec<Value> = hotspots.into_iter().map(|sample| json!({"stableId":sample.node.stable_id,"name":sample.node.name,"source":sample.node.source,
            "phase":phase(sample.node.phase),"sampleCount":sample.count,"averageInclusiveUs":sample.total / sample.count.max(1),"maxInclusiveUs":sample.max,"inclusive":true})).collect();

        json!({"windows":windows,"referenceBudgetUs":16667,"omittedWindows":self.omitted_windows,"hotspots":hotspots,"omittedHotspots":self.omitted_hotspots})
    }

    fn events(&self, end: u64) -> Vec<Value> {
        self.windows
            .iter()
            .flat_map(|(id, draws)| {
                track(
                    &format!("Window {id} / recording summary"),
                    0,
                    end,
                    "Summary",
                    "tertiary",
                    vec![
                        json!(["Draw count", draws.count.to_string()]),
                        json!([
                            "Average draw (µs)",
                            (draws.total / draws.count.max(1)).to_string()
                        ]),
                        json!(["Maximum draw (µs)", draws.max.to_string()]),
                        json!(["Draws over 16.67 ms", draws.slow.to_string()]),
                        json!(["Omitted callback timings", draws.omitted.to_string()]),
                    ],
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(
        sequence: u64,
        elapsed_us: u64,
        draw_us: u64,
    ) -> rake_protocol::inspection::FrameTiming {
        rake_protocol::inspection::FrameTiming {
            sequence,
            window: rake_protocol::router::WindowId(1),
            elapsed_us,
            draw_us,
            dirty_to_draw_us: None,
            invalidations: 2,
            nodes: vec![],
            omitted_nodes: 0,
        }
    }

    #[test]
    fn callback_hotspots_keep_identity_source_and_inclusive_samples() {
        use rake_protocol::inspection::{FramePhase, NodeTiming, SourceLocation};
        let mut frame = sample(1, 100, 500);

        frame.nodes = vec![NodeTiming {
            stable_id: Some("stable-button".into()),
            name: "button".into(),
            source: SourceLocation {
                file: "src/view.rs".into(),
                line: 12,
                column: 4,
            },
            phase: FramePhase::Paint,
            elapsed_us: 110,
            duration_us: 300,
        }];
        let mut trace = Trace::default();

        trace.begin(&json!({})).unwrap();
        trace.source_prefix = Some("http://127.0.0.1/token/runtime/source/frame".into());
        let events = trace.collect(&Frames {
            clock: None,
            recording: true,
            frames: vec![frame.clone()],
            cursor: 1,
            more: false,
            gap: false,
            dropped: 0,
        });

        frame.sequence = 2;
        frame.nodes[0].duration_us = 100;
        trace.collect(&Frames {
            clock: None,
            recording: true,
            frames: vec![frame],
            cursor: 2,
            more: false,
            gap: false,
            dropped: 0,
        });
        let summary = trace.summary();
        let hotspot = &summary["hotspots"][0];

        assert_eq!(hotspot["stableId"], "stable-button");
        assert_eq!(hotspot["sampleCount"], 2);
        assert_eq!(hotspot["averageInclusiveUs"], 200);
        assert_eq!(hotspot["maxInclusiveUs"], 300);
        assert_eq!(hotspot["source"]["line"], 12);
        assert_eq!(hotspot["inclusive"], true);
        let event = events
            .iter()
            .find(|event| event["cat"] == "rake.callbacks")
            .unwrap();

        assert_eq!(event["args"]["data"]["stackTrace"][0]["lineNumber"], 11);
        assert_eq!(
            event["args"]["data"]["url"],
            "http://127.0.0.1/token/runtime/source/frame/1/0/view.rs"
        );
        assert_eq!(event["dur"], 300);
    }

    #[test]
    fn frame_analysis_preserves_measured_durations_and_missing_latency() {
        let mut trace = Trace::default();

        trace.begin(&json!({})).unwrap();
        let mut first = sample(1, 2000, 20000);

        first.dirty_to_draw_us = Some(30000);
        first.omitted_nodes = 3;
        let events = trace.collect(&Frames {
            clock: None,
            recording: true,
            frames: vec![first, sample(2, 30000, 10000)],
            cursor: 2,
            more: false,
            dropped: 0,
            gap: false,
        });

        let summary = trace.summary();

        assert_eq!(summary["windows"][0]["averageDrawUs"], 15000);
        assert_eq!(summary["windows"][0]["maxDrawUs"], 20000);
        assert_eq!(summary["windows"][0]["slowDrawCount"], 1);
        assert_eq!(summary["windows"][0]["omittedCallbackTimings"], 3);
        let draw = events
            .iter()
            .find(|event| event["name"] == "Draw #1" && event["ph"] == "b")
            .unwrap();

        let detail: Value = serde_json::from_str(draw["args"]["detail"].as_str().unwrap()).unwrap();

        assert_eq!(detail["devtools"]["color"], "error");
        assert_eq!(
            events
                .iter()
                .filter(|event| event["name"] == "Dirty → draw end" && event["ph"] == "b")
                .count(),
            1
        );
    }

    #[test]
    fn empty_recordings_are_loadable_and_handles_never_retarget() {
        let mut trace = Trace::default();

        trace
            .begin(&json!({"transferMode":"ReturnAsStream"}))
            .unwrap();
        assert!(trace.begin(&json!({})).is_err());
        let first = trace.end()["stream"].clone();
        let data = trace.read(&json!({"handle":first})).unwrap();
        let document: Value = serde_json::from_str(data["data"].as_str().unwrap()).unwrap();

        assert!(
            document["traceEvents"]
                .as_array()
                .unwrap()
                .iter()
                .any(|event| event["name"] == "TracingStartedInBrowser")
        );
        trace
            .begin(&json!({"transferMode":"ReturnAsStream"}))
            .unwrap();
        let second = trace.end()["stream"].clone();

        assert_ne!(first, second);
        assert!(trace.read(&json!({"handle":first})).is_err());
        assert!(trace.close(&json!({"handle":first})).is_err());
        assert!(trace.read(&json!({"handle":second})).is_ok());
    }

    #[test]
    fn streams_valid_trace_json_with_bounded_reads_and_explicit_close() {
        let mut trace = Trace::default();

        trace
            .begin(&json!({"transferMode":"ReturnAsStream"}))
            .unwrap();
        let frames = Frames {
            clock: None,
            recording: true,
            frames: Vec::new(),
            cursor: 3,
            more: false,
            gap: true,
            dropped: 1,
        };

        assert!(trace.collect(&frames).is_empty());
        let handle = trace.end()["stream"].as_str().unwrap().to_owned();

        assert!(handle.starts_with("rake-frames-"));
        let mut data = String::new();

        loop {
            let read = trace.read(&json!({"handle":handle,"size":7})).unwrap();

            data.push_str(read["data"].as_str().unwrap());

            if read["eof"] == true {
                break;
            }
        }

        let value: Value = serde_json::from_str(&data).unwrap();

        assert_eq!(value["metadata"]["dataLossOccurred"], true);
        trace.close(&json!({"handle":handle})).unwrap();
        assert!(trace.read(&json!({"handle":handle})).is_err());
    }
}
