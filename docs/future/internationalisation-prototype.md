# Internationalisation prototype results

This slice exercises [the design for #21](internationalisation.md) with English
and European Portuguese. The normal view example opens the original counter.
Enable `i18n-icu` to open the inbox with Fluent, cardinal plurals, decimal
formatting and Gregorian civil dates. The counter remains reachable from the
inbox. Enable `kit` on native to see both Kit-owned copy and a placeholder supplied by
the app in the selected language.

## Implementation and evidence

- `locale::install` registers parsed resources and selects a locale from the host
  or an explicit preference. The setter updates the GPUI global and Kit, then
  queues a refresh for every window. The example defaults to System and accepts
  `RAKE_VIEW_LOCALE` on native. On macOS, System resolved to `en`, and explicit
  `pt-BR` resolved to `pt-PT` with a warning. Tests cover preference order,
  normalization, same-language and parent matching, default fallback, malformed
  tags and preserving the selection when an explicit locale has no match.
- `rake::t!(cx, "inbox-count", count = n)` renders a label with a message key.
  Missing messages and failed argument resolution log a diagnostic and display
  the key. The app default supplies messages missing from the selected catalogue
  and its registered parent. The key remains available on direct expression
  children and on controls with a literal `message-key` attribute. CDP and
  `rake ctl` found the same button by `inbox-add` across languages in both
  native windows. The name changed from “Add a message” to “Adicionar uma
  mensagem”, and the capture revision advanced. Role and window filters were
  used for these queries.
- Native input was driven through `rake ctl` using the raw `preview_input`
  request with coordinates and generation from the captured button and
  `preview` window. The count took the singular and plural branches at 1 and 2.
  A second inbox opened after the Portuguese switch. It started in Portuguese
  with a fresh count, while the first window retained its count. Switching
  repeatedly through English, Portuguese and System updated both windows.
- Kit `Select` keeps its entity across switches. Native window captures showed
  its own “Please select” in both locales. A second retained Select with
  app-supplied copy displayed “Choose a folder”, “Escolha uma pasta”, then
  “Choose a folder”. Kit 0.9 has neither a `pt-PT` nor a `pt` catalogue. That
  gap covers Select, Calendar, list search, attachment actions and the other
  Kit-owned copy in the pinned catalogue. Its Portuguese translations are tagged
  `pt-BR`. After review, the adapter maps `pt-PT` to Kit's `pt-BR` rather than
  English; that change was not re-captured at runtime. Separately, the source asks for `Combobox.placeholder` while the
  catalogue entry is spelled `ComboBox.placeholder`. This source mismatch was
  observed without exercising Combobox at runtime.

Native inspection and Web rendering in headless Chrome 157 on the same Mac
produced these values from ICU4X 2.4 and CLDR 49.

| Input | English | Portuguese (Portugal) |
| --- | --- | --- |
| Decimal `1234.50` | `1,234.50` | `1234,50` |
| Gregorian date `2026-11-21` | `Nov 21, 2026` | `21/11/2026` |

Chrome started the production Web artefacts with cache disabled. The English
preference was `en-GB`. A preference list of `fr-FR, pt-PT, en-GB` started in
Portuguese. Browser clicks switched English to Portuguese and back. Captured
frames showed the same formatted values after each switch. Single observations
from navigation to resolution of the generated `init()` promise on loopback
were 138 ms for the baseline, 66 ms for Fluent only and 121 ms for Fluent plus
ICU in English. These times include WebAssembly initialization and are order-dependent.
First-format and switch-to-paint latency were not measured. The Web build has one
canvas window. Multi-window behavior was verified on native.

### Reproduce the native run

Use a dedicated target directory and check `df -h /` before each build.

```sh
out=/tmp/rake-i18n-21
export CARGO_TARGET_DIR="$out/target" CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0
cargo build -p rake-app-cli -p rake-view --features i18n-icu,kit,devtools
export PATH="$CARGO_TARGET_DIR/debug:$PATH"
mkdir -m 700 -p "$out/native/control"
RAKE_DEV_CONTROL=1 RAKE_DEV_PREVIEW=1 RAKE_CONTROL_DIR="$out/native/control" rake-view
```

From another terminal using the same `rake` executable, switch using the buttons
and use these queries. Get the second window ID from `windows list`.

```sh
rake ctl --session /tmp/rake-i18n-21/native windows list
rake ctl --session /tmp/rake-i18n-21/native inspect message-key inbox-add --role Button --window window-1
rake cdp --session /tmp/rake-i18n-21/native --no-open --json
```

On the reported runtime WebSocket, call `Rake.selectWindow` with the numeric
`windowId`, then `Rake.findElements` with
`{"query":{"messageKey":"inbox-add","role":"Button"}}`.

## Web release size

Measured on arm64 macOS 26.6.2 with Rust 1.98.0 and wasm-bindgen 0.2.129.
The baseline is the ordinary counter build from the prototype branch with no
features. Both locale builds add the same routed inbox and two catalogues. The
Fluent-only build therefore measures the routing, resolver and example together.
The full variant adds the formatter API and the filtered provider. Existing
GPUI segmentation and font resources are in the baseline. The figures cover
the generated Wasm and all five JavaScript files, including bindgen snippets.
The sums use one compression stream per file, as with static HTTP assets.

| Release variant | Wasm raw | Wasm gzip | Wasm Brotli | JS raw | JS gzip | JS Brotli | Total gzip increase | Total Brotli increase |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Baseline | 21,646,118 | 6,096,310 | 4,353,417 | 193,359 | 31,269 | 28,965 | 0 | 0 |
| Fluent only | 22,078,350 | 6,217,481 | 4,447,703 | 193,493 | 31,294 | 28,985 | 121,196 | 94,306 |
| Fluent + filtered ICU4X | 22,246,210 | 6,285,770 | 4,490,768 | 193,493 | 31,294 | 28,986 | 189,485 | 137,372 |

All sizes are in bytes. The full build adds **189,485 bytes with gzip** and
**137,372 bytes with Brotli** over baseline, within the provisional 300 KB
compressed budget. Filtered ICU4X adds 68,289 gzip bytes or 43,066 Brotli bytes
on top of the Fluent-only build.

The catalogue sources total 1,191 bytes and the embedded provider is 4,734 bytes.
No broad ICU4X date, decimal or plural compiled-data default is enabled. The
provider includes the two shipped locales and `und` entries for the
formatters. The Web Intl variant and wider locale sets remain unmeasured.

### Reproduce the size matrix

Use the matching wasm-bindgen 0.2.129 CLI on PATH and the toolchain in
`rust-toolchain.toml`. Use Apple gzip 479 and Brotli 1.2.0 to reproduce the
compression. Check `df -h /` before each build.

```sh
out=/tmp/rake-i18n-21
export CARGO_TARGET_DIR="$out/target" CARGO_INCREMENTAL=0
export PATH="$CARGO_TARGET_DIR/debug:$PATH"
rake build --target web --package rake-view --release --output "$out/web-baseline"
rake build --target web --package rake-view --release --features i18n --output "$out/web-i18n"
rake build --target web --package rake-view --release --features i18n-icu --output "$out/web-i18n-icu"

for variant in baseline i18n i18n-icu; do
  for suffix in wasm js; do
    find "$out/web-$variant/pkg" -name "*.$suffix" -type f | sort | while IFS= read -r file; do
      printf '%s %s %s ' "$variant" "$suffix" "$file"
      printf '%s ' "$(wc -c < "$file")"
      printf '%s ' "$(gzip -n -9 -c "$file" | wc -c)"
      printf '%s\n' "$(brotli -q 9 -c "$file" | wc -c)"
    done
  done
done
```

Sum the three size columns for each variant and suffix. To regenerate
[the provider](../../examples/view/locales/icu.postcard) using the
[ICU4X data-generation tool](https://docs.rs/crate/icu4x-datagen/2.4.0):

```sh
cargo install icu4x-datagen --version 2.4.0 --locked --no-default-features \
  --features networking,blob_exporter,provider
icu4x-datagen --format blob --locales @en @pt-PT @und --cldr-tag 49.0.0 \
  --markers DecimalSymbolsV1 DecimalDigitsV1 PluralsCardinalV1 \
    DatetimePatternsDateGregorianV1 DatetimeNamesYearGregorianV1 DatetimeNamesMonthGregorianV1 \
  --out examples/view/locales/icu.postcard --overwrite
```

The `@` locale selectors restrict export to those exact entries. Their data
comes from [CLDR 49](https://github.com/unicode-org/cldr-json/releases/tag/49.0.0)
and carries the [Unicode license](../../examples/view/locales/LICENSE-UNICODE).

## Limits and API recommendations

- **#100** Keep the explicit application context and observable locale global.
  Negotiate against registered tags, with registration order breaking regional
  ties. The prototype uses ICU locale parsing and a small exact/parent/language
  matcher. Production should specify aliases, script and regional tie-breaking,
  and error types for malformed overrides. Host language changes require an
  explicit re-detect in this slice. App persistence stays with the app.
- **#101 and #102** Fluent 0.16 hard-wires `intl_pluralrules` inside
  `FluentValue::matches`. Its formatter callback controls output only. The
  prototype's `PLURAL($count)` function takes a numeric Fluent argument and
  returns the lowercase category from the catalogue's ICU4X cardinal rules.
  This same rule instance backs `locale::plural_category`. The demo selects
  only on those category strings. Raw numeric selectors still use Fluent's
  resolver, so its plural code and locale machinery still ship. Exact numeric
  keys such as `[0]` do not match the result of `PLURAL`. Fluent numbers use
  `f64`, including for integer arguments. Either adopt an upstream resolver
  hook or constrain and validate category selectors during catalogue validation
  before standardizing this API. The Fluent-only measurement runs the same FTL
  through Fluent's built-in plural matcher.
- **#101** Registration checks Fluent syntax, duplicate IDs and a required
  default. It leaves cross-catalogue coverage and argument contracts to
  follow-up tooling. `t!` resolves a message value, and Fluent still owns
  interpolation and bidi isolation. Inline paragraph runs flatten to strings.
  A translation's GPUI ID is derived from its call site, so repeated labels
  should be wrapped in an element with a stable ID. GPUI attributes taking
  `SharedString` can accept a cloned `LocalizedText` through its conversion.
- **#102** Replace the temporary `with_icu_data` setup with documented provider
  generation and a typed date-only input and length option. The slice uses a
  fixed medium Gregorian formatter and decimal default grouping. Currency,
  timestamps, zones, ordinal plurals and automatic formatting of Fluent numbers
  are outside this slice.
- **#104** Carry message keys on explicit boundaries. Matching is exact,
  returns all captured matches and has the existing scope and role filters.
  Opaque Kit descendants and flattened paragraph runs remain outside this
  coverage. The resolved locale is captured with the tree. Native platform
  screen-reader speech and language announcements were outside verification.
- **#105** Retain the feature-gated one-way Kit adapter. The prototype pins the
  Kit locale list and tries the exact tag, the bare language, then a regional
  sibling such as `pt-BR` for `pt-PT`, falling back to English only when Kit
  has no catalogue in that language. Prefer a Kit-supported catalogue inventory
  and audit cached strings in other controls. The window wrapper and the
  existing counter retain their current English copy. RTL layout remains in
  #103 and #106.
