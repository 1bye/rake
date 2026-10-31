//! Literal-leaf class expressions retain lazy control flow and one evaluation.

use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};
use syn::spanned::Spanned;
use syn::{Block, Expr, Pat, Stmt};

use super::super::{Diagnostics, style_plan::StylePlan};

const LITERAL_ONLY: &str = "class requires a literal utility string; use style={|s| ...} for dynamic styles; class expressions may only choose between string literals";

struct Branches<'a> {
    literal: &'a dyn Fn(&Expr) -> syn::Result<StylePlan>,
    message: &'a str,
    prefix: &'a Ident,
    next: usize,
    leaves: Vec<StylePlan>,
    errors: Diagnostics,
}

impl Branches<'_> {
    fn local(&mut self, kind: &str) -> Ident {
        let id = format_ident!(
            "{}_{}_{}",
            self.prefix,
            kind,
            self.next,
            span = Span::mixed_site()
        );

        self.next += 1;

        id
    }

    fn block(&mut self, block: &Block) -> TokenStream {
        if let [Stmt::Expr(value, None)] = block.stmts.as_slice() {
            return self.select(value);
        }

        self.errors
            .push(syn::Error::new_spanned(block, self.message));

        quote!(0usize)
    }

    fn condition(&mut self, value: &Expr) -> TokenStream {
        match value {
            Expr::Let(value) => {
                if let Err(error) = check_pattern(&value.pat) {
                    self.errors.push(error);
                }

                let expression = &value.expr;
                let pattern = &value.pat;

                quote!(::core::matches!(&(#expression), #pattern))
            }
            Expr::Binary(value) if matches!(value.op, syn::BinOp::And(_)) => {
                let left = self.condition(&value.left);
                let right = self.condition(&value.right);

                quote!((#left) && (#right))
            }
            Expr::Paren(value) => {
                let expression = self.condition(&value.expr);

                quote!((#expression))
            }
            Expr::Group(value) => self.condition(&value.expr),
            _ => quote!(#value),
        }
    }

    fn select(&mut self, value: &Expr) -> TokenStream {
        match value {
            Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(_),
                ..
            }) => match (self.literal)(value) {
                Ok(plan) => {
                    self.leaves.push(plan);
                    let leaf = self.leaves.len();

                    quote!(#leaf)
                }
                Err(error) => {
                    self.errors.push(error);

                    quote!(0usize)
                }
            },
            Expr::Paren(value) => self.select(&value.expr),
            Expr::Group(value) => self.select(&value.expr),
            Expr::Block(value) => self.block(&value.block),
            Expr::If(value) => {
                let condition = self.local("if");
                let then = self.block(&value.then_branch);

                let otherwise = value
                    .else_branch
                    .as_ref()
                    .map(|(_, value)| self.select(value))
                    .unwrap_or_else(|| quote!(0usize));

                if let Expr::Let(value) = value.cond.as_ref() {
                    if let Err(error) = check_pattern(&value.pat) {
                        self.errors.push(error);
                    }

                    let input = self.local("input");
                    let pattern = &value.pat;
                    let expression = &value.expr;

                    quote!({
                        let #input = &(#expression);
                        let #condition: bool = ::core::matches!(#input, #pattern);
                        if #condition { #then } else { #otherwise }
                    })
                } else {
                    let expression = self.condition(&value.cond);

                    quote!({
                        let #condition: bool = #expression;
                        if #condition { #then } else { #otherwise }
                    })
                }
            }
            Expr::Match(value) => {
                let input = self.local("match");
                let expression = &value.expr;
                let mut arms = Vec::new();
                let mut witness = TokenStream::new();

                for arm in &value.arms {
                    if let Err(error) = check_pattern(&arm.pat) {
                        self.errors.push(error);
                    }

                    let pattern = &arm.pat;
                    let guard = arm.guard.as_ref().map(|(_, value)| quote!(if #value));
                    let guard_witness = arm.guard.as_ref().map(|_| quote!(if false));
                    let body = self.select(&arm.body);

                    // Preserve rustc's exhaustiveness check without running guards twice.
                    witness.extend(quote!(#pattern #guard_witness => (),));
                    arms.push((quote!(#pattern #guard), body));
                }

                let mut chain = quote!(0usize);

                for (pattern, body) in arms.into_iter().rev() {
                    chain = quote!(if ::core::matches!(#input, #pattern) { #body } else { #chain });
                }

                quote!({
                    let #input = &(#expression);
                    if false { match #input { #witness } }
                    #chain
                })
            }
            _ => {
                self.errors
                    .push(syn::Error::new_spanned(value, self.message));

                quote!(0usize)
            }
        }
    }
}

fn check_pattern(pattern: &Pat) -> syn::Result<()> {
    match pattern {
        Pat::Ident(value)
            if value.by_ref.is_none()
                && value.mutability.is_none()
                && value.subpat.is_none()
                && value
                    .ident
                    .to_string()
                    .starts_with(|c: char| c.is_ascii_uppercase()) => {}
        Pat::Ident(_) | Pat::Macro(_) | Pat::Verbatim(_) => {
            return Err(syn::Error::new_spanned(
                pattern,
                "class patterns cannot bind variables; use `_`, a constant or unit variant (`None`, `Mode::Dark`), or a plain comparison",
            ));
        }
        Pat::Tuple(value) => {
            for pattern in &value.elems {
                check_pattern(pattern)?;
            }
        }
        Pat::TupleStruct(value) => {
            for pattern in &value.elems {
                check_pattern(pattern)?;
            }
        }
        Pat::Struct(value) => {
            for field in &value.fields {
                check_pattern(&field.pat)?;
            }
        }
        Pat::Or(value) => {
            for pattern in &value.cases {
                check_pattern(pattern)?;
            }
        }
        Pat::Slice(value) => {
            for pattern in &value.elems {
                check_pattern(pattern)?;
            }
        }
        Pat::Reference(value) => check_pattern(&value.pat)?,
        Pat::Paren(value) => check_pattern(&value.pat)?,
        _ => {}
    }

    Ok(())
}

pub(in crate::view) fn compile(
    value: &Expr,
    rake: &TokenStream,
    disabled: Option<&Ident>,
    prefix: &Ident,
    setup: &mut TokenStream,
) -> syn::Result<StylePlan> {
    let literal = |value: &Expr| super::class_methods(value, rake, disabled);

    compile_with(value, prefix, setup, &literal, LITERAL_ONLY)
}

pub(in crate::view) fn compile_with(
    value: &Expr,
    prefix: &Ident,
    setup: &mut TokenStream,
    literal: &dyn Fn(&Expr) -> syn::Result<StylePlan>,
    message: &str,
) -> syn::Result<StylePlan> {
    let mut branches = Branches {
        literal,
        message,
        prefix,
        next: 0,
        leaves: Vec::new(),
        errors: Diagnostics::default(),
    };

    let selection = branches.select(value);

    if let Some(errors) = branches.errors.0 {
        return Err(errors);
    }

    setup.extend(quote!(let #prefix: usize = #selection;));
    let mut plan = StylePlan::default();

    for (index, mut leaf) in branches.leaves.into_iter().enumerate() {
        let selected = format_ident!("{prefix}_leaf_{index}", span = Span::mixed_site());
        let id = index + 1;

        setup.extend(quote!(let #selected: bool = #prefix == #id;));
        leaf.gate(&selected, setup);
        plan.extend(leaf, value.span()).map_err(|error| {
            syn::Error::new(
                error.span(),
                format!(
                    "{error}, across all class branches; use the same group name in every branch"
                ),
            )
        })?;
    }

    Ok(plan)
}
