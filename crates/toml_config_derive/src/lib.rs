use proc_macro2::{TokenStream, TokenTree};
use quote::quote;
use syn::{parse_macro_input, Attribute, Data, DataStruct, DeriveInput, Fields, Ident, Type};

#[proc_macro_derive(TomlConfig)]
pub fn derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    let comment = extract_comment_string(input.attrs);
    let ident = input.ident;

    match input.data {
        Data::Struct(struct_data) => {
            let mut output_stream = quote! {
                let mut output = String::new();
            };

            extract_from_struct(ident.clone(), struct_data, &mut output_stream);

            proc_macro::TokenStream::from(quote! {
                impl ::aquatic_toml_config::TomlConfig for #ident {
                    fn default_to_string() -> String {
                        let mut output = String::new();

                        let comment: Option<String> = #comment;

                        if let Some(comment) = comment {
                            output.push_str(&comment);
                            output.push('\n');
                        }

                        let body = {
                            #output_stream

                            output
                        };

                        output.push_str(&body);

                        output
                    }
                }
                impl ::aquatic_toml_config::__private::Private for #ident {
                    fn __to_string(&self, comment: Option<String>, field_name: String) -> String {
                        let mut output = String::new();

                        output.push('\n');

                        if let Some(comment) = comment {
                            output.push_str(&comment);
                        }
                        output.push_str(&format!("[{}]\n", field_name));

                        let body = {
                            #output_stream

                            output
                        };

                        output.push_str(&body);

                        output
                    }
                }
            })
        }
        Data::Enum(_) => proc_macro::TokenStream::from(quote! {
            impl ::aquatic_toml_config::__private::Private for #ident {
                fn __to_string(&self, comment: Option<String>, field_name: String) -> String {
                    let mut output = String::new();
                    let wrapping_comment: Option<String> = #comment;

                    if let Some(comment) = wrapping_comment {
                        output.push_str(&comment);
                    }

                    if let Some(comment) = comment {
                        output.push_str(&comment);
                    }

                    let value = match ::aquatic_toml_config::toml::ser::to_string(self) {
                        Ok(value) => value,
                        Err(err) => panic!("Couldn't serialize enum to toml: {:#}", err),
                    };

                    output.push_str(&format!("{} = {}\n", field_name, value));

                    output
                }
            }
        }),
        Data::Union(_) => panic!("Unions are not supported"),
    }
}

fn extract_from_struct(
    struct_ty_ident: Ident,
    struct_data: DataStruct,
    output_stream: &mut TokenStream,
) {
    let fields = if let Fields::Named(fields) = struct_data.fields {
        fields
    } else {
        panic!("Fields are not named");
    };

    output_stream.extend(::std::iter::once(quote! {
        let struct_default = #struct_ty_ident::default();
    }));

    for field in fields.named.into_iter() {
        let ident = field.ident.expect("Encountered unnamed field");
        let ident_string = format!("{}", ident);
        let comment = extract_comment_string(field.attrs);

        if let Type::Path(path) = field.ty {
            output_stream.extend(::std::iter::once(quote! {
                {
                    let comment: Option<String> = #comment;
                    let field_default: #path = struct_default.#ident;

                    let s: String = ::aquatic_toml_config::__private::Private::__to_string(
                        &field_default,
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
        let path_ident = if let Some(path_ident) = attr.path.get_ident() {
            path_ident
        } else {
            continue;
        };

        if format!("{}", path_ident) != "doc" {
            continue;
        }

        for token_tree in attr.tokens {
            if let TokenTree::Literal(literal) = token_tree {
                let mut comment = format!("{}", literal);

                // Strip leading and trailing quotation marks
                comment.remove(comment.len() - 1);
                comment.remove(0);

                // Add toml comment indicator
                comment.insert(0, '#');

                output.push_str(&comment);
                output.push('\n');
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
