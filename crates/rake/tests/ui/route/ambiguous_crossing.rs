#[derive(Clone, rake::Route)]
enum Destination {
    #[route(id = "file", path = "/files/{id}")]
    File(String),
    #[route(id = "settings", path = "/{section}/settings")]
    Settings(String),
}

fn main() {}
