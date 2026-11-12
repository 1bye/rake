# Full-stack GPUI: semantic HTML, server functions and preloaded data

Status: proposed. Nothing here is implemented. It is written against `main` at
`e78d73f` and describes a future direction, not a compatibility guarantee.

## Summary

Rake's browser target draws to a GPUI canvas, so it cannot hydrate server-rendered
markup the way React does. It can render an application's real layouts headlessly and
emit their meaning as static HTML: correct headings, landmarks, lists and links. The first
version covers `view!` content and fails the build on content it cannot represent.

| Application need | Rake feature | Needs a server? |
| --- | --- | --- |
| Link previews, titles, search metadata | Route metadata and static shells | No |
| Something visible before Wasm starts | Static shells | No |
| Indexable, readable page content | Semantic HTML from layouts | No, for build-time routes |
| Secrets, databases, private APIs | Server functions | Yes |
| Data available on first load | Preloaded loader data | Yes |

These build in order. Each is useful without the next, and a static host remains a
supported deployment for applications that only want the first two.

## Non-goals

- Hydrating HTML into GPUI, or streaming pixels or a draw stream from a server.
- Visual parity. The HTML is a readable document, not a copy of the canvas.
- React Server Components, streaming UI or partial hydration.
- Edge or serverless runtimes, incremental static regeneration and caching layers.
  These can follow if real applications need them.
- A built-in authentication provider or ORM. Rake defines the boundary only.

## Existing foundations

- `Route` lives in `rake-protocol`, has no GPUI dependency and shares `to_uri` and
  `from_uri` with the browser adapter, so a server can decode the same URLs.
- `RouteDescriptor` is a closed registry of stable IDs, path templates and parameters.
- `ApplicationCommand` already has stable IDs, argument and result JSON Schemas,
  Serde decoding and a `Headless` capability. `CommandContext` carries services and
  a cancellation token. Agents and the CLI already discover and call these.
- `LoadedPage` separates asynchronous `load` from `mount`, with typed `Data`.
- `rake bundle --target web` emits a content identity, `rake-version.json` and
  immutable `rake-builds/<content-id>/` payloads. `Deployment` detects changed builds
  and `reload_web_update` reloads through the guarded quit path.
- CI already builds and tests `rake` with `--no-default-features`, which is the
  starting point for a server that does not link GPUI.
- `view!` already assigns AccessKit roles: `h1`-`h6` become `Role::Heading` with a
  level, `nav`, `main`, `header`, `footer`, `aside` and `article` become landmarks, and
  `ul`, `ol`, `li`, `p`, `button` and `img` have roles or labels. Kit declares roles too.
- Rake's inspection captures per-element roles, labels, text and authored tags, using the
  public `a11y_role` and `write_a11y_info` methods. A spike has shown this works headlessly.
- `gpui_web` at this pin has no accessibility integration that we could find, so the
  browser currently exposes no GPUI semantics to assistive technology.

## Phase 1: route metadata and static shells

Add optional static metadata to the route derive:

```rust
#[derive(Clone, Route)]
enum Destination {
    #[route(id = "home", path = "/", title = "Fieldnotes",
            description = "Private notes with search and crash recovery")]
    Home,
    #[route(id = "document", path = "/documents/{id}", title = "Document", index = false)]
    Document(u64),
}
```

`RouteDescriptor` gains a `meta` field with `title`, `description`, `image` and
`index`. Its public fields make this a source-breaking change for manual
descriptors, which is acceptable during the alpha.

`rake bundle --target web` then writes:

- One `index.html` per route without required parameters, and one fallback shell for
  parameterized routes. Each has a title, description, canonical URL and Open Graph
  tags, plus a small visible loading state and `<noscript>` text.
- `sitemap.xml` and `robots.txt` when `base_url` is configured. Sitemaps include
  only indexable routes without required parameters.

The bundler must read descriptors without running the application. The preferred
approach is a Wasm custom section emitted by the route derive and read from the
built module, so no native build is needed. Dynamic titles require a server and
belong to Phase 4. The running application should also update `document.title` on
navigation from the same metadata, if it does not already.

Shells must keep the current deployment rules:

