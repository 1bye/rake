//! Covers: standalone capture, child redraws, focus commands and root-owned teardown.
//! Real: GPUI layout/paint, view macro and inspection client. Doubles: GPUI TestPlatform.
//! Requires: view-test-support; no display or GPU execution.

use super::*;
use crate::{
    control::{Command, Message, PROTOCOL_VERSION, Reply, Request},
    diagnostics::Collector,
};

struct Content {
    width: f32,
    focus: gpui::FocusHandle,
}
impl Render for Content {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::view! { <div id="content" class="h-10" style={|s| s.w(gpui::px(self.width))} focus={&self.focus}><button id="button">"Standalone"</button></div> }
    }
}

#[test]
fn standalone_root_captures_child_updates_and_navigation_without_a_router() {
    let mut app = gpui::TestAppContext::single();
    let client = InspectionClient::new(Collector::default());
    let mut content = None;
    let (_, cx) = app.add_window_view(|_, cx| {
        let view = cx.new(|cx| Content {
            width: 120.,
            focus: cx.focus_handle(),
        });
        content = Some(view.clone());
        InspectionRoot::new(
            view.into(),
            client.clone(),
            ControlConfig::Disabled,
            "standalone.test".into(),
            cx,
        )
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let before = client.inspection().snapshot();
    let element = before
        .elements
        .iter()
        .find(|e| e.semantic_id.as_deref() == Some("content"))
        .unwrap();
    assert_eq!(element.layout.unwrap().width, 120.);
    assert!(
        before
            .elements
            .iter()
            .all(|e| e.tab_id.is_none() && e.entry_id.is_none())
    );
    content.unwrap().update(cx, |content, cx| {
        content.width = 180.;
        cx.notify();
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    let after = client.inspection().snapshot();
    let updated = after
        .elements
        .iter()
        .find(|e| e.semantic_id.as_deref() == Some("content"))
        .unwrap();
    assert_eq!(updated.layout.unwrap().width, 180.);
    assert_eq!(updated.stable_id, element.stable_id);
    let host = client.host();
    assert!(matches!(
        futures::executor::block_on(host.request_async(Request {
            protocol: PROTOCOL_VERSION,
            runtime_id: host.info().runtime_id,
            key: None,
            message: Message::Command(Command::NavigateElement {
                id: updated.id.clone(),
                generation: after.capture,
                action: crate::inspection::NavigationAction::Focus
            })
        }))
        .unwrap(),
        Reply::Acknowledged
    ));
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
    });
    assert!(
        client
            .inspection()
            .snapshot()
            .elements
            .iter()
            .any(|e| e.semantic_id.as_deref() == Some("content") && e.navigation.focused)
    );
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
    assert!(matches!(
        futures::executor::block_on(host.request_async(Request {
            protocol: PROTOCOL_VERSION,
            runtime_id: host.info().runtime_id,
            key: None,
            message: Message::Query(crate::control::Query::Info {})
        })),
        Err(crate::control::ControlError::RuntimeShutdown)
    ));
}

#[test]
fn disabled_policy_returns_the_original_root_without_creating_a_host() {
    let app = gpui::TestAppContext::single();
    app.update(|cx| {
        let content: AnyView = cx.new(|_| gpui::Empty).into();
        let id = content.entity_id();
        let services = crate::App::new("standalone.disabled").services().unwrap();
        let root = mount(
            content,
            &services,
            ControlConfig::Disabled,
            "standalone.disabled".into(),
            cx,
        );
        assert_eq!(root.entity_id(), id);
    });
}

#[test]
fn inspection_edits_invalidate_content_and_draw_text_styles_reset_and_undo() {
    use crate::inspection::{StyleDeclaration, StyleEdit, TextEdit};
    use futures::{FutureExt, StreamExt};

    let mut app = gpui::TestAppContext::single();
    let client = InspectionClient::new(Collector::default());
    let mut content = None;
    let (_, cx) = app.add_window_view(|_, cx| {
        let view = cx.new(|cx| Content {
            width: 120.,
            focus: cx.focus_handle(),
        });
        content = Some(view.clone());
        InspectionRoot::new(
            view.into(),
            client.clone(),
            ControlConfig::Disabled,
            "standalone.edits".into(),
            cx,
        )
    });
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    let mut invalidations = cx.notifications(&content.unwrap());
    let host = client.host();
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
                futures::executor::block_on(host.request_async(Request {
                    protocol: PROTOCOL_VERSION,
                    runtime_id: host.info().runtime_id,
                    key: None,
                    message: Message::Command(command),
                }))
                .unwrap(),
                Reply::Acknowledged
            ));
        }
        cx.run_until_parked();
        assert_eq!(
            invalidations.next().now_or_never(),
            Some(Some(())),
            "inspection edits must invalidate the content entity that GPUI tracks for redraw"
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
    }
}
