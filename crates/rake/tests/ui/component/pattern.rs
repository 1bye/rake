#[rake::component]
fn Invalid(cx: &mut rake::Scope, (first, second): (i32, i32)) -> impl rake::gpui::IntoElement {
    rake::gpui::div()
}

fn main() {}
