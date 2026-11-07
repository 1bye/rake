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
pub(super) const REASON: &str = "suppression-reason";
pub(super) const SUPPRESSION: &str = "invalid-suppression";
pub(super) const SYNTAX: &str = "syntax-error";

pub(super) const RULES: &[Rule] = &[
    Rule {
        id: STYLE,
        level: Level::Warn,
        message: "style closure has an exact class equivalent",
        explanation: "Pure zero-argument utility method chains duplicate the class vocabulary, making intent harder for agents to recognize. Write the equivalent class utilities in the same attribute position. The syntax tier accepts only methods emitted verbatim by the shared class compiler, without conversions, captured values, side effects or variants. Existing class attributes and component properties are left alone. Comments inside the attribute prevent an automatic edit.",
        fix: "replace the style attribute with the equivalent class string, preserving attribute order",
        before: "<div style={|s| s.flex().gap_4()}/>",
        after: "<div class=\"flex gap-4\"/>",
    },
    Rule {
        id: THEME,
        level: Level::Warn,
        message: "literal color or design size bypasses the installed theme",
        explanation: "Literal colors, typography sizes and corner radii cannot follow a theme change. In a package with an explicit rake::theme::install call or rake::App::new(...).theme(...) chain, prefer semantic theme utilities or read the installed theme in a dynamic style. This conservative check covers arbitrary color/text/radius classes and literal rgb/rgba/hsl/hsla or px/rems constructor arguments in the corresponding style methods. Ordinary layout spacing is not diagnosed. It does not expand macros, evaluate cfg, follow aliases or prove which theme a view uses; suppress intentional samples with a reason.",
        fix: "use a semantic utility such as bg-surface, text-ink, text-body or rounded-control; keep dynamic styles for theme values",
        before: "<div class=\"bg-[#112233] text-[13px]\"/>",
        after: "<div class=\"bg-surface text-body\"/>",
    },
    Rule {
        id: RELEASE,
        level: Level::Warn,
        message: "debug assertions are explicitly enabled in a release profile",
        explanation: "Release builds normally disable debug assertions. Enabling this development control retains debug-only checks and cfg(debug_assertions) code in shipped artifacts. Set debug-assertions to false for release and profiles inheriting release, including explicit package/build overrides; keep intentional diagnostics builds separate or suppress this setting with a reason. Cargo profiles do not select features, so this rule never guesses release devtools selection from feature declarations or dependencies.",
        fix: "set debug-assertions = false; use the dev profile for development checks",
        before: "[profile.release]\ndebug-assertions = true",
        after: "[profile.release]\ndebug-assertions = false",
    },
    Rule {
        id: BUTTON,
        level: Level::Warn,
        message: "button content has no automatic accessible label",
        explanation: "Rake automatically labels only nonempty buttons made entirely of literal text. Icons, expressions, and control-flow children need an explicit label so assistive technology can name the action. Use the visible text as the label (label-in-name); add context only after it. For icon-only buttons, name the action.",
        fix: "add aria-label={visible_text}; use the visible text as the label (label-in-name); add context only after it",
        before: "<button>{title.clone()}</button>\n<button>{icon}</button>",
        after: "<button aria-label={title.clone()}>{title.clone()}</button>\n<button aria-label=\"Search\">{icon}</button>",
    },
    Rule {
        id: IMAGE,
        level: Level::Deny,
        message: "image is missing alt text",
        explanation: "The img tag maps alt to its accessible label. Describe meaningful images; an empty alt explicitly marks a decorative image. aria-label does not replace the required alt attribute.",
        fix: "add alt=\"Description\", or alt=\"\" for a decorative image",
        before: "<img src=\"logo.png\"/>",
        after: "<img src=\"logo.png\" alt=\"Company logo\"/>",
    },
    Rule {
        id: INDEX,
        level: Level::Warn,
        message: "loop item ID is derived from an enumerate index",
        explanation: "Position changes when items are reordered or inserted, moving retained focus and state to another item. This syntax rule recognizes index bindings from enumerate(), including rev/skip/take/step_by/by_ref adapters. It checks direct IDs, tuples, casts, arithmetic, index methods and format! arguments/captures. Arbitrary calls and data lookups are opaque; identifier spelling alone never makes a value an index.",
        fix: "replace the index in id with a stable data key, such as row.id",
        before: "for (index, row) in rows.iter().enumerate() { <div id={(\"row\", index)}/> }",
        after: "for row in &rows { <div id={(\"row\", row.id)}/> }",
    },
    Rule {
        id: REASON,
        level: Level::Warn,
        message: "lint suppression needs a reason",
        explanation: "A reason documents why a rule does not apply here. A suppression without a reason is ignored, so the original diagnostic is still reported.",
        fix: "append a nonempty reason after allow(<rule-id>)",
        before: "// rake-lint: allow(view-button-label)",
        after: "// rake-lint: allow(view-button-label) The mounted widget supplies its own name.",
    },
    Rule {
        id: SUPPRESSION,
        level: Level::Warn,
        message: "invalid lint suppression",
        explanation: "Suppressions must name one known, suppressible rule. Misspellings and malformed directives are ignored. Syntax errors and suppression diagnostics cannot themselves be suppressed inline.",
        fix: "use // rake-lint: allow(<known-rule-id>) <reason>",
        before: "// rake-lint: allow(view-buton-label) Named by widget.",
        after: "// rake-lint: allow(view-button-label) Named by widget.",
    },
    Rule {
        id: SYNTAX,
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
    matches!(id, BUTTON | IMAGE | INDEX | STYLE | THEME | RELEASE)
}
