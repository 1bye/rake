fn main() {
    let _ = rake::view! { <div class={match true { true => "p-4" }}/> };
}
