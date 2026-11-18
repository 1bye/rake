# Internationalisation and locale

Design for [#21](https://github.com/1bye/rake/issues/21). Source baseline:
`679789f9` (2026-11-18). Proposal only: no runtime code or prototype is included.
The issue's full done-when still requires a prototype and implementation issues;
this document deliberately does not complete that work.

## Decisions

Rake owns one application locale, detected at startup and replaceable at runtime.
Use Fluent for messages and ICU4X for dates, numbers and plural categories, with
only the application's shipped locale data. Keep Kit optional and copy the
resolved locale into its translation global. Translation in `view!` is explicit.
Refresh every window on a change. Keep translated accessible names and add
separate message-key metadata for inspection. Add logical start/end utilities;
full RTL layout and editing remain follow-up work.

Evidence qualifies two starting assumptions: Kit's locale is a process global,
not a GPUI observable global; and taffy supports direction even though GPUI does
not expose it. Also, `inbox.count` is not a Fluent message identifier: use
`inbox-count` (a dot selects an attribute in Fluent; see the [grammar:57–74](https://github.com/projectfluent/fluent/blob/master/spec/fluent.ebnf#L57)), rather than invent a key
rewriting scheme. The examples below use valid Fluent identifiers.

## 1. Kit and the source of truth

The pinned facade is `gpui-kit =0.9.0` ([manifest](../../Cargo.toml#L21)).
Kit is currently native-only in Rake ([rake-ui manifest:32–33](../../crates/rake-ui/Cargo.toml#L32)); this design does not expand Kit to Web.
Its [lib.rs:170,205–209](https://docs.rs/crate/gpui-kit/0.9.0/source/src/lib.rs#170)
re-exports `gpui-component` and initializes it. Component
[lib.rs:125,154–160](https://docs.rs/crate/gpui-component/0.9.0/source/src/lib.rs#125)
loads `locales` with English fallback and exposes `locale()` / `set_locale(&str)`.
Its [Cargo.toml:342–343](https://docs.rs/crate/gpui-component/0.9.0/source/Cargo.toml#342)
uses rust-i18n 4.2; the locked runtime's
[lib.rs:23–31](https://docs.rs/crate/rust-i18n/4.2.4/source/src/lib.rs#23)
holds the process locale with thread caches. Setting it does not notify GPUI.

Coverage is component-owned copy, not arbitrary app children: attachment remove
and retry accessible labels ([attachment.rs:607,629](https://docs.rs/crate/gpui-component/0.9.0/source/src/attachment.rs#607)),
combobox placeholder ([combobox.rs:571](https://docs.rs/crate/gpui-component/0.9.0/source/src/combobox.rs#571)),
and calendar month names ([time/calendar.rs:15–26](https://docs.rs/crate/gpui-component/0.9.0/source/src/time/calendar.rs#15)).
Calendar display also contains hand-written numeric date patterns
([time/utils.rs:125–129](https://docs.rs/crate/gpui-component/0.9.0/source/src/time/utils.rs#125));
copying locale does not make all Kit formatting ICU-based.

Use a small feature-gated adapter: after Kit initialization and before the first
render, call `gpui_kit::component::set_locale`; repeat before refreshing windows
on every Rake locale change. Apps must use Rake's setter thereafter. Map an
unsupported Kit locale to its supported parent, then English, without changing
Rake's locale. Prototype must check cached strings in existing component entities.
Reject Kit as authority: it is optional, its setter has no window notification,
and its catalogue does not contain app messages. Reject replacing Kit's message
engine: the public setter already provides the integration point. Multiple
independent GPUI apps in one process would share Kit's locale; not supported by
this first adapter.

## 2. Message format

Choose Fluent `.ftl`: translators own sentence order, select variants and shared
terms, while Rust supplies typed arguments. See the official
[syntax guide](https://projectfluent.org/fluent/guide/) and
[terms](https://projectfluent.org/fluent/guide/terms.html).
Reject plain interpolated keys because plural and grammatical choices would
move into Rust. Reject ICU MessageFormat for this round because Fluent's terms
and message attributes fit component copy without adding a second app format.
This is a design choice, not a measured claim about translator productivity.

Register catalogues once; validate syntax, duplicate identifiers, fallback
coverage and argument contracts before release. Invalid registration returns an
error. Runtime lookup tries the selected catalogue, its registered language
parent, then the app's required default catalogue. A missing message or invalid
arguments produce a diagnostic and a visible key as last resort, never a panic
or an empty control name. No network catalogue loading in the first round.

## 3. Detection, ownership and updates

Model the preference as `System` or an explicit validated locale, separately
from the resolved supported locale. ThemeMode already separates preference from
appearance ([mode.rs:7–37,49–61](../../crates/rake-ui/src/theme/mode.rs#L7)).
Theme installation sets a global and calls `refresh_windows`
([theme.rs:46–63](../../crates/rake-ui/src/theme.rs#L46)); reuse that update pattern.
The appearance observer ([mode.rs:87–114](../../crates/rake-ui/src/theme/mode.rs#L87))
is appearance-specific, not an existing locale-change event.

At startup prefer an app override, otherwise desktop `sys_locale::get_locales()`
or Web `navigator.languages`, then the app default. Normalize and validate BCP 47
input, negotiate in preference order against registered locales, trying exact
then registered language parent. Never load arbitrary paths from a locale tag.
Keep app persistence outside the core. Explicit unsupported overrides return an
error without changing the current selection; unsupported system preferences
fall back. A setter updates the resolved locale, Kit adapter and global, then
refreshes all windows, including custom windows. Render-time translations must
read the new value; long-lived cached strings must be recomputed by observers.
Web `languagechange` can re-detect in System mode; native automatic OS-change
watching is deferred, with an explicit re-detect operation available.

`sys-locale` is already locked through cosmic-text, not Rake locale ownership
([Cargo.lock:1469–1490](../../Cargo.lock#L1469)); cosmic-text enters through the
Linux and WGPU backend packages ([lock:3823](../../Cargo.lock#L3823)).
It supports wasm with its `js` feature; without that provider it returns an empty
iterator ([sys-locale lib.rs:20–46](https://docs.rs/crate/sys-locale/0.3.2/source/src/lib.rs#20)).
Use the browser API directly to keep the platform policy explicit. A lockfile
entry does not establish that a dependency is enabled in every target build.

## 4. Explicit translation in view!

The parser accepts a Rust expression block as a child
([view.rs:598–605,766–778](../../crates/rake-view-syntax/src/view.rs#L598));
expression children are lowered at [view.rs:1387](../../crates/rake-view-syntax/src/view.rs#L1387).
Thus `{t!(cx, "inbox-count", count = n)}` needs no parser grammar change.
The explicit `cx` makes the owning application and render dependency clear;
a context-free `t!` would need hidden ambient state. Plain literals remain plain.
An opt-in untranslated-copy lint is later work.

A string result alone loses its key. Return a `LocalizedText` element retaining
rendered text and message identifier at its inspection boundary; provide
`as_str()` for APIs that require text. Passing that string to an aria-label loses
metadata, so offer an explicit `message-key` attribute for the control boundary.
That attribute needs syntax validation/lowering and inspection changes, even
though the translation expression itself needs none. Do not infer keys by
matching translated output or relying on a thread-local last translation.

## Proposed public API (illustrative, not compiled)

```rust
locale::install(cx, Catalogues::embedded(/* app locales, required default */)?,
                LocaleMode::System)?;
locale::set_mode(cx, LocaleMode::Explicit("pt-PT".parse()?))?;
locale::redetect(cx)?;
let locale = locale::current(cx); // resolved locale
let label = t!(cx, "inbox-count", count = n); // LocalizedText
view! { <button aria-label={label.as_str()} message-key="inbox-count">
    {label}
</button> }
let amount = locale::format_number(cx, value, NumberOptions::default())?;
let date = locale::format_date(cx, date, DateOptions::default())?;
```

Expose through `rake::locale` and a qualified `rake::t!` to avoid collisions with
Kit's macro. Keep locale/catalogue implementation independent of Kit; add the
adapter under the existing `kit` feature. Exact types and ownership of formatted
text are prototype decisions. Date-only input must not silently acquire a time
zone; timestamp formatting requires an explicit zone. The first API can limit
itself to date-only formatting until the timestamp contract is agreed.

## 5. Formatting and Web size

Use ICU4X date, decimal and plural services on every platform. Fluent selection
must use the same locale and plural data as the formatting layer: the prototype
must establish how to connect Fluent's resolver to ICU4X, rather than silently
shipping two plural engines with different data versions. Keep numeric arguments
numeric until selection; never select on a preformatted string.

Existing ICU dependencies are not date/number formatting: GPUI depends on
`icu_segmenter` ([Cargo.toml:363](https://docs.rs/crate/gpui-pre/0.3.11/source/Cargo.toml#363));
the lockfile also has normalizer/properties via idna_adapter
([lock:4407–4414](../../Cargo.lock#L4407)), properties via urlpattern, and
segmenter's locale fallback/provider/data dependencies
([lock:4252–4380](../../Cargo.lock#L4252)). No ICU date, decimal or plural service
is currently locked. Segmenter is portable `no_std`
([lib.rs:6](https://docs.rs/crate/icu_segmenter/2.4.0/source/src/lib.rs#6));
ICU4X also documents [WebAssembly use](https://icu4x.unicode.org/2_0/tsdoc/).
This establishes platform intent, not a verified Rake wasm build.

Generate a provider restricted to shipped languages and required services;
disable broad compiled-data defaults for new formatting services. Include any
parent/fallback data those services require. ICU4X documents the
[data-generation approach](https://icu4x.unicode.org/1_5/tutorials/data-management/);
that older tutorial is architectural evidence, not a command recipe for the
future selected version. Existing segmentation data is a separate baseline and
must not be incorrectly counted as removable formatting data.

All incremental sizes and timings are **unmeasured**. After disk space permits,
compare the same release Web example, toolchain, optimization and compression:
baseline; Fluent only; Fluent plus filtered ICU4X; then an Intl formatting variant.
Record raw wasm, compressed wasm and JS, locale/provider bytes, cold load,
first-format latency, and locale-switch render latency. Vary shipped locale sets
and report exact inputs and commands. Count duplicate plural/locale machinery.
No size budget has been agreed. If filtered ICU4X exceeds the user-approved
budget, propose browser Intl for Web formatting/plurals with explicit output,
rounding, calendar and locale-data-version differences. Reject choosing that
fallback before measuring; reject shipping all locales by default.

## 6. Direction and bidi

Confirmed: GPUI 0.3.11 has no application layout-direction field in its style
surface. Its taffy conversion sets physical margin/padding and flex direction,
then defaults remaining fields ([taffy.rs:494–535](https://docs.rs/crate/gpui-pre/0.3.11/source/src/taffy.rs#494)).
Correction: locked taffy 0.15.0 **does** expose `Direction::Ltr/Rtl` and a style
`direction` field defaulting to Ltr
([style/mod.rs:670–682,727,883](https://docs.rs/crate/taffy/0.15.0/source/src/style/mod.rs#670)).
GPUI does not pass it through. Reversing a flex row is not inherited RTL layout.
Taffy lays out boxes; it is not the text shaping/bidi implementation.

Do not claim GPUI has no bidi text support: its line-layout tests explicitly
cover RTL and mixed Hebrew/Latin wrapping, using synthetic glyph order
([line_layout.rs:925–945](https://docs.rs/crate/gpui-pre/0.3.11/source/src/text_system/line_layout.rs#925)).
WGPU depends on cosmic-text ([Cargo.toml:66](https://docs.rs/crate/gpui-pre-wgpu/0.3.11/source/Cargo.toml#66)),
whose locked dependencies include unicode-bidi. Conversely Windows DirectWrite
code contains fixed `bidiLevel: 0`
([direct_write.rs:761,1001](https://docs.rs/crate/gpui-pre-windows/0.3.11/source/src/direct_write.rs#761)).
These are partial source findings, not proof of correct shaping, caret movement,
selection or screen-reader order on any platform. Native and browser runtime
bidi remain unverified.

First round adds margin/padding/inset and alignment start/end classes resolved
from locale direction, while physical left/right classes retain their meaning.
Do not automatically reverse child order or directional icons. Full RTL requires
GPUI direction plumbing, inherited direction, text alignment and platform editing
checks; defer that to its own issue. Reject a global row-reverse workaround.

## 7. Inspection and agents

Inspection reads GPUI a11y metadata, preferring explicit labels and deriving
label names from text ([accessibility.rs:15–45](../../crates/rake/src/inspection/view/accessibility.rs#L15)).
CDP emits `name` with label fallback
([accessibility.rs:70–71](../../crates/rake-cli/src/cdp/accessibility.rs#L70)).
Semantic queries deserialize a strict selector with `name`, then compare exact
accessible names ([semantic.rs:162–180,344–379](../../crates/rake-cli/src/cdp/semantic.rs#L162)).
`ctl` exchanges shared protocol requests, rather than running that CDP matcher
([terminal.rs:603–614](../../crates/rake-cli/src/terminal.rs#L603)).
Therefore adding a CDP selector alone is insufficient.

Add optional `messageKey` to captured label/control boundaries in rake-protocol,
Rake capture, CDP projection and strict selectors/capabilities. Add a `ctl` query
path over inspection with the same exact-key semantics; do not overload runtime
node IDs or accessible names. Keys identify messages, not unique nodes: return
all matches and allow scope/role filters, preserving ambiguity errors for actions.
Expose the resolved locale on snapshots. Refresh revisions after locale changes;
existing stale-action protections still apply. Attribute-based accessible labels
must carry their key on the control, not just on a nested text element. Arbitrary
opaque Kit descendants cannot be promised keys without Kit support. Keep this
coverage limit visible. Store keys, not interpolation arguments or private input
values. Reject English-only accessible names: assistive technology needs the
same language as the visible interface.

## Prototype after disk recovery

Extend `examples/view`, keeping its normal build as baseline; add an optional Kit
control on native using the pattern in `examples/kit`. Use English and Portuguese
catalogues, a count select, decimal and date, a language picker and two windows.
Prove startup negotiation, override, fallback, invalid input, repeat switching,
newly opened windows, cached Kit placeholder updates, translated accessible
names and stable key queries through both CDP and ctl. Add parser expansion
coverage for `t!` and metadata, and focused locale/fallback/argument tests.
Use RTL sample text only to expose gaps, not to claim RTL readiness. Later RTL
work tests real mixed-script shaping and editing across platforms.

Run native and wasm checks only when authorized disk capacity is available;
record the Web size matrix above and identical-output formatting cases. Record
failures and any Fluent/ICU integration compromise before committing to the API.
No prototype, compilation, runtime verification or size measurement was done for
this document.

## Open questions for review

- Which locales and compressed Web size budget should the prototype use?
- Is explicit `cx` in `t!` acceptable, and are Fluent hyphenated message IDs acceptable?
- Should unsupported explicit locales error as proposed, or negotiate like System?
- Is date-only formatting sufficient initially; which timestamp/time-zone contract follows?
- Is Kit's process-global limitation acceptable, and should Kit translation gaps block release?

## Proposed follow-up issues (not filed)

| Title | Scope | Done when |
| --- | --- | --- |
| Locale core and window updates | Rake global, detection, negotiation, overrides, custom windows | Invalid-input/fallback tests pass; all existing and new windows follow changes on desktop and Web |
| Fluent messages and explicit t! | Catalogue validation, argument errors, LocalizedText, view expression coverage | Example translates explicitly; literals stay literal; missing-message behavior tested |
| ICU formatting and filtered Web data | Dates, numbers, plural integration, provider generation | Native/Web output cases and reproducible size matrix reviewed against agreed budget |
| Logical start/end classes | Spacing, inset and alignment; preserve physical utilities | Both directions resolve correctly with macro/lint coverage; no mirrored-layout claim |
| Inspection message keys | Protocol, capture, label metadata, CDP selectors, ctl query | Same key finds translated controls in both paths; ambiguity, revisions and privacy tested |
| Kit locale adapter | Optional one-way setter bridge, supported-locale mapping, cache audit | Existing Kit controls update after repeated switches; no-Kit build remains independent |
| RTL layout and editing | GPUI direction plumbing and platform bidi audit | Mirrored boxes, mixed text, caret/selection, focus and a11y order verified across native/Web |

The prototype is the next separate task; create these issues only after user
review. #21 stays open until its broader done-when is met.