- All metadata is HTML-escaped. The Content Security Policy does not change.
- Shells reference immutable build directories, so an old shell can still start an
  old build. A shell belongs to the content identity.
- Offline bundles precache every generated shell. Navigation keeps its network-first
  policy, and an uncached route falls back to the generic shell.

**Done when:** `curl` of each route returns its own metadata on a plain static host,
the application starts identically from every shell, hash routing and offline mode
still work, and hostile metadata strings are covered by escaping tests.

## Phase 2: semantic HTML from layouts

Render a route's real layout in a headless GPUI window, capture its semantic content, and
write it as HTML inside that route's shell. The application's views stay the only source of
truth, so the HTML cannot drift from them. Generating HTML directly from `view!` was rejected:
it would be a second renderer that ignores runtime state and component output.

The first version covers `view!` content completely and refuses to lose anything else silently.

### Spike findings

`codex/headless-html-spike` tested this against a routed `LoadedPage` at this pin. Its full report
is `.agents/reports/headless-html-spike.md` on that branch.

| Source | Result |
| --- | --- |
| Rake inspection, with no GPUI patch | Correct landmarks, heading levels, lists including `ol`, image alternatives, buttons, dynamic and loaded text. Misses Kit descendants and plain GPUI text |
| Real AccessKit tree, with a temporary GPUI patch | Sees Kit's button and table structure. Loses table cell text, plain GPUI text, list kind and decorative images |

- Capture ran with no GPU renderer in about 34 ms per route, and ten processes produced
  byte-identical HTML. These are unoptimized single-machine measurements, not production costs.
- The real tree is private and inactive when headless. Reading it needs a GPUI change.
- Opening the tree would not be enough. Content with no semantic node cannot be mapped, so
  Phase 2 needs an explicit coverage contract rather than an assumption that roles cover everything.
- Inline `strong` and `em` were flattened by both sources.

### Capture source

Use Rake's inspection data and its authored `view.tag`, normalized into a portable tree before
any HTML is written:

```text
Route -> headless window -> layout + page -> settled frame -> semantic tree -> coverage check -> HTML
```

Inspection is bounded for interactive use today, including a 16 KiB text prefix and 256-byte names.
A `prerender` capture must have no such truncation, and must fail rather than cut content. The tree
contains no IDs, timestamps or provenance, and follows child order rather than map or vector order.

### Mapping

| Authored element or semantic node | HTML |
| --- | --- |
| `h1`-`h6` | The same tag |
| `main`, `nav`, `header`, `footer`, `aside`, `article` | The same tag |
| `p`, standalone text | `p`, or text |
| `ul`, `ol`, `li` | The same tag |
| `img` with a nonempty `alt` | `img` with a bundled `src` when available; otherwise the `alt` text |
| `img` with `alt=""` | Omitted |
| `a route` | `a href` |
| `button` | `button disabled` |
| Other named node | `div` with the matching ARIA `role` and label |

Text is always escaped. `aria-label` is omitted when it only repeats the visible text. An element's
kind must come from an explicit authored or semantic fact, never from a guess based on names or
content. Inline emphasis is flattened in the first version and documented as such. Beyond the shell's
background and text colors, no application style is copied.

### Coverage contract

A page that silently omits text looks correct and misleads search engines and screen-reader users,
so missing content is a build error. Every captured leaf must be one of:

1. A known semantic element or text node from `view!`.
2. A boundary with explicit static content.
3. A boundary explicitly excluded with a reason.

Anything else, such as an opaque Kit component, a type-erased child or an embedded plain GPUI
element, fails that route with its element type and construction site:

```rust
// Provide typed content for the static document.
kit_table.inspect("inventory").static_content(
    rake::html::table(["Item", "Count"], [["Pears", "7"]]),
)

// Or state that it has no meaning in a document.
activity_chart.inspect("activity").static_excluded("decorative preview")
```

`rake::html` builds typed, escaped nodes. Raw HTML strings are not accepted. These calls compile to
no work without the `prerender` feature. The build report lists all exclusions, and agents can read
it as JSON.

