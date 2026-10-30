# Chrome Sources

Sources provides read-only browsing of captured Rust files. It does not register
Rust as JavaScript or WebAssembly, and does not expose breakpoint or execution
controls. The native CDP document is a projection; source documents in its frame
tree are catalog entries, not application windows or browser iframes.

Delivery proceeds in five separate commits:

1. A bounded catalog of captured Rust files and source-document navigation.
2. Confined source loading with stable workspace paths and explicit failures.
3. File/content search and line navigation through Chrome's resource APIs.
4. Consistent source locations across Elements, Console, Network and Performance.
5. Replacement/expiry hardening and verification in stock Chrome.

The catalog admits at most 128 distinct workspace Rust files per connection and
runtime. Files outside `RAKE_SOURCE_ROOT` (default: current workspace directory)
are unavailable. Capturing the same file at different lines does not duplicate it.

Source URLs use the authenticated loopback endpoint and percent-encoded workspace
paths. The endpoint accepts only files admitted by a live inspection connection;
knowing a path inside the workspace is insufficient. Both HTTP and CDP reads
reapply workspace/symlink confinement and the existing 256 KiB UTF-8 file limit.
Contents are read from the workspace on request, rather than reconstructed from
the compiled binary. Editing source on disk can therefore precede runtime replacement.
Source streams retain at most eight files, return UTF-8-safe chunks and reject
invalid offsets/sizes. Dropping the owning session revokes its file grants.
