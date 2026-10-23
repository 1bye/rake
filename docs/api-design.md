**Status: Rake API design draft.** Rake is the proposed application framework for native GPUI and browser WebAssembly, beginning with routing, independent application tabs, managed work, CLI parity, HTTP, tracing, hot reload, and custom devtools. All Rake signatures, macros, configuration, and commands below are proposed interfaces. The snippets describe the intended developer experience; they are not a compiled implementation or a claim that a published Rake package already provides them.

The core design is that a destination identifies content, a tab owns a history, and a history entry owns one screen instance. Opening a new tab preserves existing tabs. Background execution is an explicit policy for managed work. The GUI, CLI, and devtools use the same application operations.

The framework crate and CLI are named **rake**. Ordinary application code imports framework services and macros through rake. CLI parity means every supported runtime command and diagnostic query has a terminal interface; project commands also cover native and Web development, builds, and packaging. A command reports its execution and platform requirements explicitly. Business operations can run without a GUI when their registered handlers are headless-capable.

# Rake facade and application entry

Rake provides one application-facing crate, with a small prelude and explicit modules for less common APIs. The runtime selects native or browser adapters. Users writing an ordinary portable application do not configure an HTTP executor, browser Fetch bindings, a tracing subscriber, or a devtools request adapter.

```rust
use rake::prelude::*;

#[rake::app]
fn app() -> App<Destination> {
    App::new("com.example.notes")
        .routes(AppRoutes)
        .http(
            HttpConfig::new()
                .base_url("https://api.example.com/")
        )
        .web(WebConfig::new().routing(WebRouting::Path))
}
```

The proposed app attribute and Rake project scaffold provide the target entry point: native process startup or a Wasm export with its generated browser loader. The same application factory is used for both. AppRoutes and Destination are application definitions described below. Rake's App is the framework configuration and entry API; the underlying GPUI context remains available as rake::gpui::App.

Rake's runner installs normal application services and diagnostics. rake dev enables the local inspector and control connection; a production build does not expose that development connection by default. Embedding an application in a larger host has an explicit diagnostics integration option so the host can supply its existing subscriber or collector.

| Surface | Role |
| - | - |
| rake::prelude | Common application, router, task and service types, including HttpClient. |
| rake::router | Destinations, navigation operations, tabs and histories. |
| rake::http | Rake-owned request, response, error and client configuration types. |
| rake::diagnostics | Collector configuration, tracing integration and structured context. |
| rake::devtools | Inspector configuration and extension points. |
| rake::gpui | Advanced access to the selected GPUI revision. |

Internal implementation crates can remain separate. The facade should own the compatibility contract for its HTTP, routing, task and diagnostic types; a public HTTP signature should not require a reqwest::Client, web_sys::Response, Tokio handle, or browser-specific future. Optional features can keep inspector UI and native-only facilities out of a small build while the default application profile includes the basic HTTP, JSON and logging experience.

## Macro surface

| API | Intended use |
| - | - |
| #[rake::app] | Generate the platform entry point and standard service bootstrap. |
| #[derive(rake::Route)] | Generate the route codec and descriptor schema. |
| #[derive(rake::Command)] | Generate command arguments and registry metadata. |
| #[rake::data] | Apply standard serialization derives with the correct facade paths. |
| #[rake::instrument(skip_all)] | Add a function span, with explicitly selected fields when needed. |
| rake::info!, warn!, error! | Emit ordinary structured logging events. |

HTTP works as a runtime service without a derive. A generated endpoint client can be added later as an optional api attribute once the ordinary request API is stable. Each macro has a distinct job; starting with these small pieces avoids requiring a dependency-injection language to fetch data or emit a log.

The single-crate promise includes generated code. Rake's data attribute can emit normal Serde derives plus the resolved serde crate path; it need not invent a new serialization format. The instrumentation attribute must emit references through the resolved Rake facade and support a documented argument subset. Re-exporting an external procedural macro alone does not establish this guarantee. Support renamed facade dependencies and test examples with rake as their only framework dependency. [Serde crate attribute](https://serde.rs/container-attrs.html) and [Rust procedural macro hygiene](https://doc.rust-lang.org/reference/procedural-macros.html#procedural-macro-hygiene).

# Public API choices

Three API shapes are reasonable.

| Style | Example | Tradeoff |
| - | - | - |
| Direct commands | navigator.open(destination, OpenTarget::NewTab) | Clear side effect and straightforward CLI mapping. Recommended primary API. |
| Request builder | navigator.request(destination).new_tab().send() | Useful with many options. An explicit send method is essential. Optional later sugar. |
| Tab handles | navigator.tab(tab_id).navigate(destination) | Makes targeting and ownership explicit. Recommended companion API. |

Start with direct commands, an open_with method for options, and tab handles. Each spelling should lower to the same operation types. Avoid implementing separate routing behavior for a builder or CLI adapter.

```rust
use rake::prelude::*;

#[derive(Clone, Route)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,

    #[route(id = "project", path = "/projects/{id}")]
    Project(ProjectId),

    #[route(id = "document", path = "/documents/{id}")]
    Document(DocumentId),

    #[route(id = "settings", path = "/settings/{section}")]
    Settings(SettingsSection),
}

navigator.open(
    Destination::Document(document_id),
    OpenTarget::NewTab,
);

navigator.open_with(
    Destination::Document(document_id),
    OpenOptions::new_tab()
        .activation(Activation::PreserveSelection)
        .mode(TabMode::Live),
);

navigator.tab(tab_id).navigate(Destination::Home);
navigator.tab(tab_id).back();
navigator.tab(tab_id).forward();
navigator.tab(tab_id).select();
navigator.tab(tab_id).close();
```