Detecting plain GPUI text inside custom elements and components needs testing. `view!` instruments
embedded expressions, but a custom `Element` may paint text with no boundary. If it cannot be
detected reliably, the documentation must name this limit, and the fixtures must show exactly what
gets through.

### Links need new work

`view!` has no `a` tag today. Crawlers and screen readers need real links, so add
`<a route={Destination::Document(1)}>`, which navigates through the router on click and sets
`Role::Link` with a URL from `Route::to_uri`. Without this, every page in the HTML is a dead end.

### Choosing what renders

```rust
#[route(id = "pricing", path = "/pricing", prerender)]
Pricing,
#[route(id = "post", path = "/posts/{slug}", prerender = post_slugs)]
Post { slug: String },
```

- Prerendering is opt-in per route. Private routes keep the plain shell, so an authenticated page
  can never end up in a public static file.
- `prerender = post_slugs` names an application function listing the parameters.
- `LoadedPage` loaders run at build time, so they need build-time services.

### Readiness and failures

The first version captures after the route operation returns `Outcome::Committed`, with its loader
complete and mounted, followed by one frame. The spike observed this directly instead of using a
delay. Work the page starts on its own, image decoding and animations are not awaited.

A route fails its build on a loader error, a navigation that is blocked or redirected, a deadline, a
truncation or a coverage error. No error page is ever published as content. A fuller readiness contract
can follow if real pages need it.

### Export host

Reading routes from Wasm is not enough here, because the layouts must run. `rake bundle --target web`
also builds the application natively with a `prerender` feature and runs it once per route.

- Use the GPU-free `HeadlessAppContext`, which needs `gpui/test-support`. Keep that behind the
  build-only feature, and never in shipped binaries. On macOS, `gpui_platform::headless()` still
  creates a native window and renderer, so calling it is not enough.
- The spike's synthetic text system does not shape real text. It matched real macOS fonts on this
  fixture, but a view that branches on measured text may differ. Support a real text system if needed.
- The spike hosted its layout through `Router::in_window`, because `App::layout` is tied to the
  platform runner. Rake needs a supported entry point that runs a normal `App` for one route.
- The first supported host is macOS. Linux must be tested before it is promised for build machines.

Request-time rendering is deferred. Build-time routes come first, and any later server rendering
should be measured on real routes.

### What the browser does with it

The HTML is visible until the first canvas frame, then visually hidden but kept in the document rather
than removed, for three reasons:

1. `gpui_web` exposes no semantics, so it is the only thing assistive technology can read.
2. A crawler that does run the Wasm still finds the content.
3. If WebGPU or Wasm fails, the page remains readable. The application reveals it again.

On client navigation, the application replaces the fragment with the new route's prebuilt one, updates
`document.title` and moves focus to the new `h1`. Routes without prebuilt HTML get an empty fragment
and a title.

This gives **readable** accessibility, not **operable** accessibility. A screen reader user can read the
page and follow links, but cannot press canvas controls or hear state changes, and the documentation must
say so. Operable accessibility needs a live layer that mirrors semantics into hidden DOM and routes its
actions back, which is a project of its own. The mapping should be a reusable function so that layer can
share it.

### Document checks

Because the tree is available at build time, `rake bundle` can check each prerendered route and fail on a
missing or repeated `h1`, skipped heading levels, unnamed buttons or links, and pages with no `main`.

### Upstream work

These run in parallel and do not block the first version:

- **GPUI:** a stable, typed way to activate and read the accessibility tree without a window.
- **Kit:** text in table cells. The spike's fixture produced four unnamed, childless cells.
- **Native accessibility:** the same tree feeds native screen readers, so missing cell text and plain GPUI
  text may also be real VoiceOver bugs. Test this first, because it decides how the fixes are argued.
- **Rake:** set a list-kind property from `ol` and `ul`, so that the real tree carries it too.

Once a stable tree exists, it can supply content for opaque boundaries, so Kit-heavy pages need fewer
`static_content` calls. Rake's authored data stays authoritative where both sources have an answer.

