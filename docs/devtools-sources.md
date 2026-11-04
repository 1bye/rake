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

Source URLs use the authenticated loopback endpoint and percent-encoded workspace paths. The endpoint accepts only files admitted by a live inspection connection, so knowing a workspace path isn't enough.

- HTTP and CDP reads both reapply workspace and symlink confinement and the 256 KiB UTF-8 file limit.
- Contents come from the workspace on request, not the compiled binary, so editing source on disk can precede runtime replacement.
- Source streams keep at most eight files, return UTF-8-safe chunks and reject invalid offsets and sizes.
- Dropping the owning session revokes its file grants.

Chrome's Open file picker uses the captured file catalog, and Go to line works on the loaded text.

**Search.** Resource content search supports case-sensitive or insensitive text and Rust-regex syntax through `Page.searchInResource`, with zero-based line numbers and full matching lines. Queries are limited to 4096 bytes. Searches over 1000 matching lines fail explicitly, so narrow the query rather than accepting partial results.

**Opening files.** `Rake.getSourceLocation` and `Rake.openSource` also accept `{url,lineNumber,columnNumber}` for a captured source URL. CDP coordinates are zero-based with UTF-16 columns, and invalid lines, columns and split surrogate pairs are rejected. The system default editor opens the file, and its association determines whether line navigation works.

Console diagnostic links, Network Rust initiators and Performance callback links use the same canonical file URL when the source is in the workspace. Frame and diagnostic sequence URLs remain a compatibility fallback, and native line and column metadata stays attached to each call site.

Elements node properties also expose `rakeSourceURL` and a clickable `rakeSourceLocation`. `rake.source($0)` returns the captured URL and location and accepts style and handler source selectors. `rake.openSource` uses the system default editor. The adapter still rejects arbitrary evaluation and executable script editing.

The catalog has a 256 KiB metadata budget as well as the file count limit. `rake.sources()` and `Rake.getSources` expose admitted files, runtime identity and whether the budget omitted files.

- Source documents publish in batches of 16, and republish after outbound event loss.
- Newly captured files use `Page.documentOpened`, which Chrome needs to add their document resources.
- Publication waits for Chrome's initial resource tree.
- Replacement clears the old catalog, aliases and streams before admitting new sources, and detaches old documents so stale entries leave Quick Open.
- Stream handles never retarget a later file, and expired URLs fail instead of reading the replacement runtime's file.

For a native regression run, build `rake-cli` and run `node scripts/test-sources-cdp.mjs` on a macOS GUI host with Node 22 or newer. It uses real GPUI captures and doesn't automate Chrome. `--serve` keeps an example running and accepts `work`, `log`, `restart` and `quit` on stdin for manual Chrome checks.

Rust resources are plain text. Syntax highlighting, breakpoints, stepping, local variables and source edits are outside this adapter's coverage. See [test coverage](internal/test-coverage.md) for what's been checked in stock Chrome.
