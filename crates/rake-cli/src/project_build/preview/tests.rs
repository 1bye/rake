//! Covers: preview reference/input/selection contracts ported from test-preview.py.
//! Real: preview state and protocol; doubles: runtime replies and capture helpers.
//! Requires: Unix pipes and loopback; no GPU.
use super::*;
use serde_json::json;
use std::sync::Mutex;

type Reply = dyn Fn(&Value) -> Result<Value> + Send + Sync;

struct Inspector {
    requests: Mutex<Vec<Value>>,
    reply: Box<Reply>,
}

impl super::Inspector for Inspector {
    fn handle(&self, action: &str, body: &Value) -> Result<Value> {
        if action == "inspect" {
            self.requests.lock().unwrap().push(body["request"].clone());

            return Ok(
                json!({"Ok":{"type":"data","value":(self.reply)(&body["request"]["message"]["value"])?}}),
            );
        }

        Ok(Value::Null)
    }
}

fn fixture(
    reply: impl Fn(&Value) -> Result<Value> + Send + Sync + 'static,
) -> (Preview, Arc<Inspector>) {
    let inspector = Arc::new(Inspector {
        requests: Mutex::default(),
        reply: Box::new(reply),
    });

    let preview = Preview::new(inspector.clone(), "/unused".into());

    preview
        .state
        .lock()
        .unwrap()
        .references
        .push_back(Reference {
            token: "frame".into(),
            runtime: "runtime".into(),
            window: window(),
            created: Instant::now(),
        });

    (preview, inspector)
}

fn window() -> Value {
    json!({"window":"window-1","native_id":42,"generation":"current","frame":{"width":600,"height":428},"content":{"x":0,"y":28,"width":600,"height":400}})
}

fn body() -> Value {
    json!({"reference":"frame","x":100.4,"y":80})
}

#[test]
fn selection_keeps_source_and_exact_window_without_mutation() {
    let (preview, inspector) = fixture(|_| Ok(json!({"source":{"file":"src/main.rs","line":12}})));

    assert_eq!(
        preview.handle("preview_select", &body()).unwrap()["source"]["file"],
        "src/main.rs"
    );
    let requests = inspector.requests.lock().unwrap();

    assert_eq!(requests[0]["message"]["type"], "query");
    assert_eq!(
        requests[0]["message"]["value"]["point"],
        json!({"window":"window-1","native_id":42,"generation":"current","x":100,"y":80})
    );
}

#[test]
fn click_is_submitted_once_without_retry_key_and_fractional_scroll_keeps_geometry() {
    let (preview, inspector) = fixture(|_| Ok(json!({})));
    let mut input = body();

    input["input"] = json!({"type":"click"});
    preview.handle("preview_input", &input).unwrap();
    {
        let requests = inspector.requests.lock().unwrap();

        assert_eq!(requests.len(), 1);
        assert!(requests[0]["key"].is_null());
        assert_eq!(requests[0]["message"]["type"], "command");
        assert_eq!(
            requests[0]["message"]["value"]["point"]["geometry"],
            json!([38400, 27392, 0, 1792, 38400, 25600])
        );
    }
    input["input"] = json!({"type":"wheel","dx":1,"dy":-31,"phase":"moved"});
    preview.handle("preview_input", &input).unwrap();
    assert_eq!(
        inspector.requests.lock().unwrap()[1]["message"]["value"]["input"]["dy"],
        -31
    );
}

#[test]
fn invalid_expired_or_foreign_positions_never_reach_runtime() {
    let (preview, inspector) = fixture(|_| panic!("invalid input reached runtime"));

    for (key, value) in [
        ("reference", json!("other")),
        ("x", json!(true)),
        ("y", json!(-1)),
        ("x", json!(600)),
        ("x", Value::Null),
    ] {
        let mut value_body = body();

        value_body[key] = value;
        assert!(preview.handle("preview_select", &value_body).is_err());
    }

    preview.state.lock().unwrap().references[0].created -= Duration::from_secs(6);
    assert_eq!(
        preview
            .handle("preview_select", &body())
            .unwrap_err()
            .status,
        409
    );
    assert!(inspector.requests.lock().unwrap().is_empty());
}