The mode in open_with belongs to the new tab. It does not change the mode of the previously selected tab. Preserving selection and running while hidden are separate choices.

# Identity model

| Type | Meaning | Lifetime |
| - | - | - |
| RuntimeId | One running application instance, native or browser | Changes on process restart or full browser-page reload; retained when the host runtime survives code replacement. |
| WindowId | One Rake UI host surface | A native application window or the Web canvas host; ends on disposal. |
| TabId | One application tab session | Survives hiding and selection changes; can be restored from a supported checkpoint. |
| EntryId | One visit in a tab history | Survives history traversal while retained; can be restored from a supported checkpoint. |
| GenerationId | One loaded application-code generation | Changes when replacement code is accepted; interpreted with RuntimeId. |
| RunId | One execution of managed activity | Changes when activity restarts. |
| OperationId | One submitted application operation | Supports observation, decisions, and bounded completed-operation retention. |

A destination is a compact value containing IDs and parameters. It does not contain GPUI entities, closures, live sockets, or task handles.

Opening the same document twice creates separate TabId and EntryId values. Those screens have independent scroll positions, selections, and histories. They may intentionally share a document model through an application service. Model sharing is independent of tab reuse.

Stable framework handles can be rebound to replacement GPUI entities after reload. They are not a promise that old typed entities remain valid after arbitrary Rust type changes.

# Navigator surface

```rust
pub enum OpenTarget {
    CurrentTab,
    NewTab,
    NewWindow,
}

pub enum Activation {
    Activate,
    PreserveSelection,
}

pub enum TabMode {
    Foreground,
    Live,
}

pub struct OpenOptions {
    pub target: OpenTarget,
    pub activation: Activation,
    pub mode: Option<TabMode>,
}

impl OpenOptions {
    pub fn new_tab() -> Self;
    pub fn new_window() -> Self;
    pub fn activation(self, activation: Activation) -> Self;
    pub fn mode(self, mode: TabMode) -> Self;
}

impl<R: Route> Navigator<R> {
    pub fn open(
        &self,
        destination: R,
        target: OpenTarget,
    ) -> Operation<Opened>;

    pub fn open_with(
        &self,
        destination: R,
        options: OpenOptions,
    ) -> Operation<Opened>;

    pub fn tab(&self, tab_id: TabId) -> TabHandle<R>;

    pub fn for_window(&self, window_id: WindowId) -> Navigator<R>;

    pub fn reveal(
        &self,
        destination: R,
        scope: RevealScope,
    ) -> Operation<Revealed>;
}

impl<R: Route> TabHandle<R> {
    pub fn navigate(&self, destination: R) -> Operation<Entered>;
    pub fn replace(&self, destination: R) -> Operation<Entered>;
    pub fn back(&self) -> Operation<Entered>;
    pub fn forward(&self) -> Operation<Entered>;
    pub fn select(&self) -> Operation<Selected>;
    pub fn close(&self) -> Operation<Closed>;
    pub fn set_mode(&self, mode: TabMode) -> Operation<ModeChanged>;
}
```

These are signature sketches; omitted definitions include IDs, result records, and builder implementations.

A screen receives a Navigator bound to its originating window and tab. An application service uses explicit targeting when an operation depends on a current window or tab. Contextual targets are resolved to concrete IDs on submission, before asynchronous guards run. If the target disappears, the operation reports that fact instead of redirecting to a different current tab.

Navigator is a command proxy. GPUI entity mutations and view creation happen on the host UI event loop, including the browser event loop on Web. Callers use owned handles and do not hold a borrowed GPUI App or Context across an asynchronous wait. GPUI's context model separates entity ownership from fallible asynchronous access. [GPUI contexts](https://github.com/zed-industries/zed/blob/main/crates/gpui/docs/contexts.md).

## Operation meanings

| Operation | Required behavior |
| - | - |
| Open into NewTab | Always create a fresh tab and root entry. Preserve other tabs and their histories. |
| Open into CurrentTab | Push a new entry in the resolved tab, equivalent to navigate. |
| Open into NewWindow | Create a window with a fresh tab. Report unsupported capability where unavailable. |
| Select | Change selection without adding a per-tab history entry or disposing the old tab. The optional browser adapter records the resulting visible selection. |
| Navigate | Push an entry, preserving the backward history and guarding disposal of the forward branch. |
| Replace | Replace the current entry with a new instance; guard disposal of the old instance. |
| Back or forward | Move this tab's cursor to an existing retained entry. |
| Reveal | Select a matching existing current entry in scope, or create a new tab if none exists. |
| Close | Guard disposal, remove the tab, and request cancellation of its owned work. |

Reveal matches the canonical destination and complete parameters by default. It considers current entries of existing tabs, not hidden history entries. If multiple tabs match, select the most recently selected matching tab in the requested scope. Lookup and creation occur in one serialized router operation so concurrent reveal requests do not create duplicates.

NewTab never silently deduplicates. A caller wanting an exact instance uses TabId. A caller wanting semantic reuse uses reveal.

A mode supplied in OpenOptions is a creation setting. Supplying it for CurrentTab should return InvalidOptions; changing an existing mode uses set_mode explicitly.

NewWindow can follow the initial tab implementation without changing the identity model. The runtime advertises supported targets. The initial Web adapter reports UnsupportedCapability for NewWindow. Opening a URL in a separate browser context is a distinct platform action and does not return a shared in-memory Rake window. Unsupported targets never silently become NewTab.

# Tab modes and activity

Both modes retain the tab, current entry, local state, and history. Hidden content is not added to the displayed content layout or paint tree. Data updates may still affect visible tab labels or badges.

