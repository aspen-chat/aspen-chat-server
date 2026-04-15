use proc_macro_error::{abort, proc_macro_error};
use quote::{ToTokens, format_ident, quote};
use syn::{
    Attribute, Field, Fields, FieldsNamed, ItemEnum, LitStr, MetaList, parse_macro_input,
    spanned::Spanned,
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
    let mut command_structs = Vec::new();
    let mut subvariant_type_names = Vec::new();
    let mut event_variants = Vec::new();
    let mut event_variant_types = Vec::new();
    let mut record_types = Vec::new();
    for variant in en.variants {
        let mut events = true;
        let mut commands = true;
        let mut custom_event = false;
        for attr in our_attrs(variant.attrs.iter()) {
            let r = attr.parse_nested_meta(|meta| {
                let Some(ident) = meta.path.get_ident() else {
                    return Ok(());
                };
                match ident.to_string().as_str() {
                    "no_events" => {
                        events = false;
                    }
                    "no_commands" => {
                        commands = false;
                    }
                    "custom_event" => {
                        custom_event = true;
                        commands = false;
                        events = false;
                    }
                    _ => {
                        proc_macro_error::emit_warning!(ident.span(), "unrecognized parameter");
                    }
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
        if custom_event {
            let mut event_variant = variant.clone();
            let new_attrs = not_our_attrs(event_variant.attrs.iter());
            event_variant.attrs = new_attrs.cloned().collect();
            event_variants.push(event_variant.into_token_stream());
            continue;
        }
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
        // Basically exists just for the user password.
        let mut secret_fields = Vec::new();
        populate_field_types(
            fields,
            &mut id_fields,
            &mut other_fields,
            &mut other_permanent_fields,
            &mut server_authoritative_fields,
            &mut secret_fields,
        );
        if id_fields.is_empty() && commands && !custom_event {
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
        let variant_ident = &variant.ident.clone();
        if commands {
            let create_command_ident = format_ident!("{}CreateCommand", variant.ident);
            let create_command_response_ident =
                format_ident!("{}CreateCommandResponse", variant.ident);
            command_structs.push(quote! {
                #[derive(::serde::Deserialize, ::utoipa::ToSchema)]
                #[serde(rename_all = "camelCase")]
                pub struct #create_command_ident {
                    #(pub #client_auth_ids,)*
                    #(pub #other_fields,)*
                    #(pub #other_permanent_fields,)*
                    #(pub #secret_fields,)*
                }

                #[derive(::serde::Serialize, ::utoipa::ToSchema)]
                #[serde(rename_all = "camelCase")]
                pub enum #create_command_response_ident {
                    CreateOk(super::#variant_ident),
                    NotAllowed {
                        reason: Option<Cow<'static, str>>,
                    },
                    Error {
                        cause: Option<Cow<'static, str>>,
                    }
                }
            });
        }
        if events {
            event_sub_variants.push(quote! {
                #[serde(rename_all = "camelCase")]
                Create(super::#variant_ident)
            });
        }
        // Generate Read variant for command if we have any field that isn't an ID field
        if commands
            && (!other_fields.is_empty()
                || !server_authoritative_fields.is_empty()
                || !other_permanent_fields.is_empty())
        {
            let read_command_ident = format_ident!("{}ReadCommand", variant.ident);
            let read_command_response_ident = format_ident!("{}ReadCommandResponse", variant.ident);
            command_structs.push(quote! {
                #[derive(::serde::Deserialize, ::utoipa::ToSchema)]
                #[serde(rename_all = "camelCase")]
                pub struct #read_command_ident {
                    #(pub #id_fields_all,)*
                }

                #[derive(::serde::Serialize, ::utoipa::ToSchema)]
                #[serde(rename_all = "camelCase")]
                pub enum #read_command_response_ident {
                    #variant_ident(super::#variant_ident),
                    NotAllowed {
                        reason: Option<String>,
                    },
                    Error {
                        cause: Option<String>,
                    }
                }
            });
        }

        // Generate update variants however, skip it if the variant has no other fields.
        if !other_fields.is_empty() {
            // No need for a Read server event, we simply don't broadcast this.
            let update_command_ident = format_ident!("{}UpdateCommand", variant.ident);
            let update_command_response_ident =
                format_ident!("{}UpdateCommandResponse", variant.ident);
            let other_fields_ident = other_fields
                .iter()
                .map(|f| f.ident.clone())
                .collect::<Vec<_>>();
            let other_fields_ty = other_fields
                .iter()
                .map(|f| f.ty.clone())
                .collect::<Vec<_>>();
            let other_fields_attr = other_fields
                .iter()
                .map(|f| {
                    let attrs = f.attrs.clone();
                    quote!(#(#attrs)*)
                })
                .collect::<Vec<_>>();
            // Generate update variant
            command_structs.push(quote! {
                #[derive(::serde::Deserialize, ::utoipa::ToSchema)]
                #[serde(rename_all = "camelCase")]
                pub struct #update_command_ident {
                    #(pub #id_fields_all,)*
                    #(#other_fields_attr pub #other_fields_ident: Option<#other_fields_ty>,)*
                }

                #[derive(::serde::Serialize, ::utoipa::ToSchema)]
                #[serde(rename_all = "camelCase")]
                pub enum #update_command_response_ident {
                    UpdateOk,
                    NotAllowed {
                        reason: Option<String>,
                    },
                    Error {
                        cause: Option<String>,
                    }
                }
            });
            if events {
                event_sub_variants.push(quote! {
                    #[serde(rename_all = "camelCase")]
                    Update {
                        #(#id_fields_all,)*
                        #(#other_fields_attr #other_fields_ident: Option<#other_fields_ty>,)*
                    }
                })
            }
        }
        // Generate delete variant
        if commands {
            let delete_command_ident = format_ident!("{}DeleteCommand", variant.ident);
            let delete_command_response_ident =
                format_ident!("{}DeleteCommandResponse", variant.ident);
            command_structs.push(quote! {
                #[derive(::serde::Deserialize, ::utoipa::ToSchema)]
                #[serde(rename_all = "camelCase")]
                pub struct #delete_command_ident {
                    #(pub #id_fields_all,)*
                }

                #[derive(::serde::Serialize, ::utoipa::ToSchema)]
                #[serde(rename_all = "camelCase")]
                pub enum #delete_command_response_ident {
                    DeleteOk,
                    NotAllowed {
                        reason: Option<String>,
                    },
                    Error {
                        cause: Option<String>,
                    }
                }
            });
        }
        if events {
            event_sub_variants.push(quote! {
                #[serde(rename_all = "camelCase")]
                Delete {
                    #(#id_fields_all,)*
                }
            });
            let event_variant_ident = format_ident!("{}Event", variant.ident);
            event_variants.push(quote! {
                #variant_ident(#event_variant_ident)
            });
            event_variant_types.push(quote! {
                #[derive(::serde::Serialize, ::schemars::JsonSchema)]
                #[serde(rename_all = "camelCase")]
                #[serde(tag = "type")]
                pub enum #event_variant_ident {
                    #(#event_sub_variants,)*
                }
            });
            subvariant_type_names.push(variant.ident.clone());
        }
        // Structure definition for use in associations (pub fields so app layer can construct these)
        record_types.push(quote! {
            #[derive(::serde::Serialize, ::utoipa::ToSchema, ::schemars::JsonSchema)]
            #[serde(rename_all = "camelCase")]
            pub struct #variant_ident {
                #(pub #id_fields_all,)*
                #(pub #server_authoritative_fields,)*
                #(pub #other_fields,)*
                #(pub #other_permanent_fields,)*
            }
        });
    }
    quote! {
        use std::borrow::Cow;

        #(#record_types)*

        pub mod command {
            use super::*;
            #(#command_structs)*
        }

        pub mod server_event {
            use super::*;

            #(#event_variant_types)*

            #[derive(::serde::Serialize, ::schemars::JsonSchema)]
            #[serde(rename_all = "camelCase")]
            #[serde(tag = "serverEvent")]
            #[schemars(inline)]
            pub enum ServerEvent {
                #(#event_variants),*
            }
        }
    }
    .into_token_stream()
    .into()
}

