fn main() {
    let _ = rake::view! { <div class={if let Some(value) = Some(1) { "p-4" } else { "p-2" }}/> };
}
