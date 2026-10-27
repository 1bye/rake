fn main() {
    let _ = rake::view! { <div>for n in [1] { match n { _ => { <span/> } } }</div> };
}
