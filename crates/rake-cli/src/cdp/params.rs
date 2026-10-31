//! Borrowed decoding at the CDP boundary; runtime capture IDs remain opaque strings.

use super::wire::Error;
use serde::Deserialize;
use serde_json::Value;

pub(super) fn decode<'a, T: Deserialize<'a>>(params: &'a Value) -> Result<T, Error> {
    T::deserialize(params).map_err(|error| Error::new(-32602, error.to_string()))
}

#[derive(Clone, Copy, Deserialize)]
#[serde(try_from = "i32")]
pub(super) struct NodeId(i32);

impl TryFrom<i32> for NodeId {
    type Error = &'static str;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        if value > 0 {
            Ok(Self(value))
        } else {
            Err("node IDs must be positive integers")
        }
    }
}

impl NodeId {
    pub fn get(self) -> i32 {
        self.0
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NodeSelector {
    node_id: Option<NodeId>,
    backend_node_id: Option<NodeId>,
}

pub(super) fn node_id(params: &Value) -> Result<i32, Error> {
    let selector: NodeSelector = decode(params)?;

    selector
        .node_id
        .or(selector.backend_node_id)
        .map(NodeId::get)
        .ok_or_else(|| Error::new(-32602, "nodeId or backendNodeId required"))
}

#[derive(Deserialize)]
#[serde(try_from = "i64")]
struct Depth(usize);

impl TryFrom<i64> for Depth {
    type Error = &'static str;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        let limit = rake_protocol::inspection::MAX_ELEMENTS + 1;

        match value {
            -1 => Ok(Self(limit)),
            0.. => Ok(Self(
                usize::try_from(value).unwrap_or(usize::MAX).min(limit),
            )),
            _ => Err("depth must be -1 or nonnegative"),
        }
    }
}

pub(super) fn depth(params: &Value, default: i64) -> Result<usize, Error> {
    // Missing depth and explicit null have different CDP meanings.
    match params.get("depth") {
        Some(value) => Ok(decode::<Depth>(value)?.0),
        None => Depth::try_from(default)
            .map(|depth| depth.0)
            .map_err(|message| Error::new(-32602, message)),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SearchRange {
    pub from_index: usize,
    pub to_index: usize,
}

/// Validate common hot-path commands before any blocking runtime exchange.
/// Unrecognized optional fields remain forward compatible with newer frontends.
pub(super) fn validate(method: &str, params: &Value) -> Result<(), Error> {
    match method {
        "DOM.getDocument" | "DOM.requestChildNodes" => {
            depth(params, 1)?;

            if let Some(pierce) = params.get("pierce") {
                decode::<bool>(pierce)?;
            }

            if method == "DOM.requestChildNodes" {
                node_id(params)?;
            }
        }
        "DOM.getSearchResults" => {
            decode::<&str>(&params["searchId"])?;
            let range: SearchRange = decode(params)?;

            if range.from_index >= range.to_index {
                return Err(Error::new(-32602, "Invalid search result range"));
            }
        }
        "DOM.pushNodesByBackendIdsToFrontend" => {
            decode::<Vec<NodeId>>(&params["backendNodeIds"])?;
        }
        "DOM.getBoxModel"
        | "DOM.getAttributes"
        | "CSS.getInlineStylesForNode"
        | "CSS.getMatchedStylesForNode"
        | "CSS.getComputedStyleForNode"
        | "CSS.setEffectivePropertyValueForNode"
        | "Rake.resetStyles"
        | "Rake.undoStyles" => {
            node_id(params)?;
        }
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn typed_dom_parameters_cover_pinned_schema_fields() {
        let contract: Value = serde_json::from_str(include_str!("contract.json")).unwrap();

        let dom = contract["domains"]
            .as_array()
            .unwrap()
            .iter()
            .find(|domain| domain["domain"] == "DOM")
            .unwrap();

        for command in dom["commands"].as_array().unwrap() {
            let name = command["name"].as_str().unwrap();
            let method = format!("DOM.{name}");

            let params = match name {
                "getDocument" => json!({"depth":1,"pierce":false}),
                "requestChildNodes" => json!({"nodeId":2,"depth":-1,"pierce":true}),
                "getSearchResults" => json!({"searchId":"search-1","fromIndex":0,"toIndex":2}),
                "pushNodesByBackendIdsToFrontend" => json!({"backendNodeIds":[2,3]}),
                _ => panic!("uncovered contract command"),
            };

            validate(&method, &params).unwrap();

            for field in command["parameters"].as_array().unwrap() {
                let mut invalid = params.clone();

                invalid[field["name"].as_str().unwrap()] = json!({"wrong":"type"});
                assert_eq!(validate(&method, &invalid).unwrap_err().code, -32602);
            }
        }
    }

    #[test]
    fn selectors_reject_lossy_numbers_but_allow_future_optional_fields() {
        for invalid in [
            json!(0),
            json!(-1),
            json!(1.5),
            json!(2147483648u64),
            json!("2"),
        ] {
            assert_eq!(
                node_id(&json!({"nodeId":invalid})).unwrap_err().code,
                -32602
            );
        }

        assert_eq!(
            node_id(&json!({"backendNodeId":2,"futureOption":true})).unwrap(),
            2
        );
        assert!(depth(&json!({"depth":null}), 1).is_err());
        assert!(depth(&json!({"depth":-2}), 1).is_err());
        assert_eq!(
            depth(&json!({"depth":-1}), 1).unwrap(),
            rake_protocol::inspection::MAX_ELEMENTS + 1
        );
        assert!(decode::<SearchRange>(&json!({"fromIndex":-1,"toIndex":2})).is_err());
    }
}
