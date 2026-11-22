#[rake::component]
fn Label(text: String) -> impl rake::gpui::IntoElement {
    text
}

fn main() {
    let _ = rake::view! {
        <Label/>
    };
}
