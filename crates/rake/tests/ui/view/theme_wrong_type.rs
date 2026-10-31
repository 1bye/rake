#[derive(Clone, rake::Theme)]
struct Colors { ink: rake::gpui::Hsla }
rake::theme!(Colors);
fn main() { let _ = rake::view! { <div class="font-ink"/> }; }