| Event | Foreground | Live |
| - | - | - |
| Tab becomes hidden | Stop managed activity and retain state | Continue managed activity and retain state |
| Tab becomes displayed again | Construct fresh activity runs from registered factories | Existing runs continue |
| Current history entry changes | Stop outgoing entry activity | Stop outgoing entry activity |
| Tab closes | Dispose entries and request cancellation of tab-owned work | Same |
| Window loses keyboard focus | No automatic change | No automatic change |

Foreground is the default. Live applies to the current entry of an inactive tab; it does not keep every historical screen running.

Distinguish selection, presentation in the app layout, host visibility, and keyboard focus. Future split panes may present several tabs. On Web, Foreground activity requires both app presentation and a visible host document; Live remains eligible when either becomes hidden. Native occlusion and minimization use separately documented host signals. Keyboard focus alone does not decide visibility.

Foreground does not freeze arbitrary Rust execution. It controls framework-managed activity with a known cancellation and restart contract. State subscriptions, shared-model updates, and explicitly longer-lived work may continue.

## Work ownership

```rust
// Registered factory: the framework can construct a fresh run.
page.activity("refresh", move |run| {
    let api = api.clone();

    async move {
        let project = api.load_project(project_id).await?;
        run.publish(ProjectLoaded(project))
    }
});

// Always follows presentation, even in a Live tab.
page.while_visible("preview", move |run| {
    preview_frames(run)
});

// Explicit user work remains active while the tab is hidden.
page.tab().tasks().spawn("download", async move {
    download_attachment(attachment_id).await
});

// An application-owned export survives closing its source tab.
page.app().tasks().spawn("export", async move {
    export_project(project_id).await
});
```

The activity example elides registration of the typed ProjectLoaded event handler. Publication schedules the registered handler through the runtime; it does not directly mutate GPUI state from a worker.

| Work kind | Hidden tab | Navigating within tab | Tab close | Restart mechanism |
| - | - | - | - | - |
| Managed page activity | Follows mode | Stops outgoing run | Stops | Registered factory |
| Visibility activity | Stops | Stops outgoing run | Stops | Registered factory |
| Tab-owned task | Continues | Continues | Cancellation requested | None unless separately registered |
| Application-owned task | Continues | Continues | Continues | Task-specific policy |

Activity factories produce fresh futures; spawn accepts one future. Cancelling a future and later invoking a factory is a restart, not a resume. No generic pause method is promised.

