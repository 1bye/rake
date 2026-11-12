#[rake::component]
fn Invalid(cx: &mut rake::reactive::Setup, value: i32) -> impl rake::gpui::IntoElement {
    rake::gpui::div()
}

fn main() {}
