#[rake::component]
fn Invalid<T>(cx: &mut rake::Scope, value: T) -> impl rake::gpui::IntoElement {
    rake::gpui::div()
}

fn main() {}
