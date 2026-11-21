#[rake::component(cached = "w-40 h-10")]
fn Invalid(cx: &mut rake::reactive::Setup) -> impl rake::gpui::IntoElement {
    rake::view! { <div class="absolute left-4 top-4 w-40 h-10"/> }
}

fn main() {}
