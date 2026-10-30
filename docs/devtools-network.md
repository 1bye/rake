# Chrome Network

Network inspects requests made through Rake's HTTP client. Requests from unrelated
transports are not captured. Rich capture is compiled only with `rake/devtools`;
production clients retain their existing redacted lifecycle diagnostics.

## Request details

Chrome receives methods, URLs with query parameters, request/response headers,
status, final response URL, MIME type and the transport's HTTP version. Credential
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
and exact send phases are unavailable and use CDP's unavailable values. Encoded
wire sizes are not measured; the existing byte counts describe consumed bytes.

Initiator links point at the real Rust request-construction call site. The CDP
stack presentation carries that Rust location and captured span names, command,
operation, task and generation IDs when instrumented. It is not a JavaScript or
Rust backtrace. Links use the same workspace-confined read-only source mechanism
as Console, and work with Network enabled independently of Console.
