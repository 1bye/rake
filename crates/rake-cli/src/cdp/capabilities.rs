//! Exact agent operation discovery; this is not a claim of complete browser CDP.

use rake_protocol::control::RuntimeInfo;
use serde_json::{Value, json};

pub(super) fn describe(
    info: &RuntimeInfo,
    native: bool,
    native_error: Option<String>,
    capture: bool,
    sources: bool,
) -> Value {
    let inspection = info.queries.iter().any(|q| q == "inspect_window");
    let input = native && info.commands.iter().any(|c| c == "preview_input");
    let mut methods = serde_json::Map::new();

    for (method, supported) in [
        ("Rake.getCapabilities", true),
        (
            "Rake.getWindows",
            info.queries.iter().any(|q| q == "preview"),
        ),
        ("Rake.selectWindow", native),
        ("Rake.getSnapshot", inspection && native),
        ("Rake.getEvidence", inspection && native),
        ("Rake.getActionability", inspection && native),
        ("Rake.findElements", inspection && native),
        ("Rake.waitFor", inspection && native),
        ("Rake.getWait", inspection && native),
        ("Rake.cancelWait", inspection && native),
        ("Input.dispatchMouseEvent", input),
        ("Input.dispatchKeyEvent", input),
        ("Input.insertText", input),
        ("Page.captureScreenshot", native && capture),
        ("Rake.getCaptureStatus", true),
        ("DOM.focus", info.element_interaction),
    ] {
        methods.insert(method.into(), json!({"supported":supported}));
    }

    json!({
        "version":1,"runtimeId":info.runtime_id,"platform":info.platform,"methods":methods,
        "errorContractVersion":1,
        "agentQuery":{"filters":["nodeId","stableId","id","role","name","text","textContains","textContent","textContentContains","bounds","enabled","visible"],"snapshotOptions":["limit","offset","expectedRevision","rootNodeId"],"textContent":"captured rendered text; a boundary owns its text or aggregates descendants; private input values are excluded"},
        "sourceRead":{"supported":sources && inspection && native,"transport":"snapshot source.url HTTP GET"},
        "nativeWindowsAvailable":native,"nativeError":native_error,
        "limits":{"evidenceBytes":921600,"evidenceTreeNodes":64,"capturedTextBytes":16777216,"snapshotNodes":256,"waitHandles":32,"waitTimeoutMs":30000,"requestTimeoutMs":2000,"textBytes":16384,"imageBytes":768000,"messageBytes":1048576},
        "semantics":{"actions":"accepted by native event dispatch; use a condition wait to observe rendering","replay":"never retry uncertain mutations","references":"runtime and window scoped; guarded pointer admission rejects changed capture","visibility":"captured bounds and clipping; final GPU occlusion is unavailable","coverage":"rake::view and opted-in inspection hosts; opaque descendants may be omitted","screenshot":"whole native window, independently sampled from inspection; permission checked during capture","source":"workspace-confined HTTP source URLs from semantic snapshot metadata","evaluation":"documented Rake console helpers only; no arbitrary JavaScript execution"}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_native_targets_do_not_advertise_input_or_images() {
        let info:RuntimeInfo=serde_json::from_value(json!({"protocol":1,"runtime_id":"00000000-0000-0000-0000-000000000001","platform":"native","commands":["preview_input"],"queries":["preview","inspect_window"],"supported_targets":[],"completed_operations":0,"pending_operations":0,"idempotency_capacity":0})).unwrap();
        let absent = describe(&info, false, Some("unavailable".into()), true, true);

        assert_eq!(absent["methods"]["Input.insertText"]["supported"], false);
        assert_eq!(
            absent["methods"]["Page.captureScreenshot"]["supported"],
            false
        );
        let live = describe(&info, true, None, false, true);

        assert_eq!(live["methods"]["Input.insertText"]["supported"], true);
        assert_eq!(
            live["methods"]["Page.captureScreenshot"]["supported"],
            false
        );
        assert_eq!(live["methods"]["Rake.getSnapshot"]["supported"], true);
    }
}
