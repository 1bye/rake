//! Retains Base input state and displays edit summaries.

use rake::gpui::{
    self, Context, Entity, FocusHandle, Focusable, IntoElement, Render, SharedString, Subscription,
    Window, accesskit::Role, div, prelude::*, px,
};

use rake::ui::input::{Input, InputEvent, InputState, Textarea, TextareaState};

pub(super) struct Inputs {
    message: Entity<InputState>,
    password: Entity<InputState>,
    notes: Entity<TextareaState>,
    _changes: [Subscription; 3],
}

impl Inputs {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let message = cx.new(|cx| InputState::new(window, cx).placeholder("Type a message"));
        let password = cx.new(|cx| InputState::new(window, cx).masked(true));
        let notes = cx.new(|cx| TextareaState::new(window, cx).placeholder("Write a few lines"));

        let changes = [
            cx.subscribe(&message, |_, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe(&password, |_, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe(&notes, |_, _, event, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];

        Self {
            message,
            password,
            notes,
            _changes: changes,
        }
    }
}

impl Render for Inputs {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let message = self.message.read(cx);
        let password = self.password.read(cx);
        let notes = self.notes.read(cx);
        let message_value = message.value();
        let notes_value = notes.value();
        let message_length = format!("Message characters: {}", message_value.chars().count());

        let notes_summary = format!(
            "Note characters: {} across {} lines",
            notes_value.chars().count(),
            notes_value.lines().count()
        );

        let password_status = if password.value().is_empty() {
            "Password field empty"
        } else {
            "Password entered"
        };

        rake::view! {
            <section id="base-inputs" class="flex flex-col gap-2 text-ink">
                <p>"Message"</p>
                <div style={|s| s.child(field("base-message", "Message", Role::TextInput,
                    message.focus_handle(cx), Some(message_value), Input::new(&self.message)))}/>
                <p class="text-xs">{message_length}</p>
                <p>"Password"</p>
                <div style={|s| s.child(field("base-password", "Password", Role::PasswordInput,
                    password.focus_handle(cx), None, Input::new(&self.password)))}/>
                <p class="text-xs">{password_status}</p>
                <p>"Notes"</p>
                <div style={|s| s.child(field("base-notes", "Notes", Role::MultilineTextInput,
                    notes.focus_handle(cx), Some(notes_value), Textarea::new(&self.notes)))}/>
                <p class="text-xs">{notes_summary}</p>
            </section>
        }
    }
}

fn field(
    id: &'static str,
    label: &'static str,
    role: Role,
    handle: FocusHandle,
    value: Option<SharedString>,
    input: impl IntoElement,
) -> impl IntoElement {
    let focus = handle.clone();
    let colors = rake::theme::current::<super::Colors>();

    let height = if role == Role::MultilineTextInput {
        84.
    } else {
        36.
    };

    let frame = div()
        .id(id)
        .role(role)
        .aria_label(label)
        .when_some(value, |frame, value| frame.aria_value(value))
        .on_a11y_action(gpui::AccessibleAction::Focus, move |_, window, cx| {
            focus.focus(window, cx);
        })
        .flex()
        .w_full()
        .h(px(height))
        .px_2()
        .border_1()
        .border_color(colors.line)
        .rounded_md()
        .child(input);

    #[cfg(feature = "devtools")]
    let frame = rake::inspection::inspect(frame)
        .inspect(id)
        .focus_handle(&handle);

    frame
}