#[test]
fn scroll_is_bounded_and_unsupported_input_is_rejected() {
    let (preview, inspector) = fixture(|_| panic!("invalid input reached runtime"));

    for input in [
        json!({"type":"keyboard"}),
        json!({"type":"scroll","dx":0,"dy":2001}),
        json!({"type":"scroll","dx":0,"dy":null}),
        json!({"type":"wheel","dx":0,"dy":128001,"phase":"started"}),
        json!({"type":"wheel","dx":0,"dy":1.5,"phase":"moved"}),
        json!({"type":"wheel","dx":0,"dy":1,"phase":"unknown"}),
    ] {
        let mut b = body();

        b["input"] = input;
        assert!(preview.handle("preview_input", &b).is_err());
    }

    assert!(inspector.requests.lock().unwrap().is_empty());
}

#[test]
fn grab_promotes_text_and_collects_pinned_ancestors_and_descendant_text() {
    let root = json!({"id":"root","kind":"View","view":{"tag":"Editor"},"source":{"file":"src/editor.rs","line":4,"column":2}});
    let button = json!({"id":"button","parent":"root","kind":"Div","view":{"tag":"button"},"source":{"file":"src/button.rs","line":7},"styles":{"not":"copied"}});

    let leaf =
        json!({"id":"leaf","parent":"button","kind":"Text","view":{"tag":"Text"},"text":"Save"});

    let (preview, inspector) = fixture(move |query| {
        Ok(match query["type"].as_str().unwrap() {
            "preview_element" => leaf.clone(),
            "inspect_element" => button.clone(),
            _ => {
                assert_eq!(query["generation"], "current");

                if query["offset"] == 0 {
                    json!({"snapshot":{"capture":"current","elements":[root,button]},"next":2})
                } else {
                    json!({"snapshot":{"capture":"current","elements":[leaf,{"id":"elsewhere","parent":"root","text":"Do not include"}]},"next":null})
                }
            }
        })
    });

    let result = preview.handle("preview_grab", &body()).unwrap();

    assert_eq!(result["element"]["id"], "button");
    assert_eq!(result["element"]["grab"]["text"], "Save");
    assert_eq!(result["element"]["grab"]["frames"][0]["name"], "button");
    assert_eq!(result["element"]["grab"]["frames"][1]["name"], "Editor");
    assert_eq!(result["element"]["grab"]["truncated"], false);
    assert!(result["element"].get("styles").is_none());
    assert_eq!(
        inspector.requests.lock().unwrap().last().unwrap()["message"]["value"]["offset"],
        2
    );
}

#[test]
fn hover_is_lightweight_and_expired_context_never_uses_another_capture() {
    let (preview, inspector) = fixture(|query| {
        Ok(if query["type"] == "preview_element" {
            json!({"id":"button","kind":"Div","view":{"tag":"button"},"text":"Save","source":{"file":"src/view.rs","line":9},"resolved":{"large":"not copied"}})
        } else {
            json!({"snapshot":{"capture":"different","elements":[{"id":"button","text":"wrong"}]},"next":null})
        })
    });

    let hovered = preview.handle("preview_hover", &body()).unwrap();

    assert_eq!(inspector.requests.lock().unwrap().len(), 1);
    assert!(hovered["element"].get("grab").is_none());
    assert!(hovered["element"].get("resolved").is_none());
    let grabbed = preview.handle("preview_grab", &body()).unwrap();

    assert_eq!(grabbed["element"]["grab"]["truncated"], true);
    assert_eq!(grabbed["element"]["grab"]["text"], "Save");
    assert_eq!(grabbed["element"]["grab"]["frames"][0]["source"]["line"], 9);
}

fn annotation(targets: Value) -> Value {
    json!({"reference":"frame","targets":targets})
}

