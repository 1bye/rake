#[rake::component]
fn Invalid(id: String, cx: &mut rake::reactive::Setup) -> impl rake::gpui::IntoElement {
    rake::gpui::div()
}

fn main() {}
