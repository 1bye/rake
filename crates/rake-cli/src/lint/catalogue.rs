//! One catalogue for diagnostics, explanations, levels, and fixes.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Level {
    Allow,
    Warn,
    Deny,
}

impl Level {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Warn => "warn",
            Self::Deny => "deny",
        }
    }
}

pub(super) struct Rule {
    pub id: &'static str,
    pub agent_summary: Option<&'static str>,
    pub level: Level,
    pub message: &'static str,
    pub explanation: &'static str,
    pub fix: &'static str,
    pub before: &'static str,
    pub after: &'static str,
}

pub(super) const STYLE: &str = "view-style-class";
pub(super) const THEME: &str = "view-theme-literal";
pub(super) const RELEASE: &str = "release-debug-assertions";
pub(super) const BUTTON: &str = "view-button-label";
pub(super) const IMAGE: &str = "view-image-alt";
pub(super) const INDEX: &str = "view-loop-index-id";
pub(super) const DROPPED_TASK: &str = "gpui-dropped-task";
pub(super) const RENDER_NEW: &str = "gpui-render-new";
pub(super) const REENTRANT: &str = "gpui-reentrant-update";
pub(super) const UI_BLOCKING: &str = "gpui-ui-blocking";
pub(super) const REASON: &str = "suppression-reason";
pub(super) const SUPPRESSION: &str = "invalid-suppression";
pub(super) const SYNTAX: &str = "syntax-error";

