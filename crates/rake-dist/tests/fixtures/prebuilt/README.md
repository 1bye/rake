# Prebuilt input fixture

Copy an existing loose native executable to `bin/viewer` (`bin/viewer.exe` on
Windows). Keep executable permissions. For example, on Linux:

```sh
mkdir -p bin
cp /bin/true bin/viewer
/path/to/rake bundle --plan --json --target-triple x86_64-unknown-linux-gnu
PATH= /path/to/rake bundle --plan --json --target-triple x86_64-unknown-linux-gnu
```

Use the triple matching the copied executable. Planning inspects its architecture
without Cargo or rustc. No binary blob is checked in; integration tests materialize
this fixture from a native system executable. Packaging still needs native tools.
