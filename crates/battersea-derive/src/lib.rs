//! Attach an inline catalogue manifest to a Rust node type.
//!
//! `#[derive(NodeDefinition)]` requires
//! `#[node_definition(manifest = "...one-node manifest...")]`.
//! Set `crate = renamed_flow_crate` when `battersea-flow` has been renamed.
//! Execution remains an explicit `NodeHandler` implementation.
#![forbid(unsafe_code)]
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, LitStr, Path};

#[proc_macro_derive(NodeDefinition, attributes(node_definition))]
pub fn node_definition(input: TokenStream) -> TokenStream {
    expand(parse_macro_input!(input as DeriveInput))
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let mut manifest: Option<LitStr> = None;
    let mut crate_path: Option<Path> = None;
    for attribute in &input.attrs {
        if !attribute.path().is_ident("node_definition") {
            continue;
        }
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("manifest") {
                if manifest.is_some() {
                    return Err(meta.error("duplicate manifest"));
                }
                manifest = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("crate") {
                if crate_path.is_some() {
                    return Err(meta.error("duplicate crate path"));
                }
                crate_path = Some(meta.value()?.parse()?);
            } else {
                return Err(meta.error("expected manifest or crate"));
            }
            Ok(())
        })?;
    }
    let manifest = manifest.ok_or_else(|| {
        syn::Error::new_spanned(
            &input.ident,
            "NodeDefinition requires #[node_definition(manifest = \"...\")]",
        )
    })?;
    let value: serde_yaml::Value = serde_yaml::from_str(&manifest.value())
        .map_err(|error| syn::Error::new_spanned(&manifest, error.to_string()))?;
    let definitions = value
        .get("node_definitions")
        .unwrap_or(&value)
        .as_sequence();
    if definitions.is_none_or(|entries| entries.len() != 1) {
        return Err(syn::Error::new_spanned(
            &manifest,
            "NodeDefinition requires exactly one node definition",
        ));
    }
    let crate_path = crate_path.unwrap_or_else(|| syn::parse_quote!(::battersea_flow));
    let name = &input.ident;
    let (implementation, types, constraints) = input.generics.split_for_impl();
    Ok(quote! {
        impl #implementation #crate_path::catalog::NodeDefinition for #name #types #constraints {
            const DEFINITION_MANIFEST: &'static str = #manifest;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_or_ambiguous_macro_inputs_are_compile_errors() {
        for source in [
            "struct Missing;",
            "#[node_definition(manifest = \"[\")] struct BadYaml;",
            "#[node_definition(manifest = \"node_definitions: []\")] struct Empty;",
            "#[node_definition(manifest = \"[{},{}]\")] struct Multiple;",
            "#[node_definition(unknown = true)] struct Unknown;",
            "#[node_definition(manifest = \"[{}]\", manifest = \"[{}]\")] struct Duplicate;",
            "#[node_definition(crate = a, crate = b, manifest = \"[{}]\")] struct Crate;",
        ] {
            assert!(expand(syn::parse_str(source).unwrap()).is_err(), "{source}");
        }
    }
}
