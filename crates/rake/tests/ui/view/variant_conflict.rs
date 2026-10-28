fn main() {
    let _ = rake::view! { <div class="hover:opacity-50" style={|s| s.hover(|s| s.opacity(0.5))}/> };
}
