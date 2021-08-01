use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, Data, DataEnum, DataStruct, DataUnion, DeriveInput, Field, Type,
    WherePredicate,
};

fn field_sub_types<'a>(
    f: impl IntoIterator<Item = &'a Field> + 'a,
) -> impl Iterator<Item = Type> + 'a {
    f.into_iter().map(|f| f.ty.clone())
}

fn sub_types(d: &DeriveInput) -> Vec<Type> {
    match &d.data {
        Data::Struct(DataStruct { fields, .. }) => field_sub_types(fields).collect(),
        Data::Enum(DataEnum { variants, .. }) => variants
            .iter()
            .flat_map(|v| field_sub_types(&v.fields))
            .collect(),
        Data::Union(DataUnion { fields, .. }) => field_sub_types(&fields.named).collect(),
    }
}

#[proc_macro_derive(Freeze)]
pub fn freeze(input: TokenStream) -> TokenStream {
    // Parse the input tokens into a syntax tree
    let mut input = parse_macro_input!(input as DeriveInput);
    let types = sub_types(&input);
    input.generics.make_where_clause().predicates.extend({
        let x: syn::punctuated::Punctuated<WherePredicate, syn::token::Comma> = syn::parse_quote!(
            #(#types: freeze::Freeze,)*
        );
        x
    });

    let name = &input.ident;
    let generics = &input.generics;

    let (impl_generics, type_generics, where_clause) = &generics.split_for_impl();

    let expanded = quote! {
        unsafe impl #impl_generics freeze::Freeze for #name #type_generics #where_clause {}
    };

    expanded.into()
}
