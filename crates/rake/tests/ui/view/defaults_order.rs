fn main() {
    let _ = rake::view! { <Card {..Default::default()} id={"id".into()}/> };
}
