use proc_macro::{TokenStream};
use proc_macro2::TokenTree;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Type};
use syn::{Attribute, Ident, Result, Token, Data, Fields, FieldsNamed, DataStruct};
use syn::parse::{Parse, ParseStream};

#[proc_macro_derive(TomlConfig)]
pub fn derive_toml_config(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let mut output = String::new();

    output.push_str(&extract_comment_string(input.attrs));

    let ident = input.ident;

    extract_from_data(input.data, &mut output);

    let expanded = quote! {
        impl TomlConfig for #ident {
            fn to_string(&self) -> String {
                (#output).to_string()
            }
        }
    };

    TokenStream::from(expanded)
}

fn extract_from_data(data: Data, output: &mut String) {
    let struct_data = if let Data::Struct(data) = data {
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
        let comments = extract_comment_string(field.attrs);

        output.push_str(&comments);

        match field.ty {
            Type::Verbatim(tokens) => {
            },
            ty => {
                //panic!("Field type not verbatim: {:?}", ty);
            }
        }
    }
}

fn extract_comment_string(attrs: Vec<Attribute>) -> String {
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

    output
}