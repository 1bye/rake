#![deny(deprecated)]
fn main() {
    let _ = rake::view! { <button onclick={|_, _, _| {}}>"Old"</button> };
}