pub(super) const RULES: &[Rule] = &[
    Rule {
        id: STYLE,
        agent_summary: Some(
            "Always prefer exact class utilities to equivalent pure style closures.",
        ),
        level: Level::Warn,
        message: "style closure has an exact class equivalent",
        explanation: "Pure zero-argument utility method chains duplicate the class vocabulary, making intent harder for agents to recognize. Write the equivalent class utilities in the same attribute position. The syntax tier accepts only methods emitted verbatim by the shared class compiler, without conversions, captured values, side effects or variants. Existing class attributes and component properties are left alone. Comments inside the attribute prevent an automatic edit.",
        fix: "replace the style attribute with the equivalent class string, preserving attribute order",
        before: "<div style={|s| s.flex().gap_4()}/>",
        after: "<div class=\"flex gap-4\"/>",
    },
    Rule {
        id: THEME,
        agent_summary: Some(
            "Never hard-code colors, typography sizes or radii in themed views; use theme values.",
        ),
        level: Level::Warn,
        message: "literal color or design size bypasses the installed theme",
        explanation: "Literal colors, typography sizes and corner radii cannot follow a theme change. In a package with an explicit rake::theme::install call or rake::App::new(...).theme(...) chain, prefer semantic theme utilities or read the installed theme in a dynamic style. This conservative check covers arbitrary color/text/radius classes and literal rgb/rgba/hsl/hsla or px/rems constructor arguments in the corresponding style methods. Ordinary layout spacing is not diagnosed. It does not expand macros, evaluate cfg, follow aliases or prove which theme a view uses; suppress intentional samples with a reason.",
        fix: "use a semantic utility such as bg-surface, text-ink, text-body or rounded-control; keep dynamic styles for theme values",
        before: "<div class=\"bg-[#112233] text-[13px]\"/>",
        after: "<div class=\"bg-surface text-body\"/>",
    },
    Rule {
        id: RELEASE,
        agent_summary: Some(
            "Never enable debug assertions in release profiles; use a separate diagnostics profile.",
        ),
        level: Level::Warn,
        message: "debug assertions are explicitly enabled in a release profile",
        explanation: "Release builds normally disable debug assertions. Enabling this development control retains debug-only checks and cfg(debug_assertions) code in shipped artifacts. Set debug-assertions to false for release and profiles inheriting release, including explicit package/build overrides; keep intentional diagnostics builds separate or suppress this setting with a reason. Cargo profiles do not select features, so this rule never guesses release devtools selection from feature declarations or dependencies.",
        fix: "set debug-assertions = false; use the dev profile for development checks",
        before: "[profile.release]\ndebug-assertions = true",
        after: "[profile.release]\ndebug-assertions = false",
    },
    Rule {
        id: BUTTON,
        agent_summary: Some(
            "Always give icon-only or dynamic buttons an aria-label, preserving any visible words.",
        ),
        level: Level::Warn,
        message: "button content has no automatic accessible label",
        explanation: "Rake automatically labels only nonempty buttons made entirely of literal text. Icons, expressions, and control-flow children need an explicit label so assistive technology can name the action. Use the visible text as the label (label-in-name); add context only after it. For icon-only buttons, name the action.",
        fix: "add aria-label={visible_text}; use the visible text as the label (label-in-name); add context only after it",
        before: "<button>{title.clone()}</button>\n<button>{icon}</button>",
        after: "<button aria-label={title.clone()}>{title.clone()}</button>\n<button aria-label=\"Search\">{icon}</button>",
    },
    Rule {
        id: IMAGE,
        agent_summary: Some("Always supply img alt text; use alt=\"\" only for decorative images."),
        level: Level::Deny,
        message: "image is missing alt text",
        explanation: "The img tag maps alt to its accessible label. Describe meaningful images; an empty alt explicitly marks a decorative image. aria-label does not replace the required alt attribute.",
        fix: "add alt=\"Description\", or alt=\"\" for a decorative image",
        before: "<img src=\"logo.png\"/>",
        after: "<img src=\"logo.png\" alt=\"Company logo\"/>",
    },
    Rule {
        id: INDEX,
        agent_summary: Some("Never derive loop IDs from positions; use stable data keys."),
        level: Level::Warn,
        message: "loop item ID is derived from an enumerate index",
        explanation: "Position changes when items are reordered or inserted, moving retained focus and state to another item. This syntax rule recognizes index bindings from enumerate(), including rev/skip/take/step_by/by_ref adapters. It checks direct IDs, tuples, casts, arithmetic, index methods and format! arguments/captures. Arbitrary calls and data lookups are opaque; identifier spelling alone never makes a value an index.",
        fix: "replace the index in id with a stable data key, such as row.id",
        before: "for (index, row) in rows.iter().enumerate() { <div id={(\"row\", index)}/> }",
        after: "for row in &rows { <div id={(\"row\", row.id)}/> }",
    },
    Rule {
        id: DROPPED_TASK,
        agent_summary: Some(
            "Always retain or detach GPUI tasks, allocate entities before render, mutate borrowed state directly, and move blocking work off the UI thread (rake lint --deep).",
        ),
        level: Level::Warn,
        message: "GPUI task is immediately dropped",
        explanation: "Deep tier. Dropping GPUI's Task cancels its work. This rule resolves scheduler::Task, re-exported by gpui-pre 0.3.11, and reports discarded statement expressions and wildcard let bindings. Retained bindings, returned tasks and detached tasks are accepted. Borrowing a Task does not cancel it. This is not whole-program task lifetime analysis.",
        fix: "store the task for its intended lifetime, or call .detach() for independent work",
        before: "let _ = cx.spawn(async |_, _| { save().await });",
        after: "cx.spawn(async |_, _| { save().await }).detach();",
    },
    Rule {
        id: RENDER_NEW,
        agent_summary: None,
        level: Level::Warn,
        message: "entity allocated during render",
        explanation: "Deep tier. Creating an entity in GPUI Render::render or RenderOnce::render changes its identity each frame and loses retained state. The check resolves GPUI's trait and entity constructor; an unrelated method named render is accepted. Deferred closures and helper calls are not followed.",
        fix: "create the entity during initialization and keep its handle in view state",
        before: "fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement { let child = cx.new(|_| Child); div().child(child) }",
        after: "// Initialize self.child = cx.new(|_| Child) before rendering.\nfn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement { div().child(self.child.clone()) }",
    },
    Rule {
        id: REENTRANT,
        agent_summary: None,
        level: Level::Warn,
        message: "entity updated inside its own update",
        explanation: "Deep tier. GPUI mutably borrows an entity during update; updating it again panics. This conservative check recognizes cx.entity().update/update_in on a Context and nested updates of the same local entity binding in directly supplied update callbacks. Different entities and delayed closures are accepted. It does not infer identity from matching entity types, field names, aliases or arbitrary helpers.",
        fix: "mutate the state supplied to the current callback and notify, or defer the update until the borrow ends",
        before: "entity.update(cx, |_, cx| { entity.update(cx, |state, _| state.count += 1); });",
        after: "entity.update(cx, |state, cx| { state.count += 1; cx.notify(); });",
    },
    Rule {
        id: UI_BLOCKING,
        agent_summary: None,
        level: Level::Warn,
        message: "blocking operation on the UI thread",
        explanation: "Deep tier. Blocking freezes event processing and rendering. The check resolves std::fs calls, std::thread::sleep, smol::block_on (async_io) and futures::executor::block_on in GPUI render methods and callbacks directly registered with listener, on_action, on_click, mouse and key handlers. Callbacks may be inline closures or paths resolving to local functions/methods, including generic functions. A pre-pass makes registration order irrelevant, and a function registered for UI execution is checked even when also called elsewhere. Imports and aliases resolve to the same definitions. External functions, indirect function pointers, async blocks, nested unregistered closures and transitive helper calls are not followed. This deliberately misses uncertain thread ownership rather than diagnosing background work.",
        fix: "perform blocking work on a background executor, then update the entity on the UI thread",
        before: "cx.listener(|state, _: &ClickEvent, _, cx| { state.bytes = std::fs::read(\"data\").unwrap(); cx.notify(); })",
        after: "// Read data using cx.background_executor().spawn(...), retain or detach the task, then update state and notify.",
    },
    Rule {
        id: REASON,
        agent_summary: Some(
            "Always explain suppressions; use allow-file only for a whole-file decision.",
        ),
        level: Level::Warn,
        message: "lint suppression needs a reason",
        explanation: "A reason documents why a rule does not apply here. A suppression without a reason is ignored, so the original diagnostic is still reported.",
        fix: "append a nonempty reason after allow(<rule-id>) or allow-file(<rule-id>)",
        before: "// rake-lint: allow(view-button-label)",
        after: "// rake-lint: allow(view-button-label) The mounted widget supplies its own name.",
    },
    Rule {
        id: SUPPRESSION,
        agent_summary: None,
        level: Level::Warn,
        message: "invalid lint suppression",
        explanation: "Suppressions must name one known, suppressible rule and have a reason. Misspellings and malformed directives are ignored. Use allow-file only as a standalone file-header comment for an intentional decision applying throughout that file. Unused file suppressions, including duplicates and those made redundant by allow levels, are reported here; remove them when no emitted finding needs the exception. Syntax errors and suppression diagnostics cannot themselves be suppressed.",
        fix: "use a reasoned allow directive; put allow-file in the file header and remove unused file suppressions",
        before: "// rake-lint: allow(view-buton-label) Named by widget.",
        after: "// rake-lint: allow(view-button-label) Named by widget.",
    },
    Rule {
        id: SYNTAX,
        agent_summary: None,
        level: Level::Deny,
        message: "source could not be parsed",
        explanation: "The syntax tier uses syn and the real view parser, without compiling the project. Unparseable Rust or view markup prevents a complete check and is always an error. This is a parse check, not validation of all attributes, classes, types, or macro expansions.",
        fix: "correct the syntax at the reported location and run rake lint again",
        before: "<div></span>",
        after: "<div></div>",
    },
];

pub(super) fn rule(id: &str) -> Option<&'static Rule> {
    RULES.iter().find(|rule| rule.id == id)
}

pub(super) fn suppressible(id: &str) -> bool {
    matches!(id, BUTTON | IMAGE | INDEX | STYLE | THEME | RELEASE) || is_deep(id)
}

pub(super) fn is_deep(id: &str) -> bool {
    matches!(id, DROPPED_TASK | RENDER_NEW | REENTRANT | UI_BLOCKING)
}
