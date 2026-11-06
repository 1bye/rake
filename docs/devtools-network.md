# Chrome Network

Network inspects requests made through Rake's HTTP client. Requests from unrelated
transports are not captured. Rich capture is compiled only with `rake/devtools`;
production clients retain their existing redacted lifecycle diagnostics.

## Request details

Chrome receives methods, URLs with query parameters, request and response headers, status, final response URL, MIME type and the native transport's HTTP version.

- Request headers are the built request's provisional headers. Later transport-added defaults and raw on-wire text are unavailable.
- Credential URL components are removed. Authorization, cookies and common credential query parameters are redacted.
- Each header block is capped at 16 KiB.
- Capture never modifies the transmitted request.
- Browser Fetch still restricts visible headers.
- Redirects appear as one request with the final URL, because the transport doesn't expose individual hops.

## Bodies

Chrome's Payload, Preview, Response and body-copy workflows use retained original bytes. UTF-8 responses are text, and binary responses use CDP base64. Request post data is available for UTF-8 bodies.

Capture keeps at most 64 KiB per body, 64 requests and 1 MiB per collector, evicting the oldest. Bodies are separate from diagnostics, and development diagnostic snapshots have their own 512 KiB ceiling to fit private IPC replies. Production compiles out capture storage and hooks.

Response capture sees only bytes your app consumed, without reading ahead. Pending, failed, cancelled, abandoned, truncated, evicted or contended captures return explicit errors rather than passing off a prefix as a complete body. The adapter never replays a request to get a missing body.

> Raw bodies can contain sensitive data, and unlike credential headers and query parameters, they aren't redacted. The development feature is the opt-in boundary.

## Timing and origin

The waterfall uses collector-monotonic request, headers, chunk and completion observations. Timing covers elapsed time to response headers and application body consumption, not CPU time or wire transfer.

DNS, connect, TLS, proxy and exact send phases are unavailable and use CDP's unavailable values. Chrome labels the unsplit pre-headers interval “Stalled”, which is unattributed client time, not measured stalling. Encoded wire sizes aren't measured, and byte counts describe consumed bytes.

Initiator links point at the real Rust request-construction call site. The stack presentation carries that location and captured span names, plus command, operation, task and generation IDs when instrumented. It isn't a JavaScript or Rust backtrace. Links use the same workspace-confined read-only source mechanism as Console, and work without Console enabled.

## Filtering, copying and lifecycle

Chrome's native URL/method/status/type filters, header copying, Copy as cURL,
Copy response and HAR export operate on captured data. Credential fields stay
redacted in copies; truncated or unavailable bodies are not silently fetched again.
Response-body search supports literal/regex queries and case sensitivity with a
bounded regex engine, returning up to 1,000 matching lines.

Request IDs include their runtime UUID, loader IDs follow the virtual frame, and the waterfall timeline continues across replacement.

- Chrome Clear removes rows locally, and cursors stop them reappearing.
- Stopping and resuming recording skips traffic produced while paused.
- Preserve log keeps rows across checkpointed replacement. It doesn't preserve or replay application-side requests.
- Each connection keeps completed bytes across replacement (64 requests, 1 MiB), so recent rows stay readable until the connection closes. Evicted bodies report an error.
- Old request IDs never retarget a successor with the same local counter.

Browser cache, service workers, request blocking, throttling and cookie mutation
are not Rake HTTP controls; unsupported mutations return errors. This tab does not
monitor traffic made through unrelated clients, browser internals or raw sockets.

## Verification

```sh
cargo build -p rake-app-cli -p rake-routing --features rake-routing/network-test --offline
node scripts/test-network-cdp.mjs
node scripts/test-network-cdp.mjs --serve
```

The native harness uses a real supervised GPUI app and Rake HTTP requests against a deterministic loopback server, and cleans up only its own processes, trigger asset and session directory. It doesn't automate Chrome.

For a manual pass, `--serve` prints a stock Chrome URL and starts with an empty log. In the native example, click GET JSON, POST JSON, GET binary, HTTP 503, Slow request or Timeout. Cancel request interrupts the slow request, and the app shows its result. `seed`, `restart` and `quit` on stdin repeat automated scenarios.

Stock Chrome **156.0.8078.74**, on 2026-10-30, was exercised through its bundled
DevTools UI against the native fixture:

See [test coverage](internal/test-coverage.md) for what's been checked in stock Chrome.

Native HTTP tests also cover feature-off redaction and cancellation, and diagnostic
tests exercise byte/count bounds. Production inspection-off tests and WASM builds
with/without `devtools` are checked separately; WASM compilation does not establish
browser Fetch or GPU execution.
