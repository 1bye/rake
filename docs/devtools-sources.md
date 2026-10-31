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

Chrome's Open file picker uses the captured file catalog. Resource content search
supports case-sensitive/insensitive text and Rust-regex syntax through
`Page.searchInResource`, with zero-based line numbers and full matching lines.
Queries are limited to 4096 bytes. Searches over 1000 matching lines fail explicitly;
narrow the query or search within the opened file instead of accepting partial results.
Chrome's Go to line operates on the loaded text. `Rake.getSourceLocation` and
`Rake.openSource` additionally accept `{url,lineNumber,columnNumber}` for a captured
source URL. CDP coordinates are zero-based, with UTF-16 columns; invalid lines,
columns and split surrogate pairs are rejected. The system default editor opens
the file; its association determines whether line navigation is available.

Console diagnostic links, Network Rust initiators and Performance callback links
use the same canonical file URL when the source is available in the workspace.
Frame/diagnostic sequence URLs remain a compatibility fallback, rather than file
identities in the Sources navigator. Native line/column metadata stays attached
to each call site. Elements node properties additionally expose `rakeSourceURL`
and a clickable `rakeSourceLocation` with line and column;
`rake.source($0)` returns the captured URL and location, and accepts the existing
style/handler source selectors. `rake.openSource` uses the system default editor.
The adapter continues to reject arbitrary evaluation and executable script editing.

The catalog has a 256 KiB metadata budget as well as the file count limit.
`rake.sources()` / `Rake.getSources` expose admitted files, runtime identity and
whether that budget omitted files. Source documents publish in batches of 16 and
are republished after outbound event loss. Newly captured files use
`Page.documentOpened`, which stock Chrome needs to add their document resources.
Replacement clears the old catalog, aliases and streams before admitting the new
runtime's sources. Stream handles never retarget a later file. Expired URLs fail
instead of silently reading the replacement runtime's file.
Publication waits for Chrome's initial resource tree; replacement explicitly
detaches old source documents so stale entries disappear from Quick Open.

For a native regression run, build `rake-cli` and run
`node scripts/test-sources-cdp.mjs` on a macOS GUI host with Node 22 or newer.
This exercises real GPUI captures, source HTTP/CDP reads, confinement, search,
cross-tab links, streams, checkpointed replacement and session grant expiry.
Its HTTP responses come from an owned loopback fixture. It does not automate
Chrome. `--serve` leaves an example running and accepts `work`, `log`, `restart`
and `quit` on stdin for separate manual Chrome checks.

Verified in stock Chrome 156.0.8078.74 on 2026-10-30: Sources navigator, Quick Open
with a line suffix, within-file search, global content search, read-only contents,
and Console/Network links to the corresponding Rust file and line. This includes
a Console file captured after DevTools connected, the Performance callback link
at `performance_fixture.rs:137:14`, and Quick Open after checkpointed replacement:
one current `lib.rs` entry, loading at line 597 without reconnecting. The final
native regression, all 81 CLI unit tests, formatting and warning-free CLI Clippy
checks passed. Rust resources are plain text;
Rust syntax highlighting, breakpoints, stepping, local variables and source edits
remain outside this adapter's coverage. The system editor association was reused,
without a new UI verification of external editor line navigation.