#[test]
fn annotation_resolution_follows_stable_ids_and_keeps_exact_targets_scoped() {
    let element = json!({"id":"element-current-4","stable_id":"boundary-7","layout":{"x":20,"y":30,"width":80,"height":20},"styles":{"must":"not copy"},"source":{"file":"private.rs"}});
    let layout = element["layout"].clone();

    let (preview, _) = fixture(move |query| {
        Ok(if query["type"] == "preview" {
            json!([window()])
        } else {
            json!({"snapshot":{"capture":"current","elements":[element]},"next":null})
        })
    });

    let result=preview.handle("preview_resolve",&annotation(json!([{"stable_id":"boundary-7"},{"id":"element-current-4","generation":"current"},{"id":"element-current-4","generation":"old"},{"stable_id":"removed"}]))).unwrap();

    assert_eq!(result["generation"], "current");
    assert_eq!(result["elements"][0]["layout"], layout);
    assert_eq!(result["elements"][0], result["elements"][1]);
    assert!(result["elements"][2].is_null() && result["elements"][3].is_null());
    assert!(result["elements"][0].get("styles").is_none());
    assert!(result["elements"][0].get("source").is_none());
}

#[test]
fn annotation_resolution_pins_pages_and_rejects_other_window_generations() {
    let (preview, inspector) = fixture(|query| {
        Ok(if query["type"] == "preview" {
            json!([window()])
        } else if query["offset"] == 0 {
            json!({"snapshot":{"capture":"current","elements":[]},"next":256})
        } else {
            assert_eq!(query["generation"], "current");

            json!({"snapshot":{"capture":"current","elements":[{"id":"new","stable_id":"boundary-7"}]},"next":null})
        })
    });

    let result = preview
        .handle(
            "preview_resolve",
            &annotation(json!([{"stable_id":"boundary-7"}])),
        )
        .unwrap();

    assert_eq!(result["elements"][0]["id"], "new");
    assert_eq!(
        inspector.requests.lock().unwrap().last().unwrap()["message"]["value"]["offset"],
        256
    );
    let (preview, _) = fixture(|query| {
        Ok(if query["type"] == "preview" {
            json!([])
        } else {
            json!({"snapshot":{"capture":"foreign","elements":[]}})
        })
    });

    assert_eq!(
        preview
            .handle(
                "preview_resolve",
                &annotation(json!([{"stable_id":"boundary-7"}]))
            )
            .unwrap_err()
            .status,
        409
    );
}

#[test]
fn annotation_resolution_rejects_nonadvancing_pages_and_masks_hidden_targets() {
    let (preview, _) = fixture(|query| {
        Ok(if query["type"] == "preview" {
            json!([window()])
        } else {
            json!({"snapshot":{"capture":"current","elements":[]},"next":0})
        })
    });

    assert_eq!(
        preview
            .handle(
                "preview_resolve",
                &annotation(json!([{"stable_id":"boundary-7"}]))
            )
            .unwrap_err()
            .status,
        409
    );
    let (preview, _) = fixture(|query| {
        Ok(if query["type"] == "preview" {
            json!([window()])
        } else {
            json!({"snapshot":{"capture":"current","elements":[{"id":"hidden","stable_id":"boundary-7","layout":{"x":0},"accessibility":{"hidden":true}}]},"next":null})
        })
    });

    assert!(
        preview
            .handle(
                "preview_resolve",
                &annotation(json!([{"stable_id":"boundary-7"}]))
            )
            .unwrap()["elements"][0]["layout"]
            .is_null()
    );
}

#[test]
fn invalid_annotation_targets_and_expired_references_never_reach_runtime() {
    let (preview, inspector) = fixture(|_| panic!("invalid annotations reached runtime"));

    for targets in [
        Value::Null,
        json!([]),
        json!([null]),
        json!([{"stable_id":true}]),
        json!([{"id":"x"}]),
        json!([{"stable_id":"x","extra":1}]),
        json!(vec![json!({"stable_id":"x"}); 34]),
    ] {
        assert!(
            preview
                .handle("preview_resolve", &annotation(targets))
                .is_err()
        );
    }

    let mut b = annotation(json!([{"stable_id":"boundary-7"}]));

    b["reference"] = json!("foreign");
    assert_eq!(
        preview.handle("preview_resolve", &b).unwrap_err().status,
        409
    );
    preview.state.lock().unwrap().references[0].created -= Duration::from_secs(6);
    assert_eq!(
        preview
            .handle(
                "preview_resolve",
                &annotation(json!([{"stable_id":"boundary-7"}]))
            )
            .unwrap_err()
            .status,
        409
    );
    assert!(inspector.requests.lock().unwrap().is_empty());
}

