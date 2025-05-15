use proc_macro_error::{abort, proc_macro_error};
use quote::{ToTokens, format_ident, quote};
use syn::{
    Attribute, Field, Fields, ItemEnum, LitStr, MetaList, parse_macro_input, spanned::Spanned,
};

extern crate proc_macro;
/// Based on this enum we are going to generate multiple types, none of which are the input enum.
///
/// *Command, these are create, read, update, and delete commands sent via HTTPS REST.
///
/// ServerEvent, these describe to the client actions taken by other clients (or maybe the server)
///
/// The purpose of this macro is to keep the *Command and ServerEvent types in sync, as well as reduce the toil
/// surrounding managing four different enum variants for every record.
#[proc_macro_attribute]
#[proc_macro_error]
pub fn message_enum_source(
    _attr: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let en: ItemEnum = parse_macro_input!(input);
    let mut command_enums = Vec::new();
    let mut event_variants = Vec::new();
    let mut event_variant_types = Vec::new();
    for variant in en.variants {
        let mut command_variants = Vec::new();
        let mut event_sub_variants = Vec::new();
        let Fields::Named(fields) = variant.fields else {
            abort!(
                variant.ident.span(),
                "expected all enum variants to use named fields, {} does not use named fields",
                variant.ident
            );
        };
        // At least one id field is mandatory, for some records like `react` and `community_user`, all fields could be id fields.
        let mut id_fields = Vec::new();
        let mut other_fields = Vec::new();
        let mut other_permanent_fields = Vec::new();
        let mut server_authoritative_fields = Vec::new();
        for field in fields.named {
            let mut is_id = false;
            let mut is_permanent = false;
            let mut is_server_authoritative = false;
            let mut is_other = true;
            for attr in our_attrs(field.attrs.iter()) {
                let r = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("id") {
                        id_fields.push(IdField {
                            field: Field {
                                attrs: not_our_attrs(field.attrs.iter()).cloned().collect(),
                                ..field.clone()
                            },
                            client_authoritative: meta.value().map(|v| {
                                let Ok(s) = v.parse::<LitStr>() else {
                                    abort!(v.span(), "id value must be unspecified, or \"client_authoritative\"");
                                };
                                if s.value() != "client_authoritative" {
                                    abort!(v.span(), "must be \"client_authoritative\" or unspecified for default server authority")
                                }
                            }).is_ok()
                        });
                        is_other = false;
                        is_id = true;
                    }
                    if meta.path.is_ident("permanent") {
                        other_permanent_fields.push(Field {
                            attrs: not_our_attrs(field.attrs.iter()).cloned().collect(),
                            ..field.clone()
                        });
                        is_other = false;
                        is_permanent = true;
                    }
                    if meta.path.is_ident("server_authoritative") {
                        server_authoritative_fields.push(Field {
                            attrs: not_our_attrs(field.attrs.iter()).cloned().collect(),
                            ..field.clone()
                        });
                        is_other = false;
                        is_server_authoritative = true;
                    }
                    Ok(())
                });
                if let Err(e) = r {
                    abort!(
                        attr.span(),
                        "message_enum_source attribute parse failed {}",
                        e
                    );
                }
            }
            if is_id && is_server_authoritative {
                abort!(field.span(), "ids are implicitly server_authoritative, do not explicitly \
                declare them server_authoritative. If you want a client_authoritative id then you \
                can do so with `id = \"client_authoritative\"");
            }
            if is_permanent && is_id {
                abort!(field.span(), "ids are implicitly permanent, do not explicitly declare them permanent");
            }
            if is_permanent && is_server_authoritative {
                abort!(field.span(), "server_authoritative implies permanent, you don't need both")
            }
            if is_other {
                other_fields.push(field);
            }
        }
        if id_fields.is_empty() {
            abort!(
                variant.ident.span(),
                "no id field found, at least one field in each variant must be annotated with #[message_enum_source(id)]"
            )
        }

        // TODO: Can we annotate/gather the id fields for parent records to be sent in server events? This might make
        // updating client UI easier. It can probably be managed without, though the client would likely need to retain
        // an omni-list of every ID currently in its memory to do an efficient lookup.
        let id_fields_all = id_fields
            .iter()
            .map(|id_field| id_field.field.clone())
            .collect::<Vec<_>>();
        // Generate create variant with all fields except id fields which are server authoritative
        let client_auth_ids = id_fields.iter().filter_map(|id_field| {
            id_field
                .client_authoritative
                .then_some(id_field.field.clone())
        });
        command_variants.push(quote! {
            #[serde(rename_all = "camelCase")]
            Create {
                #(#client_auth_ids,)*
                #(#other_fields,)*
                #(#other_permanent_fields,)*
            }
        });
        event_sub_variants.push(quote! {
            #[serde(rename_all = "camelCase")]
            Create {
                #(#id_fields_all,)*
                #(#server_authoritative_fields,)*
                #(#other_fields,)*
                #(#other_permanent_fields,)*
            }
        });
        // Generate Read variant for command if we have any field that isn't an ID field
        if !other_fields.is_empty() || !server_authoritative_fields.is_empty() || !other_permanent_fields.is_empty() {
            command_variants.push(quote! {
                #[serde(rename_all = "camelCase")]
                Read {
                    #(#id_fields_all,)*
                }
            });
        }
        // Generate update variants however, skip it if the variant has no other fields.
        if !other_fields.is_empty() {
            // No need for a Read server event, we simply don't broadcast this.

            // Generate update variant
            command_variants.push(quote! {
                #[serde(rename_all = "camelCase")]
                Update {
                    #(#id_fields_all,)*
                    #(#other_fields,)*
                }
            });
            event_sub_variants.push(quote! {
                #[serde(rename_all = "camelCase")]
                Update {
                    #(#id_fields_all,)*
                    #(#other_fields,)*
                }
            })
        }
        // Generate delete variant
        command_variants.push(quote! {
            #[serde(rename_all = "camelCase")]
            Delete {
                #(#id_fields_all,)*
            }
        });
        event_sub_variants.push(quote! {
            #[serde(rename_all = "camelCase")]
            Delete {
                #(#id_fields_all,)*
            }
        });
        let enum_ident = format_ident!("{}Command", variant.ident);
        command_enums.push(quote! {
            #[derive(::serde::Deserialize)]
            #[serde(rename_all = "camelCase")]
            pub enum #enum_ident {
                #(#command_variants,)*
            }
        });
        let variant_ident = &variant.ident;
        event_variants.push(quote! {
            #variant_ident(#variant_ident)
        });
        event_variant_types.push(quote! {
            #[derive(::serde::Serialize)]
            #[serde(rename_all = "camelCase")]
            pub enum #variant_ident {
                #(#event_sub_variants,)*
            }
        });
    }
    quote! {
        pub mod command {
            use super::*;
            #(#command_enums)*
        }

        pub mod server_event {
            use super::*;
            use sub_variant::*;
            pub mod sub_variant {
                use super::*;
                #(#event_variant_types)*
            }
            #[derive(::serde::Serialize)]
            #[serde(rename_all = "camelCase")]
            #[serde(tag = "serverEvent")]
            pub enum ServerEvent {
                #(#event_variants),*
            }
        }
    }
    .into_token_stream()
    .into()
}

fn our_attrs<'a>(attrs: impl Iterator<Item = &'a Attribute>) -> impl Iterator<Item = &'a MetaList> {
    attrs.filter_map(|a| {
        a.path().is_ident("message_gen").then(|| {
            a.meta.require_list().unwrap_or_else(|_| {
                abort!(
                    a.span(),
                    "message_enum_source parameters must be a list, i.e. #[message_enum_source(id)]"
                )
            })
        })
    })
}

fn not_our_attrs<'a>(
    attrs: impl Iterator<Item = &'a Attribute>,
) -> impl Iterator<Item = &'a Attribute> {
    attrs.filter(|a| !a.path().is_ident("message_gen"))
}

struct IdField {
    field: Field,
    client_authoritative: bool,
}
