fn main() {
    let _ = rake::view! {
        <div title="invalid">
            <span class="unknown-utility"/>
            <img src="icon.png"/>
        </div>
    };
}