struct StateInspector {
    window: Mutex<Value>,
    runtime: Mutex<&'static str>,
}

impl super::Inspector for StateInspector {
    fn handle(&self, action: &str, body: &Value) -> Result<Value> {
        Ok(match action {
            "development" => json!({"runtime_pid":1,"app_running":true}),
            "inspect_instances" => {
                json!([{"pid":1,"transport":"local","info":{"runtime_id":*self.runtime.lock().unwrap()}}])
            }
            _ => {
                assert_eq!(body["request"]["message"]["value"]["type"], "preview");

                json!({"Ok":{"value":[self.window.lock().unwrap().clone()]}})
            }
        })
    }
}

#[test]
fn preview_state_bounds_references_and_changes_epoch_only_for_window_geometry() {
    let inspector = Arc::new(StateInspector {
        window: Mutex::new(window()),
        runtime: Mutex::new("runtime"),
    });

    let preview = Preview::new(inspector.clone(), "/unused".into());
    let first = preview.handle("preview_state", &json!({})).unwrap();

    for _ in 0..65 {
        preview.handle("preview_state", &json!({})).unwrap();
    }

    assert_eq!(preview.state.lock().unwrap().references.len(), 64);
    assert!(preview.reference(&first).is_err());
    inspector.window.lock().unwrap()["generation"] = json!("next");
    let second = preview.handle("preview_state", &json!({})).unwrap();

    assert_eq!(first["epoch"], second["epoch"]);
    inspector.window.lock().unwrap()["content"]["width"] = json!(500);
    let third = preview.handle("preview_state", &json!({})).unwrap();

    assert_ne!(second["epoch"], third["epoch"]);
    assert!(preview.reference(&second).is_err());
    assert_eq!(preview.state.lock().unwrap().references.len(), 1);
    preview.close();
    assert!(preview.state.lock().unwrap().references.is_empty());
    assert_eq!(
        preview
            .handle("preview_state", &json!({}))
            .unwrap_err()
            .status,
        503
    );
}

#[test]
fn body_patches_keep_preview_epoch_and_restarts_rebind_the_runtime() {
    let inspector = Arc::new(StateInspector {
        window: Mutex::new(window()),
        runtime: Mutex::new("runtime"),
    });

    let preview = Preview::new(inspector.clone(), "/unused".into());
    let first = preview.handle("preview_state", &json!({})).unwrap();

    inspector.window.lock().unwrap()["generation"] = json!("patched");
    let patched = preview.handle("preview_state", &json!({})).unwrap();

    assert_eq!(first["epoch"], patched["epoch"]);
    assert!(preview.reference(&first).is_ok());

    // Even a reused native window ID must not retain the old runtime's capture.
    *inspector.runtime.lock().unwrap() = "restored-runtime";
    let restored = preview.handle("preview_state", &json!({})).unwrap();

    assert_ne!(patched["epoch"], restored["epoch"]);
    assert_eq!(restored["runtime_id"], "restored-runtime");
    assert!(preview.reference(&patched).is_err());
    assert_eq!(
        preview.reference(&restored).unwrap().runtime,
        "restored-runtime"
    );
}

