struct Card {
    header: rake::gpui::AnyElement,
    footer: rake::gpui::AnyElement,
}

fn main() {
    let _ = rake::view! { <Card header={<img/>} footer={<svg/>}/> };
}
