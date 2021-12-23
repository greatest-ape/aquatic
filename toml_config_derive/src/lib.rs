use proc_macro2::{TokenStream, TokenTree};
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Type, Attribute, Ident, Data, Fields};


#[proc_macro_derive(TomlConfig)]
pub fn derive_toml_config(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let mut output = quote! {
        let mut output = String::new();
    };

    let comment = extract_comment_string(input.attrs);

    let ident = input.ident;

    extract_from_data(ident.clone(), input.data, &mut output);

    let expanded = quote! {
        impl TomlConfig for #ident {
            fn to_string(&self) -> String {
                let mut output = String::new();

                let comment: Option<String> = #comment;

                if let Some(comment) = comment {
                    output.push_str(&comment);
                    output.push('\n');
                }

                let body = {
                    #output

                    output
                };

                output.push_str(&body);

                output
            }
            fn to_string_internal(&self, comment: Option<String>, field_name: String) -> String {
                let mut output = String::new();

                output.push('\n');

                if let Some(comment) = comment {
                    output.push_str(&comment);
                }
                output.push_str(&format!("[{}]\n", field_name));

                let body = {
                    #output

                    output
                };

                output.push_str(&body);

                output
            }
        }
    };

    proc_macro::TokenStream::from(expanded)
}

fn extract_from_data(struct_ty_ident: Ident, struct_data: Data, output: &mut TokenStream) {
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

    output.extend(::std::iter::once(quote!{
        let struct_default = #struct_ty_ident::default();
    }));

    for field in fields.named.into_iter() {
        let ident = field.ident.expect("Encountered unnamed field");
        let ident_string = format!("{}", ident);
        let comment = extract_comment_string(field.attrs);

        if let Type::Path(path) = field.ty {
            output.extend(::std::iter::once(quote!{
                {
                    let comment: Option<String> = #comment;

                    let s: String = ::toml_config::TomlConfig::to_string_internal(
                        &#path::default(),
                        comment,
                        #ident_string.to_string()
                    );
                    output.push_str(&s);
                }
            }));
        }
    }
}

fn extract_comment_string(attrs: Vec<Attribute>) -> TokenStream {
    let mut output = String::new();

    for attr in attrs.into_iter() {
        for token_tree in attr.tokens {
            match token_tree {
                TokenTree::Literal(literal) => {
                    {
                        let mut comment = format!("{}", literal);

                        comment.remove(comment.len() - 1);
                        comment.remove(0);
                        comment.insert(0, '#');

                        output.push_str(&comment);
                        output.push('\n');
                    }
                }
                _ => {}
            }
        }
    }

    if output.is_empty() {
        quote! {
            None
        }
    } else {
        quote! {
            Some(#output.to_string())
        }
    }
}