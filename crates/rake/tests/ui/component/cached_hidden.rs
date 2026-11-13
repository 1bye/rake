#[rake::component(cached = "w-40 h-10")]
fn Invalid(cx: &mut rake::reactive::Setup) -> impl rake::gpui::IntoElement {
    rake::view! { <div class="hidden w-40 h-10"/> }
}

fn main() {}
