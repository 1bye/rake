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
