//! Covers: managed standalone storage, child invalidation, inspection edits and host teardown.
//! Real: shared runtime, storage registry and GPUI layout/paint. Doubles: GPUI TestPlatform.
//! Requires: devtools and view-test-support; no native desktop/GPU or network.

use super::*;
use crate::control::{Command, Message, PROTOCOL_VERSION, Query, Reply, Request, RuntimeClient};
use crate::inspection::{StyleDeclaration, StyleEdit, TextEdit};
use crate::storage::{
    MemoryBackend, Store,
    devtools::{Area, Storage},
};
use futures::{FutureExt, StreamExt};
use std::cell::RefCell;
use std::rc::Rc;

struct Content {
    focus: gpui::FocusHandle,
}

impl Render for Content {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::view! {
            <div id="content" class="h-10" style={|s| s.w(gpui::px(120.))} focus={&self.focus}>
                <button id="button">"Standalone"</button>
            </div>
        }
    }
}

fn request(
    host: &RuntimeClient<crate::router::NoRoute>,
    message: Message,
    key: Option<&str>,
) -> Request {
    Request {
        protocol: PROTOCOL_VERSION,
        runtime_id: host.info().runtime_id,
        key: key.map(str::to_owned),
        message,
    }
}

fn config(policy: ClosePolicy) -> ManagedConfig {
    ManagedConfig {
        config: RouterConfig::default(),
        control: ControlConfig::Disabled,
        commands: Commands::new(),
        application_id: "managed.tooling".into(),
        close_policy: policy,
        quit_guards: Vec::new(),
    }
}

#[test]
fn managed_root_shares_async_storage_and_window_teardown_follows_close_policy() {
    for policy in [ClosePolicy::Quit, ClosePolicy::KeepRunning] {
        let store = Store::<String>::new(MemoryBackend::new());
        let services = crate::App::new("managed.storage")
            .devtools_storage(
                Storage::new()
                    .read_write("settings", Area::Local, store.clone())
                    .unwrap(),
            )
            .unwrap()
            .services()
            .unwrap();
        let mut app = gpui::TestAppContext::single();
        let runtime_client = Rc::new(RefCell::new(None));
        let client_slot = runtime_client.clone();
        let (_, cx) = app.add_window_view(|_, cx| {
            let runtime = Runtime::new(RouterConfig::default(), services);
            let client = runtime.router.client();
            *client_slot.borrow_mut() = Some((client, runtime.router.clone()));
            let content: AnyView = cx.new(|_| gpui::Empty).into();
            ManagedRoot::new(content, runtime, config(policy), cx)
        });
        cx.run_until_parked();
        let (client, router) = runtime_client.borrow().as_ref().unwrap().clone();
        let host = client.clone();
        let edit = request(
            &host,
            Message::Command(Command::EditStorage {
                edit: rake_protocol::storage::Edit {
                    store: "settings".into(),
                    key: "theme".into(),
                    value: Some("\"dark\"".into()),
                },
            }),
            Some("write"),
        );
        assert!(matches!(
            futures::executor::block_on(host.request_async(edit.clone())).unwrap(),
            Reply::Acknowledged
        ));
        assert_eq!(
            futures::executor::block_on(store.get("theme"))
                .unwrap()
                .as_deref(),
            Some("dark")
        );
        let read = request(
            &host,
            Message::Query(Query::Storage {
                read: rake_protocol::storage::Read::Stores,
            }),
            None,
        );
        assert!(matches!(
            futures::executor::block_on(host.request_async(read)).unwrap(),
            Reply::Data(_)
        ));

        cx.update(|window, _| window.remove_window());
        cx.run_until_parked();
        let result = futures::executor::block_on(host.request_async(edit));
        match policy {
            ClosePolicy::Quit => assert!(matches!(
                result,
                Err(crate::control::ControlError::RuntimeShutdown)
            )),
            ClosePolicy::KeepRunning => assert!(matches!(result, Ok(Reply::Acknowledged))),
        }
        router.shutdown();
        cx.run_until_parked();
    }
}