fn our_attrs<'a>(attrs: impl Iterator<Item = &'a Attribute>) -> impl Iterator<Item = &'a MetaList> {
    attrs.filter(|a| a.path().is_ident("message_gen")).map(|a| {
        a.meta.require_list().unwrap_or_else(|_| {
            abort!(
                a.span(),
                "message_enum_source parameters must be a list, i.e. #[message_enum_source(id)]"
            )
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

fn populate_field_types(
    fields: FieldsNamed,
    id_fields: &mut Vec<IdField>,
    other_fields: &mut Vec<Field>,
    other_permanent_fields: &mut Vec<Field>,
    server_authoritative_fields: &mut Vec<Field>,
    secret_fields: &mut Vec<Field>,
) {
    for field in fields.named {
        let mut is_id = false;
        let mut is_permanent = false;
        let mut is_server_authoritative = false;
        let mut is_secret = false;
        let mut is_other = true;
        for attr in our_attrs(field.attrs.iter()) {
            let r = attr.parse_nested_meta(|meta| {
                    let ident = meta.path.get_ident().expect("unrecognized value");
                    match ident.to_string().as_str() {
                        "id" => {
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
                        "permanent" => {
                            other_permanent_fields.push(Field {
                                attrs: not_our_attrs(field.attrs.iter()).cloned().collect(),
                                ..field.clone()
                            });
                            is_other = false;
                            is_permanent = true;
                        }
                        "server_authoritative" => {
                            server_authoritative_fields.push(Field {
                                attrs: not_our_attrs(field.attrs.iter()).cloned().collect(),
                                ..field.clone()
                            });
                            is_other = false;
                            is_server_authoritative = true;
                        }
                        "secret" => {
                            secret_fields.push(Field {
                                attrs: not_our_attrs(field.attrs.iter()).cloned().collect(),
                                ..field.clone()
                            });
                            is_other = false;
                            is_secret = true;
                        }
                        _ => {
                            proc_macro_error::emit_warning!(ident.span(), "unrecognized parameter");
                        }
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
            abort!(
                field.span(),
                "ids are implicitly server_authoritative, do not explicitly \
                declare them server_authoritative. If you want a client_authoritative id then you \
                can do so with `id = \"client_authoritative\""
            );
        }
        if is_permanent && is_id {
            abort!(
                field.span(),
                "ids are implicitly permanent, do not explicitly declare them permanent"
            );
        }
        if is_permanent && is_server_authoritative {
            abort!(
                field.span(),
                "server_authoritative implies permanent, you don't need both"
            )
        }
        if is_secret && is_server_authoritative {
            abort!(
                field.span(),
                "secret fields are always client authoritative"
            )
        }
        if is_secret && is_id {
            abort!(
                field.span(),
                "id fields are widely distributed and thus cannot be secret"
            )
        }
        if is_secret && is_permanent {
            abort!(
                field.span(),
                "the combination of secret and permanent is not implemented"
            )
        }
        if is_other {
            other_fields.push(field);
        }
    }
}
