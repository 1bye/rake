fn main() {
    let _ = rake::view! {
        <div class=42 title="invalid">
            <img src="icon.png"/>
        </div>
    };
}
