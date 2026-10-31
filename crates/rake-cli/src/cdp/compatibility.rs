//! Pinned CDP 1.3 subset. Native capabilities are intentionally narrower than a browser engine.

use serde_json::{Value, json};

pub(super) const VERSION: &str = "1.3";
pub(super) const PRODUCT: &str = concat!("Rake/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Domain {
    Accessibility,
    Css,
    Dom,
    DomStorage,
    Log,
    Network,
    Overlay,
    Page,
    Runtime,
}

impl Domain {
    pub fn of(method: &str) -> Option<Self> {
        Some(match method.split('.').next()? {
            "Accessibility" => Self::Accessibility,
            "CSS" => Self::Css,
            "DOM" => Self::Dom,
            "DOMStorage" => Self::DomStorage,
            "Log" => Self::Log,
            "Network" => Self::Network,
            "Overlay" => Self::Overlay,
            "Page" => Self::Page,
            "Runtime" => Self::Runtime,
            _ => return None,
        })
    }
}

pub(super) fn handle(method: &str) -> Option<Value> {
    match method {
        "Browser.getVersion" => Some(json!({
            "protocolVersion":VERSION,"product":PRODUCT,
            "revision":env!("CARGO_PKG_VERSION"),"userAgent":PRODUCT,"jsVersion":""
        })),
        "Schema.getDomains" => Some(json!({"domains":([
            "Accessibility", "Browser", "CSS", "DOM", "DOMStorage", "HeapProfiler",
            "IO", "Inspector", "Log", "Network", "Overlay", "Page", "Performance",
            "Runtime", "Schema", "Tracing", "Rake"
        ].map(|name| json!({"name":name,"version":VERSION})))})),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_reply_conforms_to_pinned_upstream_return_types() {
        let contract: Value = serde_json::from_str(include_str!("contract.json")).unwrap();
        let domain = contract["domains"]
            .as_array()
            .unwrap()
            .iter()
            .find(|domain| domain["domain"] == "Browser")
            .unwrap();
        let command = &domain["commands"][0];
        let reply = handle("Browser.getVersion").unwrap();
        for field in command["returns"].as_array().unwrap() {
            assert_eq!(field["type"], "string");
            assert!(reply[field["name"].as_str().unwrap()].is_string());
        }
        assert_eq!(reply["protocolVersion"], VERSION);
        assert_eq!(reply["jsVersion"], "", "there is no JavaScript VM");
        assert!(handle("Debugger.enable").is_none());
        let domains = handle("Schema.getDomains").unwrap();
        assert!(
            !domains["domains"]
                .as_array()
                .unwrap()
                .iter()
                .any(|domain| domain["name"] == "Debugger")
        );
    }
}
