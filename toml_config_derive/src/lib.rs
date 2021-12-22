use proc_macro2::{TokenStream, TokenTree};
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Type};
use syn::{Attribute, Ident, Result, Token, Data, Fields, FieldsNamed, DataStruct};
use syn::parse::{Parse, ParseStream};


#[proc_macro_derive(TomlConfig2)]
pub fn derive_toml_config(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let mut output = quote! {
        let mut output = String::new();
    };

    output.extend(::std::iter::once(extract_comment_string(input.attrs)));

    let ident = input.ident;

    extract_from_data(ident.clone(), input.data, &mut output);

    let expanded = quote! {
        impl TomlConfig for #ident {
            fn to_string(&self) -> String {
                #output

                output
            }
        }
    };

    proc_macro::TokenStream::from(expanded)
}

fn extract_from_data(struct_name: Ident, struct_data: Data, output: &mut TokenStream) {
    let struct_data = if let Data::Struct(data) = struct_data {
        data
    } else {
        panic!("Not a struct");
    };
    let fields = if let Fields::Named(fields) = struct_data.fields {
        fields
    } else {
        panic!("Fields are not named");
    };

    for field in fields.named.into_iter() {
        let ident = field.ident.expect("Encountered unnamed field");
        let ident_str = format!("{}", ident);

        output.extend(::std::iter::once(extract_comment_string(field.attrs)));

        output.extend(::std::iter::once(quote!{
            let struct_default = #struct_name::default();
            output.push_str(&format!("{} = {}\n", #ident_str, struct_default.#ident));
        }));

        // TODO: handle struct types
        match field.ty {
            Type::Verbatim(tokens) => {
            },
            ty => {
                //panic!("Field type not verbatim: {:?}", ty);
            }
        }
    }
}

fn extract_comment_string(attrs: Vec<Attribute>) -> TokenStream {
    let mut output = String::new();

    for attr in attrs.into_iter() {
        for token_tree in attr.tokens {
            match token_tree {
                TokenTree::Literal(literal) => {
                    output.push_str(&format!("{}\n", literal));
                }
                _ => {}
            }
        }
    }

    quote! {
        output.push_str(&#output);
    }
}