#[derive(Clone, rake::Theme)]
struct Colors { surface: rake::gpui::Rgba }
rake::theme!(Colors);
fn main() { let _ = rake::view! { <div class="bg-bra"/> }; }
