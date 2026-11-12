#[rake::component]
fn Invalid(
    (first, second): (i32, i32),
    cx: &mut rake::reactive::Setup,
) -> impl rake::gpui::IntoElement {
    rake::gpui::div()
}

fn main() {}
