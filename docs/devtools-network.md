# Chrome Network

Network inspects requests made through Rake's HTTP client. Requests from unrelated
transports are not captured. Rich capture is compiled only with `rake/devtools`;
production clients retain their existing redacted lifecycle diagnostics.

## Request details

Chrome receives methods, URLs with query parameters, request/response headers,
status, final response URL, MIME type and the native transport's HTTP version.
Request headers are the built request's provisional headers; later transport-added
defaults and raw on-wire header text are unavailable. Credential
URL components are removed. Authorization, cookies and common credential query
parameters are redacted; each header block is capped at 16 KiB. Capture never
modifies the transmitted request. Browser Fetch still restricts visible headers.
Redirects currently appear as one request with its final response URL; individual
redirect hops are not exposed by the transport.

## Bodies

Chrome's Payload, Preview, Response and body-copy workflows use retained original
bytes. UTF-8 responses are returned as text; binary responses use CDP base64.
Request post data is available for UTF-8 bodies. Capture retains at most 64 KiB per
body, 64 requests and 1 MiB total per collector, evicting oldest requests. Bodies
are separate from diagnostics; development diagnostic snapshots also have a
512 KiB retention ceiling to fit private IPC replies. Production compiles out
both capture storage and hooks.

Response capture observes only bytes consumed by the application, without reading
ahead. Pending, failed, cancelled, abandoned, truncated, evicted or contended
captures return explicit errors instead of pretending a prefix is a complete body.
Raw bodies can contain sensitive application data: the development feature is the
opt-in boundary; unlike credential headers/query parameters, body contents are not
redacted. The adapter never replays a request to obtain a missing body.

## Timing and origin

The waterfall uses collector-monotonic request, headers, chunk and completion
observations. Timing includes elapsed time to response headers and application body
consumption; it is not CPU time or wire transfer timing. DNS, connect, TLS, proxy
and exact send phases are unavailable and use CDP's unavailable values. Chrome
labels the unsplit pre-headers interval “Stalled”; this is unattributed client time,
not measured connection stalling. Encoded
wire sizes are not measured; the existing byte counts describe consumed bytes.

Initiator links point at the real Rust request-construction call site. The CDP
stack presentation carries that Rust location and captured span names, command,
operation, task and generation IDs when instrumented. It is not a JavaScript or
Rust backtrace. Links use the same workspace-confined read-only source mechanism
as Console, and work with Network enabled independently of Console.

## Filtering, copying and lifecycle

Chrome's native URL/method/status/type filters, header copying, Copy as cURL,
Copy response and HAR export operate on captured data. Credential fields stay
redacted in copies; truncated or unavailable bodies are not silently fetched again.
Response-body search supports literal/regex queries and case sensitivity with a
bounded regex engine, returning up to 1,000 matching lines.

Request IDs include their runtime UUID, loader IDs follow the virtual frame, and
the connection's monotonic waterfall timeline continues across replacement.
Chrome Clear removes rows locally; incremental cursors prevent cleared rows from
reappearing. Stopping and resuming recording skips traffic produced while paused.
Chrome Preserve log keeps rows across checkpointed runtime replacement. Each
connection retains completed request/response bytes across replacement (64 requests,
1 MiB total), so recent preserved rows remain readable. Evicted bodies report an
error; old request IDs never retarget a successor with the same local counter.
This retention ends when the connection closes. Preserve log does not preserve
application-side requests or replay them.

Browser cache, service workers, request blocking, throttling and cookie mutation
are not Rake HTTP controls; unsupported mutations return errors. This tab does not
monitor traffic made through unrelated clients, browser internals or raw sockets.

## Verification

```sh
cargo build -p rake-cli -p rake-routing --features rake-routing/network-test --offline
node scripts/test-network-cdp.mjs
node scripts/test-network-cdp.mjs --serve
```

The repeatable native/CDP harness uses a real supervised GPUI application and Rake
HTTP requests against a deterministic loopback server. It covers headers, query
redaction, Unicode JSON payloads/responses, binary base64, capture limits, actual
Rust source reads, timings, redirects, HTTP/transport failures, cancellation,
timeout, abandonment, recording pause/resume, independent Console Clear and
preserved bodies/identities/timeline across checkpointed replacement. It cleans up
only its own processes, trigger asset and isolated session directory. `--serve`
prints a stock Chrome URL and accepts `seed`, `restart` and `quit` on stdin for
repeating UI checks. This harness does not automate Chrome.

Stock Chrome **156.0.8078.74**, on 2026-10-30, was exercised through its bundled
DevTools UI against the native fixture:

- Eight seeded request rows showed HTTP/transport statuses; Headers and Payload
  displayed repeated response headers, credential redaction, decoded queries and
  the Unicode JSON request body.
- Preview and Response displayed JSON; binary Response used the hex viewer and
  showed the exact `00 FF 80 2A` bytes.
- Initiator showed command/operation/span context. Clicking the source link opened
  the actual Rust file in Chrome. Timing showed the measured interval and the
  unsplit-phase limitation described above.
- `status-code:503` filtered eight requests down to one; response search found
  three matches in two requests. Copy as cURL and Copy response were pasted into
  an inert search field and checked, without executing the copied command.
- Clear removed all rows; only eight newly generated requests reappeared. Keep log
  retained those rows across checkpointed replacement, and an old response loaded
  afterward. New traffic increased the list to sixteen requests.
- Cancelled/timed-out responses displayed explicit incomplete-capture reasons.

The long-running Chrome pass also exposed repeated warnings for evictions of
already-consumed diagnostics. Development snapshots now distinguish history
eviction from unrecorded data; unread sequence gaps still warn. A regression test
was observed failing before the fix and passing afterward. A final Chrome pass
generated 104 requests and confirmed 1,188 history evictions while Console stayed
at eleven messages without the repeated loss warnings.

Native HTTP tests also cover feature-off redaction and cancellation, and diagnostic
tests exercise byte/count bounds. Production inspection-off tests and WASM builds
with/without `devtools` are checked separately; WASM compilation does not establish
browser Fetch or GPU execution.
