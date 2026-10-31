struct Card {
    header: String,
}

fn main() {
    let _ = rake::view! { <Card header={<h2>Profile</h2>}/> };
}
