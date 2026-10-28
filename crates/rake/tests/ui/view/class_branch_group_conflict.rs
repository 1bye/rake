fn main() {
    let _ = rake::view! {
        <div class={if true { "group-hover/a:p-1" } else { "group-hover/b:p-2" }}/>
    };
}
