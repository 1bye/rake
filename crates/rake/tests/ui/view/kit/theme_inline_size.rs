#[derive(Clone, rake::Theme)]
struct Colors { body: rake::theme::TextToken }
rake::theme!(Colors);
fn main() { let _ = rake::view! { <p><span class="text-body">Hello</span></p> }; }
