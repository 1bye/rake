//! Connection-owned bounded condition watches. No mutation or sleeping worker.

use super::{semantic, wire::Error};
use rake_protocol::router::WindowId;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const MAX_WAITS: usize = 32;
#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Goal {
    #[default]
    Present,
    Absent,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    // Validated by Session before decoding this operation.
    #[serde(rename = "rakeRuntimeId")]
    _runtime: Option<String>,
    #[serde(default)]
    query: Value,
    #[serde(default)]
    state: Goal,
    #[serde(default = "default_timeout")]
    timeout_ms: u64,
    after_frame: Option<String>,
}
fn default_timeout() -> u64 {
    5000
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Pending,
    Satisfied,
    TimedOut,
    Cancelled,
    Invalidated,
}
impl Status {
    fn name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Satisfied => "satisfied",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
            Self::Invalidated => "invalidated",
        }
    }
}
struct Entry {
    id: String,
    runtime: String,
    window: WindowId,
    arguments: Value,
    until: Instant,
    after_frame: Option<u64>,
    absent: bool,
    status: Status,
    observation: Option<Value>,
    error: Option<String>,
}
#[derive(Default)]
pub(super) struct Waits(VecDeque<Entry>);
impl Waits {
    pub fn start(
        &mut self,
        params: &Value,
        runtime: String,
        window: WindowId,
        now: Instant,
    ) -> Result<String, Error> {
        let request: Request = serde_json::from_value(params.clone())
            .map_err(|e| Error::new(-32602, e.to_string()))?;
        if !(1..=30000).contains(&request.timeout_ms) {
            return Err(Error::new(-32602, "timeoutMs must be within 1–30000"));
        }
        let after_frame = request
            .after_frame
            .map(|n| {
                n.parse::<u64>()
                    .map_err(|_| Error::new(-32602, "afterFrame must be a decimal sequence string"))
            })
            .transpose()?;
        let arguments =
            json!({"query":if request.query.is_null(){json!({})}else{request.query},"limit":1});
        semantic::selector(&arguments)?;
        for entry in &mut self.0 {
            entry.expire(now);
        }
        if self.0.len() == MAX_WAITS {
            if let Some(index) = self
                .0
                .iter()
                .position(|entry| entry.status != Status::Pending)
            {
                self.0.remove(index);
            } else {
                return Err(Error::new(-32000, "32 pending waits already retained"));
            }
        }
        let id = format!("wait-{}", uuid::Uuid::new_v4());
        self.0.push_back(Entry {
            id: id.clone(),
            runtime,
            window,
            arguments,
            until: now + Duration::from_millis(request.timeout_ms),
            after_frame,
            absent: matches!(request.state, Goal::Absent),
            status: Status::Pending,
            observation: None,
            error: None,
        });
        Ok(id)
    }
    fn entry(&mut self, id: &str) -> Result<&mut Entry, Error> {
        self.0
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| Error::new(-32000, "Wait handle is unknown or evicted"))
    }
    pub fn arguments(
        &mut self,
        id: &str,
        window: Option<WindowId>,
        now: Instant,
    ) -> Result<Option<Value>, Error> {
        let entry = self.entry(id)?;
        entry.expire(now);
        if entry.status == Status::Pending && Some(entry.window) != window {
            entry.status = Status::Invalidated;
            entry.error = Some("Selected window changed".into());
        }
        Ok((entry.status == Status::Pending).then(|| entry.arguments.clone()))
    }
    pub fn observe(
        &mut self,
        id: &str,
        result: Result<Value, Error>,
        now: Instant,
    ) -> Result<Value, Error> {
        let entry = self.entry(id)?;
        entry.expire(now);
        if entry.status == Status::Pending {
            match result {
                Ok(snapshot) => {
                    let total = snapshot["totalMatches"].as_u64();
                    let frame = snapshot["frameSequence"]
                        .as_str()
                        .and_then(|s| s.parse::<u64>().ok());
                    let new_frame = entry
                        .after_frame
                        .is_none_or(|after| frame.is_some_and(|frame| frame > after));
                    let complete = snapshot["omitted"].as_u64() == Some(0);
                    let matched = if entry.absent {
                        total == Some(0) && complete
                    } else {
                        total.is_some_and(|n| n > 0)
                    };
                    if new_frame && matched {
                        entry.status = Status::Satisfied;
                    }
                    entry.error = if entry.absent && !complete {
                        Some("Omitted nodes prevent confirming absence".into())
                    } else {
                        None
                    };
                    entry.observation = Some(snapshot);
                }
                Err(error) => entry.error = Some(error.message.chars().take(1024).collect()),
            }
        }
        Ok(entry.response(now))
    }
    pub fn response(&mut self, id: &str, now: Instant) -> Result<Value, Error> {
        let entry = self.entry(id)?;
        entry.expire(now);
        Ok(entry.response(now))
    }
    pub fn cancel(&mut self, id: &str, now: Instant) -> Result<Value, Error> {
        let entry = self.entry(id)?;
        entry.expire(now);
        if entry.status == Status::Pending {
            entry.status = Status::Cancelled;
        }
        Ok(entry.response(now))
    }
    pub fn invalidate(&mut self, reason: &str) {
        for entry in &mut self.0 {
            if entry.status == Status::Pending {
                entry.status = Status::Invalidated;
                entry.error = Some(reason.into());
            }
        }
    }
}
impl Entry {
    fn expire(&mut self, now: Instant) {
        if self.status == Status::Pending && now >= self.until {
            self.status = Status::TimedOut;
        }
    }
    fn response(&self, now: Instant) -> Value {
        json!({"waitId":self.id,"runtimeId":self.runtime,"windowId":self.window,"status":self.status.name(),"remainingMs":self.until.saturating_duration_since(now).as_millis(),"observation":self.observation,"error":self.error,"scope":"captured inspection tree"})
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn condition_and_later_draw_are_required_and_late_results_cannot_succeed() {
        let now = Instant::now();
        let mut waits = Waits::default();
        let id = waits
            .start(
                &json!({"afterFrame":"4","timeoutMs":100}),
                "runtime".into(),
                WindowId(1),
                now,
            )
            .unwrap();
        assert_eq!(
            waits
                .observe(
                    &id,
                    Ok(json!({"totalMatches":1,"frameSequence":"4","omitted":0})),
                    now
                )
                .unwrap()["status"],
            "pending"
        );
        assert_eq!(
            waits
                .observe(
                    &id,
                    Ok(json!({"totalMatches":1,"frameSequence":"5","omitted":0})),
                    now
                )
                .unwrap()["status"],
            "satisfied"
        );
        let late = waits
            .start(&json!({"timeoutMs":1}), "runtime".into(), WindowId(1), now)
            .unwrap();
        assert_eq!(
            waits
                .observe(
                    &late,
                    Ok(json!({"totalMatches":1})),
                    now + Duration::from_millis(2)
                )
                .unwrap()["status"],
            "timed_out"
        );
    }
    #[test]
    fn omission_cancellation_capacity_and_selection_are_explicit() {
        let now = Instant::now();
        let mut waits = Waits::default();
        let id = waits
            .start(&json!({"state":"absent"}), "r".into(), WindowId(1), now)
            .unwrap();
        assert_eq!(
            waits
                .observe(&id, Ok(json!({"totalMatches":0,"omitted":1})), now)
                .unwrap()["status"],
            "pending"
        );
        assert_eq!(waits.cancel(&id, now).unwrap()["status"], "cancelled");
        for _ in 0..MAX_WAITS {
            waits
                .start(&json!({}), "r".into(), WindowId(1), now)
                .unwrap();
        }
        assert!(
            waits
                .start(&json!({}), "r".into(), WindowId(1), now)
                .is_err()
        );
        let id = waits.0.front().unwrap().id.clone();
        assert!(
            waits
                .arguments(&id, Some(WindowId(2)), now)
                .unwrap()
                .is_none()
        );
        assert_eq!(waits.response(&id, now).unwrap()["status"], "invalidated");
        assert!(
            waits
                .start(&json!({"timeoutMs":30001}), "r".into(), WindowId(1), now)
                .is_err()
        );
    }
}