**Done when:** with Wasm disabled, `curl` and a real screen reader get correct headings, landmarks, lists and
links for each prerendered route; output is byte-identical across builds; an uncovered Kit component and an
oversized text value both fail the build; a failed WebGPU start leaves a readable page; client navigation
replaces the fragment and moves focus; and negative fixtures pin each known loss. Verify with VoiceOver in
Safari and Chrome, since a passing tree test does not prove what a screen reader announces.

## Phase 3: server functions

A server function is an `ApplicationCommand` that an application chooses to expose
over HTTP. It is not a new concept: the same type gives agents, the CLI and clients
one schema.

### Keeping server code out of clients

Use separate crates:

```text
my-app-shared   Routes, command types, result types
my-app          Client: depends on shared and rake
my-app-server   Handlers, secrets, database: depends on shared and rake-server
```

A client cannot compile server code it does not depend on. This is the main reason
to prefer crates over a macro that strips function bodies with `cfg`, as other Rust
frameworks do: there is no way to get the split subtly wrong. A convenience macro can
come later if the crates prove too heavy. `rake check` should fail if a client's Wasm
dependency tree contains a crate marked as server-only, in the style of
`scripts/check-gpui-pins.py`.

### Defining and calling

```rust
// my-app-shared
#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "notes.search", output = Vec<Note>, capability = Headless)]
#[serde(deny_unknown_fields)]
pub struct SearchNotes { pub query: String }

// my-app-server
fn compose() -> Result<Server, ServerError> {
    let commands = Commands::new();
    commands.register::<SearchNotes, _, _>(search_notes)?;

    Server::new(commands)
        .expose::<SearchNotes>(Access::Authenticated, Retry::Idempotent)
        .authenticate(MySessions::new())
        .static_bundle("dist/web")
}

// my-app
let notes = page.server().call(SearchNotes { query }).await?;
```

`page.server()` uses `rake-http`, so requests get its cancellation, deadlines and
DevTools network inspection. The same call works from native applications, given a
configured base URL. It works in loaders, actions and commands.

### Exposure is explicit

Registered commands are not public. Local control commands can be powerful and are
designed for a trusted developer, so only `.expose` publishes a command to HTTP.
Each exposure must state its `Access` (`Public` or `Authenticated`) and has no default.
Client guards and `entry_admission` remain user experience, not security: every
server function enforces its own authorization.

### Transport

`POST {base_path}/__rake/rpc/{command-id}` with a JSON body.

- **Limits:** bounded body size, concurrency and execution deadline per exposure.
- **Browser requests:** require `application/json`, a matching `Origin` and a custom
  `x-rake-rpc` header, which together block cross-site form posts.
- **Native requests:** use bearer credentials supplied by the application.
- **Errors:** return a typed envelope. `Arguments` and application errors reach the
  client. Internal failures return a request ID only, with detail in server diagnostics.
- **Build compatibility:** clients send their content identity. A browser build the
  server no longer supports receives `UpdateRequired`, which connects to the existing
  `Deployment` and guarded reload flow. Long-lived native clients need additive schema
  changes, since they cannot be force-reloaded.
- **Retries:** none for mutations. A call may be replayed automatically only if its
  exposure says `Retry::Idempotent`. A lost response must never resend a mutation,
  matching the guarantee Rake already tests in its CDP evaluate path.
- **Cancellation:** dropping a client call aborts the request and drops the server
  future. As `CommandContext` already documents, this cannot undo completed effects.

`CommandContext` gains an optional `caller` holding the principal and request ID.
It is `None` for local and agent calls, so existing handlers keep their signature.

### Server runtime

`Commands` uses `Rc`, and handlers return local futures. That is deliberate and also
matches UI code, so the server should not force `Send` on applications:

- Each worker thread has a current-thread executor with a local task set, and its own
  `Commands` and services, created by calling the application's `compose` per worker.
- Shared resources such as connection pools live behind application-owned `Arc`s.
- HTTP accept and parsing hand each request to a worker over a channel.

A new `rake-server` crate would own this, using `rake` without its `gpui` feature, and
would also serve the bundle: deep-link shell fallback, immutable asset headers and
`no-store` on the root index, worker script and `rake-version.json`, replacing the
generated nginx example for simple deployments. It should expose a standard HTTP
service so applications can mount it inside an existing server.

