#[rake::component]
fn Invalid(cx: &mut rake::Scope, id: String) -> impl rake::gpui::IntoElement {
    rake::gpui::div()
}

fn main() {}
