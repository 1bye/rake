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

pub(super) const BUTTON: &str = "view-button-label";
pub(super) const IMAGE: &str = "view-image-alt";
pub(super) const INDEX: &str = "view-loop-index-id";
pub(super) const REASON: &str = "suppression-reason";
pub(super) const SUPPRESSION: &str = "invalid-suppression";
pub(super) const SYNTAX: &str = "syntax-error";

pub(super) const RULES: &[Rule] = &[
    Rule {
        id: BUTTON,
        level: Level::Warn,
        message: "button content has no automatic accessible label",
        explanation: "Rake automatically labels only nonempty buttons made entirely of literal text. Icons, expressions, and control-flow children need an explicit label so assistive technology can name the action.",
        fix: "add aria-label=\"Describe the action\" to the button",
        before: "<button>{icon}</button>",
        after: "<button aria-label=\"Search\">{icon}</button>",
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
    matches!(id, BUTTON | IMAGE | INDEX)
}
