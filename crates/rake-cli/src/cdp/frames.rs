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
        *self = Self {
            active: true,
            stream_mode: mode == "ReturnAsStream",
            ..Self::default()
        };
        Ok(())
    }

    pub fn collect(&mut self, frames: &Frames) -> Vec<Value> {
        let baseline = *self.initial_dropped.get_or_insert(frames.dropped);
        self.loss |= frames.gap || frames.dropped > baseline;
        self.cursor = frames.cursor;
        let events = events(frames);
        if !self.stream_mode {
            return events;
        }
        for event in events {
            let bytes = serde_json::to_vec(&event).map_or(MAX_TRACE_BYTES, |bytes| bytes.len()) + 1;
            if self.bytes + bytes < MAX_TRACE_BYTES - 1024 {
                self.bytes += bytes;
                self.events.push(event);
            } else {
                self.loss = true;
            }
        }
        Vec::new()
    }

    pub fn end(&mut self) -> Value {
        self.active = false;
        if self.stream_mode {
            let document = json!({"traceEvents":std::mem::take(&mut self.events),"displayTimeUnit":"ms",
                "metadata":{"clock":"recording monotonic microseconds","measurement":"GPUI draw and inclusive boundary CPU wall time; no GPU execution or CPU samples","dataLossOccurred":self.loss}});
            self.stream = Some((document.to_string(), 0));
            json!({"dataLossOccurred":self.loss,"stream":"rake-frames","traceFormat":"json","streamCompression":"none"})
        } else {
            json!({"dataLossOccurred":self.loss})
        }
    }

    pub fn read(&mut self, params: &Value) -> Result<Value, Error> {
        if params["handle"] != "rake-frames" {
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

    pub fn close(&mut self) {
        self.stream = None;
    }
}

fn events(frames: &Frames) -> Vec<Value> {
    let mut events = Vec::new();
    for frame in &frames.frames {
        events.push(json!({"name":"thread_name","ph":"M","pid":1,"tid":frame.window.0,"args":{"name":format!("GPUI window {}",frame.window.0)}}));
        events.push(json!({"name":"RunTask","cat":"devtools.timeline","ph":"X","pid":1,"tid":frame.window.0,
            "ts":frame.elapsed_us,"dur":frame.draw_us,"args":{"data":{"kind":"GPUI window draw","frame":frame.sequence,"invalidations":frame.invalidations,"dirtyToDrawUs":frame.dirty_to_draw_us,"omittedNodeTimings":frame.omitted_nodes}}}));
        for node in &frame.nodes {
            let phase = match node.phase {
                rake_protocol::inspection::FramePhase::Layout => "Layout",
                rake_protocol::inspection::FramePhase::Prepaint => "PrePaint",
                rake_protocol::inspection::FramePhase::Paint => "Paint",
            };
            events.push(json!({"name":phase,"cat":"devtools.timeline","ph":"X","pid":1,"tid":frame.window.0,"ts":node.elapsed_us,"dur":node.duration_us,
                "args":{"data":{"boundary":node.name,"stableId":node.stable_id,"url":node.source.file,"lineNumber":node.source.line.saturating_sub(1),"columnNumber":node.source.column.saturating_sub(1),"inclusive":true}}}));
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn streams_valid_trace_json_with_bounded_reads_and_explicit_close() {
        let mut trace = Trace::default();
        trace
            .begin(&json!({"transferMode":"ReturnAsStream"}))
            .unwrap();
        let frames = Frames {
            recording: true,
            frames: Vec::new(),
            cursor: 3,
            more: false,
            gap: true,
            dropped: 1,
        };
        assert!(trace.collect(&frames).is_empty());
        assert_eq!(trace.end()["stream"], "rake-frames");
        let mut data = String::new();
        loop {
            let read = trace
                .read(&json!({"handle":"rake-frames","size":7}))
                .unwrap();
            data.push_str(read["data"].as_str().unwrap());
            if read["eof"] == true {
                break;
            }
        }
        let value: Value = serde_json::from_str(&data).unwrap();
        assert_eq!(value["metadata"]["dataLossOccurred"], true);
        trace.close();
        assert!(trace.read(&json!({"handle":"rake-frames"})).is_err());
    }
}
