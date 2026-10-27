fn main() {
    let _ = rake::view! { <div on:unknown={|_, _, _| {}}/> };
}