#[test]
fn managed_inspection_edits_invalidate_erased_content_and_support_reset_undo_and_focus() {
    let mut app = gpui::TestAppContext::single();
    let client_slot = Rc::new(RefCell::new(None));
    let slot = client_slot.clone();
    let mut content = None;
    let (_, cx) = app.add_window_view(|_, cx| {
        let services = crate::App::new("managed.edits").services().unwrap();
        let runtime = Runtime::new(RouterConfig::default(), services);
        let client = runtime.router.client();
        client.inspection().attach();
        *slot.borrow_mut() = Some((client, runtime.router.clone()));
        let view = cx.new(|cx| Content {
            focus: cx.focus_handle(),
        });
        content = Some(view.clone());
        ManagedRoot::new(view.into(), runtime, config(ClosePolicy::Quit), cx)
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let (client, router) = client_slot.borrow().as_ref().unwrap().clone();
    let host = client.clone();
    let mut invalidations = cx.notifications(&content.unwrap());
    let set_width = |value: &str| StyleEdit::Set {
        declaration: StyleDeclaration {
            property: "width".into(),
            value: value.into(),
            enabled: true,
        },
    };
    for (text_edit, style_edit, expected_text, expected_width) in [
        (
            TextEdit::Set {
                text: "Edited café 🙂".into(),
            },
            set_width("240px"),
            "Edited café 🙂",
            240.,
        ),
        (TextEdit::Reset {}, StyleEdit::Reset, "Standalone", 120.),
        (TextEdit::Undo {}, StyleEdit::Undo, "Edited café 🙂", 240.),
        (
            TextEdit::Set {
                text: String::new(),
            },
            set_width("180px"),
            "",
            180.,
        ),
        (TextEdit::Undo {}, StyleEdit::Undo, "Edited café 🙂", 240.),
    ] {
        while matches!(invalidations.next().now_or_never(), Some(Some(()))) {}
        let before = client.inspection().snapshot();
        let text = before
            .elements
            .iter()
            .find(|node| node.text_editable && node.text.is_some())
            .unwrap();
        let styled = before
            .elements
            .iter()
            .find(|node| node.semantic_id.as_deref() == Some("content"))
            .unwrap();
        for command in [
            Command::EditElementText {
                id: text.id.clone(),
                generation: before.capture.clone(),
                edit: text_edit,
            },
            Command::EditElementStyle {
                id: styled.id.clone(),
                generation: before.capture.clone(),
                edit: style_edit,
            },
        ] {
            assert!(matches!(
                futures::executor::block_on(host.request_async(request(
                    &host,
                    Message::Command(command),
                    None
                )))
                .unwrap(),
                Reply::Acknowledged
            ));
        }
        cx.run_until_parked();
        assert_eq!(
            invalidations.next().now_or_never(),
            Some(Some(())),
            "tooling edits must invalidate the erased application view"
        );
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let after = client.inspection().snapshot();
        let text_after = after
            .elements
            .iter()
            .find(|node| node.stable_id == text.stable_id)
            .unwrap();
        let style_after = after
            .elements
            .iter()
            .find(|node| node.stable_id == styled.stable_id)
            .unwrap();
        assert_eq!(text_after.text.as_deref(), Some(expected_text));
        assert_eq!(style_after.layout.unwrap().width, expected_width);
        assert!(
            after
                .elements
                .iter()
                .all(|node| node.tab_id.is_none() && node.entry_id.is_none())
        );
    }
    let snapshot = client.inspection().snapshot();
    let target = snapshot
        .elements
        .iter()
        .find(|node| node.semantic_id.as_deref() == Some("content"))
        .unwrap();
    let focus = Command::NavigateElement {
        id: target.id.clone(),
        generation: snapshot.capture,
        action: crate::inspection::NavigationAction::Focus,
    };
    assert!(matches!(
        futures::executor::block_on(host.request_async(request(
            &host,
            Message::Command(focus),
            None
        )))
        .unwrap(),
        Reply::Acknowledged
    ));
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        client
            .inspection()
            .snapshot()
            .elements
            .iter()
            .any(|node| node.semantic_id.as_deref() == Some("content") && node.navigation.focused)
    );
    router.shutdown();
    cx.run_until_parked();
}
