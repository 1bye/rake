use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ItemFn, Pat, ReturnType, Type};

pub(super) fn expand(args: TokenStream, mut function: ItemFn) -> syn::Result<TokenStream> {
    let memo = match args.to_string().as_str() {
        "" => false,
        "memo" => true,
        _ => {
            return Err(syn::Error::new_spanned(
                args,
                "component accepts only `memo`",
            ));
        }
    };
    let signature = &function.sig;

    if signature.asyncness.is_some()
        || signature.constness.is_some()
        || signature.unsafety.is_some()
        || signature.abi.is_some()
        || !signature.generics.params.is_empty()
        || signature.generics.where_clause.is_some()
        || signature.variadic.is_some()
    {
        return Err(syn::Error::new_spanned(
            signature,
            "components must be ordinary synchronous functions without generic parameters",
        ));
    }

    let Some(FnArg::Typed(scope)) = signature.inputs.first() else {
        return Err(syn::Error::new_spanned(
            signature,
            "the first parameter must be `cx: &mut Scope`",
        ));
    };
    let valid_scope = matches!(&*scope.ty, Type::Reference(reference)
        if reference.mutability.is_some() && matches!(&*reference.elem, Type::Path(path)
            if path.path.segments.last().is_some_and(|segment| segment.ident == "Scope")));

    if !valid_scope {
        return Err(syn::Error::new_spanned(
            scope,
            "the first parameter must be `cx: &mut Scope`",
        ));
    }

    if matches!(signature.output, ReturnType::Default) {
        return Err(syn::Error::new_spanned(
            signature,
            "components must return `impl IntoElement` or an element type",
        ));
    }

    let rake = crate::facade()?;
    let name = signature.ident.clone();
    let render = format_ident!("__rake_render_{}", name.to_string().to_lowercase());
    let visibility = function.vis.clone();
    let attributes = std::mem::take(&mut function.attrs);
    let mut names = Vec::new();
    let mut types = Vec::new();

    for argument in function.sig.inputs.iter().skip(1) {
        let FnArg::Typed(argument) = argument else {
            return Err(syn::Error::new_spanned(
                argument,
                "components do not accept a receiver",
            ));
        };
        let Pat::Ident(pattern) = &*argument.pat else {
            return Err(syn::Error::new_spanned(
                &argument.pat,
                "component props must be named parameters",
            ));
        };

        if pattern.by_ref.is_some() || pattern.subpat.is_some() || pattern.ident == "id" {
            return Err(syn::Error::new_spanned(
                pattern,
                "use a plain prop name; `id` is reserved for component identity",
            ));
        }

        names.push(pattern.ident.clone());
        types.push(argument.ty.clone());
    }

    let parameters: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(index, name)| format_ident!("__RakeProp{index}", span = name.span()))
        .collect();
    let equality = memo.then(|| quote!(+ ::core::cmp::PartialEq));
    let comparison = memo.then(|| {
        quote! {
            fn equivalent(&self, other: &Self) -> bool {
                self.id == other.id #(&& self.#names == other.#names)*
            }
        }
    });
    function.sig.ident = render.clone();
    function.vis = syn::Visibility::Inherited;
    let type_name = name.to_string();
    let docs = format!(
        "Props for the `{type_name}` function component. `id` is stable within its parent."
    );

    Ok(quote! {
        #(#attributes)*
        #[derive(Clone)]
        #[doc = #docs]
        #visibility struct #name<__RakeId = #rake::gpui::ElementId, #(#parameters = #types),*> {
            #[doc = "Stable identity within the parent element."]
            pub id: __RakeId,
            #(#[doc = "Component input, converted to the function parameter type when rendered."]
              pub #names: #parameters,)*
        }

        #function

        impl<__RakeId, #(#parameters),*> #rake::component::FunctionComponent
            for #name<__RakeId, #(#parameters),*>
        where
            __RakeId: ::core::clone::Clone + ::core::convert::Into<#rake::gpui::ElementId> + 'static #equality,
            #(#parameters: ::core::clone::Clone + ::core::convert::Into<#types> + 'static #equality,)*
        {
            fn component_id(&self) -> #rake::gpui::ElementId {
                self.id.clone().into()
            }

            fn render_component(&self, cx: &mut #rake::component::Scope<'_, '_>) -> #rake::gpui::AnyElement {
                #rake::gpui::IntoElement::into_any_element(#render(cx, #(self.#names.clone().into()),*))
            }

            #comparison
        }

        impl<__RakeId, #(#parameters),*> #rake::gpui::IntoElement
            for #name<__RakeId, #(#parameters),*>
        where
            Self: #rake::component::FunctionComponent,
        {
            type Element = #rake::component::ComponentElement<Self>;

            fn into_element(self) -> Self::Element {
                #rake::component::ComponentElement::new(self)
            }
        }
    })
}
