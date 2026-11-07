# Plain Cargo input fixture

This application has no dependencies, Rake UI/runtime, or build hook. Its own
workspace and checked-in Cargo.lock keep metadata inspection locked/offline.
From the repository root (substitute an absolute fixture path):

```sh
cargo run -p rake-app-cli -- bundle --config /path/to/plain-cargo/Bundle.toml --plan --json
cargo run -p rake-dist --example packager-host -- /path/to/plain-cargo/Bundle.toml
```

The independent packager host dispatches its worker before ordinary argument
handling. It depends on rake-dist, not rake-cli. Native packaging tools are still
required to produce artifacts. The fixture selects app/ZIP/tar for small local
checks; release trust acceptance is separate.