Normal task scopes and activity factories accept local futures without a blanket Send requirement. Their asynchronous I/O yields to the host event loop; spawning an async task does not promise a worker thread. Worker or parallel-compute APIs are separate capabilities. This accommodates browser futures and GPUI-local state without permitting CPU-heavy work to block the UI silently. [wasm-bindgen spawn_local](https://docs.rs/wasm-bindgen-futures/latest/wasm_bindgen_futures/fn.spawn_local.html).

The ownership table describes what Rake keeps eligible while the host runs. Application-owned work survives closing an internal Rake tab, but its lifetime still ends with its native process or browser application instance. Live is not a durable background service.

Cancellation does not roll back external effects or immediately stop every blocking operation. Once an owner is disposed, publication into that owner is rejected. Unfinished work and destructors must retain any code generation they still require.

A result is applied only if its owner exists, its resource run remains current, and its code generation is accepted. A useful identity tuple is EntryId, ResourceKey, RunId, and GenerationId. TabId alone cannot distinguish an earlier document or earlier request within the same tab.

Switching to another tab must not invalidate a continuing tab-owned task solely because selection changed.

# History retention and guards

Each tab owns its history and cursor. Entries carry destinations and local UI state. Navigating back and then pushing a destination discards the forward branch only after relevant disposal guards pass.

For the first implementation, preserve open tab state until explicit close. Keep history retention configurable and visible in devtools. Do not silently evict dirty entries to meet a retention limit. A configured capacity can block a push or require an explicit discard or checkpoint decision.

Later, add RestoreOnDemand as a separate retention capability for checkpoint-compatible views. It should not be another execution mode. Releasing a view while arbitrary tasks still capture it is not a valid generic policy.

## Guard categories

- Entry guards control access to a destination.

- Disposal guards protect state that an operation would destroy.

- Application shutdown coordinates remaining disposal guards and jobs.

Opening or selecting another tab normally needs no unsaved-change prompt because the old state remains. Closing, replacing, truncating forward history, evicting, and quitting may destroy state and therefore need guards.

A guard may allow, deny, or request a structured decision. Decisions expose stable choice IDs and can include registered save commands. UI buttons and CLI responses resolve the same DecisionId.

```rust
pub enum GuardResult {
    Allow,
    Deny(GuardReason),
    Ask(DecisionRequest),
}

pub enum DisposeReason {
    CloseTab,
    ReplaceEntry,
    TruncateForwardHistory,
    EvictEntry,
    QuitApplication,
}
```

The router plans affected entries, evaluates guards, performs approved preparation, revalidates target and state revisions, and commits the state change atomically. A save during guard handling is an external side effect; cancelling navigation does not undo the save.

Conflicting requests can be serialized per tab initially. Dirty state must be checked again after asynchronous saves or decisions. A response to an obsolete DecisionId cannot authorize a later decision.

# Submitted operations

Commands submit eagerly and return an operation handle. They do not depend on a future being polled to take effect.

```rust
pub enum Outcome<T> {
    Committed(T),
    Cancelled(CancelReason),
    Blocked(BlockReason),
}

impl<T> Operation<T> {
    pub fn id(&self) -> OperationId;
    pub async fn wait(self) -> Result<Outcome<T>, OperationError>;
}

pub struct Opened {
    pub window_id: WindowId,
    pub tab_id: TabId,
    pub entry_id: EntryId,
}
```

A queued operation ID does not mean success. Runtime status distinguishes queued, running, awaiting decision, committed, cancelled, blocked, and failed. Closing a CLI connection or dropping the Rust observation handle does not implicitly cancel an operation. Explicit cancellation is a separate operation and may be too late after commit.

```rust
let operation = navigator.open(
    Destination::Document(document_id),
    OpenTarget::NewTab,
);

let operation_id = operation.id();

match operation.wait().await? {
    Outcome::Committed(opened) => {
        rake::info!(
            operation = %operation_id,
            tab = %opened.tab_id,
            "Document tab opened"
        );
    }
    Outcome::Cancelled(_) => {}
    Outcome::Blocked(reason) => {
        show_navigation_reason(reason);
    }
}
```

Navigation commit means the router accepted the new state and attached a usable route view or defined loading view. It does not mean all data has loaded or the first frame has been presented. Data readiness and frame presentation have separate events.

Use distinct errors for invalid options, unavailable routes, closed targets, mount failures, restore failures, and runtime shutdown. A deliberately rejected operation is distinguishable from a technical failure.

Completed operations remain queryable for a documented retention period. An expired ID returns an explicit expiration result. Transport retries use a request idempotency key so retrying an open request cannot accidentally create another tab.

# Route schemas and CLI addressing

The Rust enum is a convenient application API. The external protocol needs stable route keys, typed parameters, parsing, formatting, and schema discovery.

Conceptual address:

```json
{
  "route": "document",
  "params": {
    "id": "doc-42"
  }
}
```

Its URI representation might be /documents/doc-42. Both are decoded into Destination::Document(DocumentId). Use a proper encoding contract for path segments and query parameters, and validate required, unknown, and malformed parameters.

```rust
pub trait Route: Clone + 'static {
    fn encode(&self) -> RouteAddress;
    fn decode(address: &RouteAddress) -> Result<Self, RouteDecodeError>;
    fn descriptors() -> &'static [RouteDescriptor];
}
```

A RouteAddress is owned serializable data. RouteDescriptor includes a stable key, parameter schema, URI mapping, and any external aliases. The proposed Route derive generates these from explicit route IDs and paths. For a single-field tuple variant, the sole path placeholder binds that field; use named fields when several parameters need mapping. Domain IDs implement Rake's parameter parsing and formatting contract. Transport messages carry owned addresses, so the root Route trait itself need not impose Send or Sync on all UI code.

Decode and encode should round-trip canonical addresses. Stable external keys survive Rust variant renaming unless an intentional migration changes them. Do not parse a Debug string to reconstruct a route.

The application owns its root enum. External components and services register through plugins; a plugin cannot add variants to an already compiled enum. The application can include a plugin's typed route value in its own enum. An erased dynamic route registry is a later extension with explicit schemas and compatibility rules.

# Web platform contract

Web means a browser application targeting wasm32-unknown-unknown. Rake selects GPUI's Web platform and packages the Wasm module, bindings, canvas host and loader. Upstream already includes a gpui_web implementation and a single_threaded_web entry path with a Fetch client. The inspected backend supports one top-level GPUI window and does not support reopening it after closure. [GPUI platform facade](https://github.com/zed-industries/zed/blob/main/crates/gpui_platform/src/gpui_platform.rs), [Web platform source](https://github.com/zed-industries/zed/blob/main/crates/gpui_web/src/platform.rs), and [browser examples](https://github.com/zed-industries/zed/blob/main/crates/gpui/examples/README.md).

Rake should pin and validate a compatible GPUI revision. GPUI Kit's current WebAssembly guide still describes browser use primarily as component showcases and calls for product-specific checks of accessibility, input, startup and deployment. Having a portable router and HTTP client is one part of shipping a complete browser application. [GPUI Kit WebAssembly guide](https://gpui-kit.com/docs/webassembly/).

## Application tabs and browser contexts

OpenTarget::NewTab always means an internal Rake application tab. It retains previous tabs in the same application instance on both native and Web. A separate browser tab boots another application instance and shares data only through an explicitly designed storage or synchronization service.

An external link action can request another browser context using a destination URL. It reports dispatch or failure according to browser capabilities instead of fabricating a Rake WindowId. Browser window creation is subject to user activation and popup policy. [Window.open](https://developer.mozilla.org/en-US/docs/Web/API/Window/open).

## URL and history adapter

```rust
.web(WebConfig::new().routing(WebRouting::Path))
// Or WebRouting::Hash for hosts without document-route rewrites.
```

Path mode maps the selected destination to the address bar and requires the deployment server to serve the app shell for deep links. Hash mode uses the URL fragment. The Route derive supplies the same canonical address codec used by native deep links and the CLI. The application registry validates unknown destinations and malformed parameters through the same error path.

Per-tab history and browser history have different scopes. Each Rake tab keeps its own entries. The browser adapter records a chronological history of visible workspace destinations and selections. A browser checkpoint contains a versioned destination plus runtime, tab and entry hints; its hints help reuse live state but do not replace the reconstructible destination.

| Action | Default browser-history effect |
| - | - |
| Open and activate an application tab | Push a checkpoint for the new visible destination. |
| Select a different application tab | Push a checkpoint for that selection; do not add a per-tab entry. |
| Navigate in the presented tab | Push a checkpoint after navigation commits. |
| Replace the visible destination | Replace the current checkpoint. |
| Open or navigate a hidden tab without selecting it | Leave the current URL and browser history unchanged. |
| Per-tab Back or Forward | Move that tab cursor; if visible, record the resulting visit in browser history. |
| Browser Back or Forward | Restore the checkpoint's selection or reconstruct its destination; do not push another checkpoint. |
| Close the selected tab | Replace the current checkpoint with the surviving selected destination. |
| Reload or restore a checkpoint | Initialize or replace the current browser state without adding navigation. |

Browser traversal does not undo document edits, close unrelated tabs, or cancel their work. If a checkpoint's runtime no longer exists, or its tab or entry was disposed, use the destination to create or reveal an appropriate view under the normal guard rules. Do not promise resurrection of discarded resources. The browser adapter must avoid feedback loops while handling popstate. Browser History exposes pushState, replaceState, and traversal notifications for this relationship. [History API](https://developer.mozilla.org/en-US/docs/Web/API/History_API/Working_with_the_History_API).

## Host scheduling and shutdown

Foreground follows app presentation and host document visibility. Live means Rake does not stop the current entry's managed activity because it becomes hidden. Browsers may throttle timers and animation callbacks, freeze execution, or discard the document. Devtools should display app-tab policy and observed host visibility separately, without pretending to measure a browser scheduler state it cannot observe. [Page Visibility](https://developer.mozilla.org/en-US/docs/Web/API/Page_Visibility_API) and [Chrome Page Lifecycle](https://developer.chrome.com/docs/web-platform/page-lifecycle-api).

Rake controls asynchronous guards for internal tab operations. Browser-page termination and cross-document navigation have different limits: the app cannot guarantee a custom asynchronous save dialog or completion of shutdown work. Persist important drafts incrementally through an explicit persistence policy. Browser history traversal may already have changed the URL when the router handles it; reconcile pending or denied navigation rather than treating popstate as a cancellable Rake command.

## Capability and error model

| Feature | Shared contract | Target-specific boundary |
| - | - | - |
| Router and application tabs | Same operations, identities and retention rules. | Browser URL/history adapter. |
| HTTP | Same Rake requests, response helpers and diagnostics. | Browser Fetch security and visibility restrictions. |
| Tasks | Same scope ownership and cancellation intent. | Local execution, optional workers, host scheduling and lifetime. |
| Tracing | Same structured events, spans and context. | Native terminal or browser console output. |
| Windows | Explicit requested target and outcome. | Additional native windows require support; Web v1 has one GPUI host. |
| Reload | Versioned logical checkpoints and visible reload outcome. | Native code replacement or Web page reload and restoration. |
| CLI control | Same command, query and event protocol. | Native local transport or browser-initiated development bridge. |

Rake's public error types distinguish unsupported capabilities, denied or unavailable host operations, invalid input, and runtime failures where the host exposes that distinction. Platform adapters must report when a specific cause or measurement is unavailable.

# CLI surface and parity

Separate project supervision from live-app operations.

| Service | Responsibilities |
| - | - |
| Development supervisor | Scaffolding, native/Wasm builds, watching, native launch or Web serving, build output, browser loader generation, reload coordination and packaging. |
| Application runtime | Router, host surfaces, tabs, tasks, resources, commands, decisions, authoritative state and diagnostics. |

The supervisor remains useful when the application is stopped or has crashed. Runtime requests are marshalled onto the application's event loop where needed.

```bash
rake new notes
rake dev --target desktop
rake dev --target web
rake doctor
rake build --target desktop --release
rake build --target web --release
rake bundle

rake instances --json

rake ctl --instance app-42 routes list --json
rake ctl --instance app-42 router open document \
  --param id=doc-42 --target new-tab --mode live --json

rake ctl --instance app-42 tabs list --json
rake ctl --instance app-42 tabs select tab-7
rake ctl --instance app-42 tabs mode tab-7 live
rake ctl --instance app-42 tabs back tab-7
rake ctl --instance app-42 tabs close tab-7 --no-wait --json

rake ctl --instance app-42 inspect tree --json
rake ctl --instance app-42 inspect element element-12 --json
rake ctl --instance app-42 tasks list --owner tab-7 --json
rake ctl --instance app-42 requests list --tab tab-7 --json
rake ctl --instance app-42 logs follow --tab tab-7 --json
rake ctl --instance app-42 traces export --operation op-19
```

Use --preserve-selection to open a new tab without selecting it. The --mode argument configures its work policy. They must not be combined into an ambiguous --background flag.

A shared schema generates or validates CLI argument parsing, Rust-client requests, devtools forms, help, and machine-readable output.

For Web development, the running browser instance opens an authenticated connection to the Rake development server. The CLI sends the same RuntimeClient messages through that broker. Each browser instance has its own RuntimeId and advertised capabilities. A browser instance never gains native filesystem, process, or window privileges merely because a command arrived from the CLI. The development broker is not required by a static production Web build.

## Guard decisions over the CLI

```bash
rake ctl --instance app-42 tabs close tab-7 --no-wait --json
```

```json
{
  "operation_id": "op-19",
  "state": "awaiting_decision",
  "decision_id": "decision-6",
  "choices": ["save", "discard", "cancel"]
}
```

```bash
rake ctl --instance app-42 decisions respond decision-6 --choice save
rake ctl --instance app-42 operations watch op-19 --json
```

An interactive terminal may display the decision. A noninteractive caller receives a structured pending status and can answer later. Timeout or disconnection does not approve or cancel the decision. Revalidation prevents a stale choice from applying to changed state.

Define parity as the same commands, queries, decision rules, and underlying data. A graphical timeline and a trace export are two presentations of the same information. Full runtime parity does not mean arbitrary private Rust methods become remotely callable.

## Application commands

```rust
#[derive(Command)]
#[command(id = "document.save")]
struct SaveDocument {
    document: DocumentId,
}

app.commands().dispatch(SaveDocument { document });
```

```bash
rake ctl --instance app-42 commands run document.save \
  --args '{"document":"doc-42"}' --json
```

The derive and registry are proposed. Registered commands have typed arguments, schemas, results, availability, and an execution capability such as Headless or RequiresWindow. Menus, buttons, shortcuts, a command palette, and CLI calls all dispatch the same command.

A command with RequiresWindow reports that requirement when no suitable UI exists. A headless export uses services independent of mounted views. This makes a future headless application CLI possible without pretending GUI-only actions work in an absent window.

# Custom devtools

The framework owns authoritative application state. Devtools queries snapshots, observes changes, and submits normal operations through a typed RuntimeClient. The CLI uses the same client.

| Panel | Initial contents |
| - | - |
| Router | Windows, tabs, destinations, entry history, mode, presentation status, pending decisions. |
| Tasks | Lifetime owner, causal origin, state, progress, cancellation, related requests and logs. |
| Requests | Instrumented network activity, method, status, timing, bounded details. |
| Console | Structured logs, legacy logs, captured process output, registered commands. |
| Traces | Instrumented span tree and timing linked to operations and tasks. |
| Reload | Generations, build/reload phases, restored state, restart reasons, retained generations. |
| Elements | Tree, layout, style, source locations and semantic identities where supplied. |

The useful relationship is: an operation created an entry, that entry started a task, that task issued a request, and a related log explains its failure.

Keep lifetime ownership separate from causal origin. An application-owned export can outlive the tab that started it. Its owner remains the application while its origin still references the earlier tab and operation.

A diagnostic context can carry RuntimeId, GenerationId, OperationId, owner scope, origin entry, TaskId, RequestId and SpanId where applicable. Missing context stays explicit; do not assign unrelated process output to whichever tab happens to be selected.

## Console and tracing integration

Rake integrates the tracing ecosystem into its application runner and custom collector. Ordinary users import structured logging and instrumentation through rake. The inspector, protocol and event storage remain Rake's own components.

```rust
use rake::prelude::*;

#[rake::data]
struct DocumentSummary {
    id: String,
    title: String,
}

#[rake::instrument(skip_all)]
async fn load_documents(
    http: &HttpClient,
) -> rake::http::Result<Vec<DocumentSummary>> {
    let documents = http
        .get("documents")
        .send_json::<Vec<DocumentSummary>>()
        .await?;

    rake::info!(count = documents.len(), "Documents loaded");
    Ok(documents)
}
```

The standard app runner sets up diagnostics and a compatible tracing subscriber; rake dev connects the custom inspector automatically. Native events can also be written to the terminal, and Web events to the browser console. Libraries and plugins emit through the existing collector. An embedding host can supply a subscriber integration instead of allowing a second global initialization; configuration conflicts are reported explicitly.

The tracing ecosystem supplies structured events and spans; tracing-subscriber supports composable layers. [tracing](https://docs.rs/tracing/latest/tracing/) and [subscriber layers](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/layer/index.html).

```rust
rake::info!(
    document_id = %document_id,
    bytes = bytes.len(),
    "Document loaded"
);

#[rake::instrument(skip_all, fields(document_id = %document_id))]
async fn load_document(
    http: &HttpClient,
    document_id: DocumentId,
) -> rake::http::Result<Document> {
    http.get("documents")
        .query(&[("id", document_id.to_string())])
        .send_json::<Document>()
        .await
}
```

A tracing-log bridge can include legacy log records where the host permits logger installation. The native development supervisor can capture process stdout and stderr. Browser structured logs use the browser console adapter; arbitrary browser console output is separate coverage. Bridges cannot recover typed fields already flattened into text. [tracing-log](https://docs.rs/tracing-log/latest/tracing_log/).

Console initially means logs, explicitly inspectable values, and registered commands. Arbitrary evaluation of Rust expressions requires a separate compiler/runtime design.

Rake task wrappers capture diagnostic context at submission and instrument the future while it is polled. The instrumentation attribute records function work through the same mechanism. It supports a documented subset of options such as skip_all and explicit fields; it does not promise to reproduce every external macro transformation. Span::enter guards must not remain active across an await. [Instrument](https://docs.rs/tracing/latest/tracing/trait.Instrument.html).

Record automatic spans around navigation, guards, mounting, activity, requests, and reload phases. Application authors add spans around domain operations. A span's elapsed duration is not CPU usage, and this instrumentation timeline is not a sampling profiler.

## Requests

HttpClient is a first-class Rake service. page.http(), run.http(), and application service contexts return inexpensive handles to a shared configured transport. Page handles carry immutable causal-origin IDs without retaining the page entity. Each request captures its current task, cancellation scope and span when execution starts. A tab change cannot reassign a request to the newly selected tab.

```rust
let http: HttpClient = page.http();

let documents = http
    .get("documents")
    .send_json::<Vec<DocumentSummary>>()
    .await?;

// The same transport supports a request body.
let saved = http
    .post("documents")
    .json(&draft)
    .send_json::<Document>()
    .await?;

// Access status, headers or streaming explicitly when needed.
let response = http.get("export").send().await?;
let response = response.error_for_status()?;
```

send_json sends the request, requires a successful 2xx status, and decodes the body. It reports transport, cancellation, HTTP status, body-reading and decoding failures through Rake error types. send returns the raw Rake response, including non-2xx status responses, so callers can handle domain-specific error bodies or streaming. Serialization uses the standard Serde traits exposed through the facade.

HttpConfig defines the base URL and shared request policy once. With base [https://api.example.com/](https://api.example.com/), the relative endpoint documents resolves to [https://api.example.com/documents](https://api.example.com/documents). URL joining follows documented URL rules, including the difference between root-relative and path-relative references. Relative URLs require an explicitly configured base on both targets; without one they return a clear configuration error. Absolute URLs work without a base.

The native transport can reuse an established Rust HTTP client; the browser transport uses Fetch, directly or through an existing Wasm-compatible adapter. Rake's own types keep runtime-specific setup out of application code. Reqwest already documents a target-specific Wasm client and limitations on native TLS, cookie and builder configuration. [reqwest Wasm support](https://docs.rs/reqwest/latest/reqwest/#wasm).

Portable HTTP covers ordinary methods, supported headers, query encoding, request bodies, response reading and explicit cancellation. Browser requests obey CORS, forbidden-header rules and the browser's credential policy; response visibility may be restricted. Native proxy, custom TLS and cookie-store controls are separate extensions. A browser network rejection does not always reveal enough information to classify a specific CORS failure. Cancellation or timeout means the client stops waiting; it does not undo server effects. [Fetch behavior](https://developer.mozilla.org/en-US/docs/Web/API/Fetch_API/Using_Fetch).

Any portable timeout requires a Rake cancellation adapter on both targets; it cannot simply forward a native-only builder method. Its observed timing is subject to host scheduling. Framework request capture is automatic for Rake clients and for GPUI network hooks connected to the observed adapter; third-party clients can contribute adapters. The inspector reports that coverage. Server trace propagation is a separate configured HTTP policy; local devtools correlation does not require injecting tracing headers into every origin.

Request events cover start, headers received, body progress where observable, body completion, cancellation, and failure. Track RequestId, owner, initiator, span and generation. Response headers arriving is distinct from the body finishing. If the caller abandons a response body, report it as abandoned rather than completed. Browser-hidden headers and unavailable transport timings remain explicitly unavailable.

Default capture is bounded metadata with credentials redacted. Body previews are opt-in, size-limited, and must preserve streaming behavior. Do not consume or buffer an entire response solely to display it.

```rust
page.activity("documents", |run| async move {
    let documents = load_documents(&run.http()).await?;
    run.publish(DocumentsLoaded(documents))
});
```

The activity factory gets a client associated with the current run. The automatic trace can link navigation, entry, activity run, request and log. A new run cannot accept the earlier run's publication, and an application-owned task keeps its original causal origin if it outlives the tab.

## Snapshot and event consistency

```rust
let watch = client.watch(WatchRequest::application()).await?;
let snapshot = watch.snapshot();
let events = watch.events();
```

The watch begins with a snapshot at a specific runtime revision and changes after that revision. Establish the subscription atomically or retain sufficient replay so no change is lost between snapshot and watch.

Cursors include runtime identity and epoch. Bounded replay can report a gap and require a new snapshot. A native process restart or a full browser-page reload creates a new RuntimeId, even when workspace IDs are restored.

Authoritative router and operation state must not be reconstructed from filtered or sampled tracing events. Keep the runtime state store separate from best-effort diagnostics buffers. Telemetry overflow reports dropped counts and never blocks the UI thread or corrupts router state.

# Hot reload adapter

Rake supplies lifecycle and checkpoint integration for two reload adapters. On desktop, the user's general Rust reloader handles supported code replacement. On Web, the initial adapter rebuilds Wasm and restores logical state after a browser-page reload. Both report progress and outcomes through the same development protocol.

```rust
pub trait ReloadState: Sized {
    const SCHEMA: &'static str;
    const VERSION: u32;

    fn checkpoint(&self) -> Result<Checkpoint>;

    fn restore(
        checkpoint: Checkpoint,
        services: &Services,
    ) -> Result<Self>;
}
```

Checkpoint is versioned owned data or documented host resource IDs. It is not arbitrary serialization of entities, callbacks, futures, GPU resources, or vtables.

Keep development ReloadState separate from durable SessionState. A reload snapshot may preserve an unsaved draft and expanded devtools nodes. Durable session storage needs its own persistence, privacy, migration, and recovery policy.

A reload preserves RuntimeId only when the runtime host continues. Native in-process replacement can retain RuntimeId, TabId and EntryId. A full Web page reload creates a fresh RuntimeId; checkpoint-compatible tabs and entries may retain their logical IDs, scoped by the new runtime. GenerationId and RunId change as appropriate. Diagnostics never mistake a restored ID for the earlier live object.

## Web reload strategy

Start with compile replacement Wasm, capture the latest versioned checkpoint, reload the page, and restore supported state. Store development checkpoints through an explicit temporary persistence adapter, and distinguish them from the user's durable session policy. Recreate tasks, HTTP requests and browser/GPU resources from declared factories. Futures, callbacks and Wasm pointers are not checkpoint data.

Reload must not add a browser-history entry. The inspector reports whether it used native code replacement, state-preserving Web reload, or a full restart after an incompatible checkpoint. A later controlled Wasm-module replacement can optimize this workflow after its teardown and resource contract is established. Upstream browser examples already use rebuild and page reload to release prior app instances and workers. [GPUI browser examples](https://github.com/zed-industries/zed/blob/main/crates/gpui/examples/README.md).

## Transition sequence

1. Compile the replacement while the existing app remains usable.

2. Establish a transition boundary and stop accepting old-generation publications.

3. Quiesce reload-owned callbacks and work; queue host events where appropriate.

4. Capture the latest logical state.

5. Validate the replacement and restore or recreate affected views.

6. Rebind stable handles and restart eligible managed activity.

7. Retire old code only when all objects, callbacks, futures and destructors that require it are gone.

A failed compilation keeps the previous app running. A restore failure has an explicit recover-or-restart outcome. External saves and network effects are not transactional with reload.

Generation checks prevent stale logical updates; they do not independently make native unloading safe. An application-owned task survives tab closure, but survival across code reload depends on whether its code and resources remain valid under the loader's contract.

Keep the diagnostic collector and control connection in the stable host where practical. Dynamically loaded code needs an explicit path to that collector; do not assume independently loaded logging state automatically attaches to a host subscriber.

# Application composition and extensions

An application can start with the small factory above and add its domain services and commands as needed:

```rust
use rake::prelude::*;

#[rake::app]
fn app() -> App<Destination> {
    App::new("com.example.workspace")
        .routes(AppRoutes)
        .tab_defaults(TabDefaults::new().mode(TabMode::Foreground))
        .http(
            HttpConfig::new()
                .base_url("https://api.example.com/")
        )
        .web(WebConfig::new().routing(WebRouting::Path))
        .service(DocumentStore::new())
        .command::<SaveDocument>(save_document)
}
```

AppRoutes maps Destination to ordinary GPUI views. Its factory receives route-scoped Rake handles plus the underlying GPUI context needed to create entities. Advanced GPUI access is imported through rake::gpui. The app runner supplies normal HTTP and diagnostic services without registration boilerplate; the development configuration selects the native or Web reload adapter.

```rust
pub trait RouteViews<R: Route> {
    fn mount(
        &self,
        destination: &R,
        page: PageContext<R>,
        window: &mut rake::gpui::Window,
        cx: &mut rake::gpui::App,
    ) -> Result<rake::gpui::AnyView, MountError>;
}
```

Mount runs when an entry is first created or deliberately reconstructed, not on every render. Retained entries reuse their existing views. A custom shell can render a TabBar and RouteOutlet from the same workspace model; displaying a different entry does not authorize disposal of its neighbors. Nested outlets can be added later without changing tab identity or ownership.

Services, commands, task scopes, router operations, diagnostics, and lifecycle hooks are the shared foundation. Components and design systems remain replaceable. Video and audio can be third-party plugins.

A minimal plugin registration shape:

```rust
pub trait Plugin<R: Route> {
    fn register(&self, app: &mut App<R>);
}
```

Plugins can contribute services, commands, diagnostic adapters, and panels. They use the existing lifecycle and operation protocol rather than creating private routing and logging systems. A statically registered Plugin is not automatically a hot-loadable native plugin; dynamic compatibility is a separate contract.

Development control is enabled by the runner, using permission-restricted native IPC or an authenticated browser-to-server connection. It advertises protocol version and capabilities, keeps bounded queues, and reports gaps. Release builds keep this endpoint disabled by default. Multiple native and browser instances require an explicit selector when discovery is ambiguous.

# Initial delivery scope

| Stage | Scope | Evidence of completion |
| - | - | - |
| Portable foundation | Rake facade, application entry, tested GPUI native/Web revision, HTTP/JSON and basic diagnostics | One example application builds for both targets with rake as its framework dependency. |
| Routing core | Typed destinations, independent tabs, per-tab histories, operation handles, guards and Web URL adapter | Opening and switching tabs preserves drafts and defined work; browser traversal follows its documented scope. |
| Lifecycle and CLI | Foreground/Live, activity factories, tab/application jobs, schemas, shared client and Web bridge | Equivalent supported operations and decisions work from GUI, CLI and inspector. |
| Development loop | Native reloader integration, Web state-preserving reload, checkpoints, Console/Router/Tasks/Reload panels | Supported state survives the reported reload mechanism; runtime identity and restart paths remain explicit. |
| Additional diagnostics | Trace views, detailed request inspection and elements panel | Signals correlate with owners and operations, remain bounded, and expose coverage and loss. |
| Extensions | Optional generated API clients, window capabilities, retention, media and external panels | Extensions preserve the existing navigation, transport and lifetime contracts. |

Do not delay instrumentation until the trace panel is visually complete. Collect useful stable events while implementing the router.

# Behavioral scenarios

1. Open A, edit it, open B in a new tab, return to A. Draft, cursor, and history remain.

2. Open A twice. The view instances are independent; shared document data follows the application's model policy.

3. Open a Live tab A and then select B. A's current-entry activity continues.

4. Open a Foreground tab A and then select B. A's activity stops and starts a fresh run when A is displayed again.

5. Run a tab-owned download from A and select B. The download continues under either mode.

6. Close A while an application-owned export is running. The export continues with its original causal origin.

7. Start a guarded close, change focus to another window, and answer. The operation still targets the original tab.

8. Respond to an obsolete decision. The response is rejected and does not authorize a different transition.

9. Back, then navigate. The forward branch is disposed only after its guards allow it.

10. Issue reveal twice concurrently. One matching tab results.

11. Reload a hidden tab. Supported checkpoints preserve logical tab state and eligible activity is recreated. Web page reload receives a new RuntimeId.

12. An old request finishes after activity restarted. The stale result cannot overwrite the newer state.

13. A telemetry consumer falls behind. The UI remains responsive and diagnostic loss is visible.

14. A CLI disconnects after submission. The operation remains queryable and is not silently cancelled.

15. Navigation commits but data is still loading. Operation status and data readiness remain distinguishable.

16. Build the same ordinary application against native and browser adapters with only rake as its framework dependency. Data and instrumentation macros resolve their generated paths correctly, including a renamed rake dependency.

17. Open a new application tab on Web without selecting it. Existing tabs remain, and the URL does not change.

18. Navigate and switch between application tabs, then use browser Back. Restore the recorded destination and selection without closing other tabs or creating a history feedback loop.

19. Hide the browser document. Foreground activity follows host visibility; Live remains eligible subject to browser scheduling. Resuming the host cannot accept stale activity results.

20. Send an HTTP request that returns a non-2xx response, an invalid JSON body, or a browser-visible network rejection. Each helper returns its documented outcome and the inspector does not invent unavailable detail.

21. Trigger a Web reload. Restore compatible tab checkpoints under a fresh RuntimeId, reconnect the CLI, and recreate managed work without duplicating browser navigation.

These scenarios are the acceptance contract for the proposed API. They clarify behavior before implementation.

