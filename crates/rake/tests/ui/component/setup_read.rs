#[rake::component]
fn Invalid(cx: &mut rake::reactive::Setup) -> impl rake::gpui::IntoElement {
    let count = cx.signal(0);
    let snapshot = count.get(cx);
    rake::gpui::div()
}

fn main() {}
