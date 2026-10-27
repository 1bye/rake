#![allow(dead_code)]
struct Host { count: usize }

fn render(cx: &mut rake::gpui::Context<Host>) {
    let _ = rake::view! {
        <div class="unknown-utility">
            <button onclick={cx.listener(|this, _, _, _| {
                this.missing = 1;
            })}>"Click"</button>
        </div>
    };
}

fn main() {}
