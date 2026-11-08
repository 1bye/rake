#![doc = "Two independently owned function counters with keyed reorder and unmount controls."]

use rake::Scope;
use rake::gpui::{Context, IntoElement, Render, SharedString, Window, prelude::*};

#[rake::component]
fn Counter(cx: &mut Scope, label: SharedString) -> impl IntoElement {
    let (count, setter) = cx.use_state(|| 0_i32);
    let doubled = cx.use_memo(count, |value| value * 2);

    rake::view! {
        <section class="flex flex-col gap-2 p-4">
            <h2>{label.clone()}</h2>
            <p>{format!("Count: {count}; doubled: {doubled}")}</p>
            <div class="flex gap-2">
                <button
                    id="increment"
                    aria-label={format!("Increment {label}")}
                    on:click={
                        move |_, _, cx| {
                            let _ = setter.update(cx, |value| *value += 1);
                        }
                    }
                >
                    Increment
                </button>
                <button
                    id="reset"
                    aria-label={format!("Reset {label}")}
                    on:click={
                        move |_, _, cx| {
                            let _ = setter.set(cx, 0);
                        }
                    }
                >
                    Reset
                </button>
            </div>
        </section>
    }
}

struct Demo {
    order: Vec<usize>,
    visible: bool,
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        rake::view! {
            <main class="size-full flex flex-col gap-4 p-6">
                <h1>Function components</h1>
                <p>Increment each counter, then reverse their order. State follows the key.</p>
                <div class="flex gap-4">
                    <button
                        id="reverse"
                        on:click={
                            cx.listener(|this, _, _, cx| {
                                this.order.reverse();
                                cx.notify();
                            })
                        }
                    >
                        Reverse order
                    </button>
                    <button
                        id="visibility"
                        on:click={
                            cx.listener(|this, _, _, cx| {
                                this.visible = !this.visible;
                                cx.notify();
                            })
                        }
                    >
                        Toggle counters
                    </button>
                </div>
                if self.visible {
                    for key in &self.order {
                        <Counter id={("counter", *key)} label={format!("Counter {key}")} />
                    }
                }
                <p>Hiding the counters unmounts them. Showing them again starts fresh.</p>
            </main>
        }
    }
}

#[rake::app]
fn app() -> rake::App {
    rake::App::new("dev.rake.function-components")
        .control(rake::control::ControlConfig::FromEnvironment)
        .root(|_, _, cx| {
            cx.new(|_| Demo {
                order: vec![1, 2],
                visible: true,
            })
            .into()
        })
}