### Agent tools

Schemas stay discoverable through the existing command descriptors. The CLI and agents
can call an exposed function in development, and a request ID links a browser network
entry to its server diagnostics. Public schema listing stays off in production unless
an application enables it.

**Done when:** a Fieldnotes variant reads from a server database on web and macOS with
no secret in its Wasm module, and tests cover authorization, CSRF rejection, oversized
bodies, deadlines, cancellation, unsupported builds and no automatic mutation replay.

## Phase 4: preloaded data and dynamic metadata

Loaders run in the client today and may use client-only services. A route opts in by
naming the server function that supplies its first-load data:

```rust
impl PreloadedPage<Destination> for DocumentPage {
    type Preload = LoadDocument;

    fn preload(destination: &Destination) -> LoadDocument { /* arguments */ }
    fn head(result: &Document) -> Head { Head::title(&result.title) }
}
```

On a navigation request, `rake-server`:

1. Decodes the URL with `Route::from_uri`. An unknown route returns a real HTTP 404
   with the shell, which still renders the application's own error view.
2. Runs the exposed function with the request's caller, under a short deadline.
3. Embeds the result as `<script type="application/json" id="rake-preload">`, keyed by
   route address and content identity, and applies `head`.

The client's loader takes the matching value once, on the first navigation only. On a
missing, mismatched or failed preload it calls the same function normally, so preload
stays an optimization and can always be removed.

Security and caching rules:

- Escape the embedded JSON so that `</script>` and `<!--` cannot break out.
- Responses containing caller-specific data use `Cache-Control: private, no-store`.
- The service worker must never store a preloaded response. It caches the generic
  shell only, so private data never persists on a shared device.
- Bound the embedded size and omit the preload when it is exceeded.
- Preload runs for each document request, so it needs the same limits as RPC.

**Done when:** first navigation to a document makes no RPC call, reload and direct links
agree with client navigation, a stale build falls back cleanly, and no response with
caller data is cached by the worker.

## Deployment matrix

| Capability | Static host | `rake-server` | Native client |
| --- | --- | --- | --- |
| Route metadata and static shells | Yes | Yes | Not applicable |
| Semantic HTML for build-time routes | Yes | Yes | Not applicable |
| Offline bundle | Yes | Yes | Not applicable |
| Server functions | Against a separate origin only | Yes | Yes |
| Preloaded data, dynamic metadata | No | Yes | No |

Cross-origin server functions need an explicit origin allowlist and are deferred until
same-origin works.

## Risks and open questions

- **Kit-heavy pages:** until Kit and GPUI change, they need `static_content` for most
  components. That is honest but laborious, and it may disappoint applications built mostly from Kit.
- **Undetectable text:** custom elements can paint text with no boundary, which the coverage
  contract may miss. Fixtures must establish what it can and cannot catch.
- **Build environment:** a truly GPU-free and windowless run, Linux, other fonts and release
  builds are all unverified. So is any real screen reader.
- **Hidden content policy:** hidden HTML must match what the application shows, or search
  engines may treat it as cloaking. One source of truth should keep it honest, but this
  needs checking against current search-engine guidance.
- **Wasm custom sections:** confirm that they survive `wasm-bindgen` and the production
  optimization steps. If not, export a pure function and read it during bundling.
- **Headless runtime:** confirm `AppHandle` and `Commands` run without a window under a
  server's executor, not only in today's headless tests.
- **HTTP dependency:** choose the server library when Phase 3 starts, and keep it out
  of the dependency tree of client builds. Extend the pin and graph checks to prove it.
- **Service lifetime:** per-worker services are simple but may surprise users with
  in-process state. Document that a request cannot assume a particular worker.
- **Perceived speed:** shells show content sooner, but the application still needs Wasm
  and GPU setup. Measure time to first canvas frame before and after, and do not market
  this as faster startup.
- **Native compatibility:** old native clients can outlive server releases, which needs
  a real schema-evolution policy before Phase 3 is declared stable.
- **Positioning:** semantic HTML makes public pages readable and indexable, but a canvas
  application remains heavier than a plain site. A content-only site should still consider
  plain HTML, and the documentation should say so.
