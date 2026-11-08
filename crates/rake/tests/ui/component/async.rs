#[rake::component]
async fn Invalid(cx: &mut rake::Scope) -> impl rake::gpui::IntoElement {
    rake::gpui::div()
}

fn main() {}
