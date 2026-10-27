fn main() {
    let count = 42usize;
    let _ = rake::view! {
        <div title="invalid">
            <p>{count}</p>
        </div>
    };
}