#[test]
fn disconnect_during_video_headers_releases_encoder() {
    use std::{
        io::Read,
        net::{TcpListener, TcpStream},
        os::fd::AsRawFd,
    };
    let preview = Preview::new(
        Arc::new(StateInspector {
            window: Mutex::new(window()),
            runtime: Mutex::new("runtime"),
        }),
        "/bin/sleep".into(),
    );

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (mut stream, _) = listener.accept().unwrap();

    let linger = libc::linger {
        l_onoff: 1,
        l_linger: 0,
    };

    assert_eq!(
        // SAFETY: client is live and linger has the socket option's exact layout.
        unsafe {
            libc::setsockopt(
                client.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_LINGER,
                (&linger as *const libc::linger).cast(),
                std::mem::size_of_val(&linger) as libc::socklen_t,
            )
        },
        0
    );
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    drop(client);
    let _ = stream.read(&mut [0]);

    preview
        .video(&mut stream, &json!({"codec":"h264"}))
        .unwrap();
    assert!(preview.state.lock().unwrap().captures.is_empty());
}

#[test]
fn preview_matches_python_reference_corpus() {
    let fixtures = json!({"window":window(),"root":{"id":"root","view":{"tag":"Editor"},"source":{"file":"src/editor.rs","line":4}},"button":{"id":"button","parent":"root","view":{"tag":"button"},"source":{"file":"src/button.rs","line":7}},"leaf":{"id":"leaf","parent":"button","view":{"tag":"Text"},"text":"Save"}});

    let mut cases = vec![
        ("preview_select", body()),
        ("preview_hover", body()),
        ("preview_grab", body()),
        ("preview_restart", json!({})),
        ("unknown", json!({})),
        ("preview_select", json!([])),
    ];

    for input in [
        json!({"type":"click"}),
        json!({"type":"move"}),
        json!({"type":"scroll","dx":-2,"dy":2000}),
        json!({"type":"wheel","dx":1,"dy":-31,"phase":"moved"}),
        json!({"type":"keyboard"}),
        json!({"type":"scroll","dx":0,"dy":2001}),
        json!({"type":"wheel","dx":0,"dy":128001,"phase":"started"}),
        json!({"type":"wheel","dx":0,"dy":1.5,"phase":"moved"}),
        json!({"type":"wheel","dx":0,"dy":1,"phase":"unknown"}),
    ] {
        let mut b = body();

        b["input"] = input;
        cases.push(("preview_input", b));
    }

    for (key, value) in [
        ("reference", json!("foreign")),
        ("x", json!(true)),
        ("y", json!(-1)),
        ("x", json!(600)),
        ("x", Value::Null),
    ] {
        let mut b = body();

        b[key] = value;
        cases.push(("preview_select", b));
    }

    for targets in [
        json!([{"id":"button","generation":"current"}]),
        json!([{"id":"button","generation":"old"}]),
        json!([{"stable_id":"removed"}]),
        Value::Null,
        json!([]),
        json!([null]),
        json!([{"stable_id":true}]),
        json!([{"id":"x"}]),
        json!([{"stable_id":"x","extra":1}]),
        json!(vec![json!({"stable_id":"x"}); 34]),
    ] {
        cases.push(("preview_resolve", annotation(targets)));
    }

    let mut expected = Vec::new();

    for (action, body) in &cases {
        let data = fixtures.clone();

        let (preview, inspector) = fixture(move |query| {
            Ok(match query["type"].as_str().unwrap() {
                "preview" => json!([data["window"]]),
                "preview_element" => data["leaf"].clone(),
                "inspect_element" => data["button"].clone(),
                "preview_input" => json!({"operation_id":9}),
                _ => {
                    json!({"snapshot":{"capture":"current","elements":[data["root"],data["button"],data["leaf"]]},"next":null})
                }
            })
        });

        let result = match preview.handle(action, body) {
            Ok(value) => json!({"Ok":value}),
            Err(error) => json!({"Err":{"status":error.status,"message":error.to_string()}}),
        };

        expected
            .push(json!({"result":result,"requests":inspector.requests.lock().unwrap().clone()}));
    }

    // Captured from the retained Python reference and compared before its deletion.
    let actual: Vec<Value> = serde_json::from_str(include_str!(
        "../../../tests/fixtures/preview-python-reference.json"
    ))
    .unwrap();

    assert_eq!(actual, expected);
    assert_eq!(cases.len(), 30);
}
