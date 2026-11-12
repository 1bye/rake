#[rake::component(unknown)]
fn Invalid(cx: &mut rake::reactive::Setup) -> impl rake::gpui::IntoElement {
    rake::gpui::div()
}

fn main() {}
