//! The same component markup is compiled by both renderer macros.
use super::backend::{Children, Element, Handler, Signal, Theme, finish, finish_button, slot};
use super::view;

pub struct ButtonProps {
    pub id: &'static str,
    pub label: &'static str,
    pub on_click: Handler,
}

pub fn panel(theme: Theme, children: Children) -> Element {
    let _ = theme;
    finish(view! {
        <div class="flex flex-col w-full gap-4 rounded-lg bg-surface p-5">{slot(children)}</div>
    })
}

pub fn button(theme: Theme, props: ButtonProps) -> Element {
    button_with_state(theme, props, None)
}

fn button_with_state(theme: Theme, props: ButtonProps, checked: Option<bool>) -> Element {
    let _ = theme;
    let ButtonProps {
        id,
        label,
        on_click,
    } = props;
    finish_button(
        view! {
            <button
                id={id}
                aria-label={label}
                class="flex flex-row items-center h-9 px-3 rounded-md bg-muted hover:bg-accent focus-visible:bg-accent"
                on:click={on_click}
            >
                {label}
            </button>
        },
        checked,
    )
}

pub fn setting_row(theme: Theme, label: &'static str, children: Children) -> Element {
    let _ = theme;
    finish(view! {
        <div class="flex flex-row w-full items-center justify-between gap-4">
            <p>{label}</p>
            {slot(children)}
        </div>
    })
}

pub fn switch(theme: Theme, signal: Signal) -> Element {
    let checked = signal.checked;
    button_with_state(
        theme,
        ButtonProps {
            id: "notifications",
            label: if checked {
                "Notifications On"
            } else {
                "Notifications Off"
            },
            on_click: signal.toggle,
        },
        Some(checked),
    )
}
