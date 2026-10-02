use proc_macro_error::{abort, proc_macro_error};
use quote::{ToTokens, format_ident, quote};
use syn::{
    Attribute, Field, Fields, FieldsNamed, ItemEnum, LitStr, MetaList, Type, parse_macro_input,
    spanned::Spanned,
};

extern crate proc_macro;
/// Based on this enum we are going to generate multiple types, none of which are the input enum.
///
/// Record structs, one per variant. These are the wire representation of an entity and appear in
/// REST responses and in `Create` server events.
///
/// `*CreateRequest` / `*UpdateRequest`, the JSON request bodies accepted by the REST API. Every
/// identifier travels in the URL path (or is derived from the session), so request bodies only
/// carry the fields a client is allowed to set.
///
/// `ServerEvent`, these describe to the client actions taken by other clients (or maybe the server)
///
/// The purpose of this macro is to keep the request and `ServerEvent` types in sync, as well as
/// reduce the toil surrounding managing several parallel type definitions for every record.
#[proc_macro_attribute]
#[proc_macro_error]
pub fn message_enum_source(
    _attr: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let en: ItemEnum = parse_macro_input!(input);
    let mut request_structs = Vec::new();
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
        let mut parent_fields = Vec::new();
        let mut server_authoritative_fields = Vec::new();
        // Server authoritative fields the server may change after creation; carried by update
        // events so clients can follow them. Also present in `server_authoritative_fields`.
        let mut server_mutable_fields = Vec::new();
        // Basically exists just for the user password.
        let mut secret_fields = Vec::new();
        populate_field_types(
            fields,
            &mut id_fields,
            &mut other_fields,
            &mut other_permanent_fields,
            &mut parent_fields,
            &mut server_authoritative_fields,
            &mut server_mutable_fields,
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
        let variant_ident = &variant.ident.clone();
        // Create request: everything the client may set at creation time. Identifiers and parent
        // references arrive via the URL path, server authoritative fields are never accepted.
        if commands
            && (!other_fields.is_empty()
                || !other_permanent_fields.is_empty()
                || !secret_fields.is_empty())
        {
            let create_request_ident = format_ident!("{}CreateRequest", variant.ident);
            request_structs.push(quote! {
                #[derive(::serde::Deserialize, ::utoipa::ToSchema)]
                #[serde(rename_all = "camelCase")]
                pub struct #create_request_ident {
                    #(pub #other_fields,)*
                    #(pub #other_permanent_fields,)*
                    #(pub #secret_fields,)*
                }
            });
        }
        if events {
            event_sub_variants.push(quote! {
                #[serde(rename_all = "camelCase")]
                Create(super::#variant_ident)
            });
        }

        // Update request and update event. Both use JSON Merge Patch semantics: a field that is
        // absent is left unchanged, a field that is present (including an explicit `null` for
        // nullable fields) is written. Skip both when the variant has no updatable fields.
        if !other_fields.is_empty() || !server_mutable_fields.is_empty() {
            let update_request_ident = format_ident!("{}UpdateRequest", variant.ident);
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
            // Serde collapses a JSON `null` into the *outer* `None` of an `Option<Option<T>>`,
            // which would make "clear this field" indistinguishable from "leave it alone".
            // Nullable fields therefore route through `double_option`, which maps a present
            // `null` to `Some(None)`.
            // Non-nullable fields are wrapped in `Option` only to express "absent"; the schema
            // must not advertise `null` as an acceptable value for them.
            let other_fields_serde = other_fields
                .iter()
                .map(|f| {
                    if is_option(&f.ty) {
                        quote!(#[serde(default, deserialize_with = "crate::api::extract::double_option")])
                    } else {
                        quote!(#[serde(default)] #[schema(nullable = false)])
                    }
                })
                .collect::<Vec<_>>();
            let mutable_fields_ident = server_mutable_fields
                .iter()
                .map(|f| f.ident.clone())
                .collect::<Vec<_>>();
            let mutable_fields_ty = server_mutable_fields
                .iter()
                .map(|f| f.ty.clone())
                .collect::<Vec<_>>();
            let mutable_fields_attr = server_mutable_fields
                .iter()
                .map(|f| {
                    let attrs = f.attrs.clone();
                    quote!(#(#attrs)*)
                })
                .collect::<Vec<_>>();
            if commands && !other_fields.is_empty() {
                // Every field is optional, so the default is a patch that changes nothing.
                request_structs.push(quote! {
                    #[derive(::serde::Deserialize, ::utoipa::ToSchema, Default)]
                    #[serde(rename_all = "camelCase")]
                    pub struct #update_request_ident {
                        #(#other_fields_attr #other_fields_serde pub #other_fields_ident: Option<#other_fields_ty>,)*
                    }
                });
            }
            if events {
                event_sub_variants.push(quote! {
                    #[serde(rename_all = "camelCase")]
                    Update {
                        #(#id_fields_all,)*
                        #(
                            #other_fields_attr
                            #[serde(skip_serializing_if = "Option::is_none")]
                            #other_fields_ident: Option<#other_fields_ty>,
                        )*
                        #(
                            #mutable_fields_attr
                            #[serde(skip_serializing_if = "Option::is_none")]
                            #mutable_fields_ident: Option<#mutable_fields_ty>,
                        )*
                    }
                })
            }
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
                #[derive(Debug, Clone, ::serde::Serialize, ::schemars::JsonSchema)]
                #[serde(rename_all = "camelCase")]
                #[serde(tag = "type")]
                pub enum #event_variant_ident {
                    #(#event_sub_variants,)*
                }
            });
        }
        // Structure definition for use in associations (pub fields so app layer can construct these)
        record_types.push(quote! {
            #[derive(Debug, Clone, ::serde::Serialize, ::utoipa::ToSchema, ::schemars::JsonSchema)]
            #[serde(rename_all = "camelCase")]
            pub struct #variant_ident {
                #(pub #id_fields_all,)*
                #(pub #parent_fields,)*
                #(pub #server_authoritative_fields,)*
                #(pub #other_fields,)*
                #(pub #other_permanent_fields,)*
            }
        });
    }
    quote! {
        #(#record_types)*

        pub mod request {
            use super::*;
            #(#request_structs)*
        }

        pub mod server_event {
            use super::*;

            #(#event_variant_types)*

            #[derive(Debug, Clone, ::serde::Serialize, ::schemars::JsonSchema)]
            #[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
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

/// Syntactic check for `Option<...>`. The macro only sees tokens, so a type alias hiding an
/// `Option` will not be detected; the enum in `message_enum.rs` spells `Option` out directly.
fn is_option(ty: &Type) -> bool {
    match ty {
        Type::Path(p) => p
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "Option"),
        _ => false,
    }
}

/// An identifier field. Whether the client or the server chooses the value is validated when the
/// annotation is parsed but does not change what the macro generates: identifiers always travel
/// in the URL path, never in a request body.
struct IdField {
    field: Field,
}

#[allow(clippy::too_many_arguments)]
fn populate_field_types(
    fields: FieldsNamed,
    id_fields: &mut Vec<IdField>,
    other_fields: &mut Vec<Field>,
    other_permanent_fields: &mut Vec<Field>,
    parent_fields: &mut Vec<Field>,
    server_authoritative_fields: &mut Vec<Field>,
    server_mutable_fields: &mut Vec<Field>,
    secret_fields: &mut Vec<Field>,
) {
    for field in fields.named {
        let mut is_id = false;
        let mut is_permanent = false;
        let mut is_parent = false;
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
                            });
                            if let Ok(v) = meta.value() {
                                let Ok(s) = v.parse::<LitStr>() else {
                                    abort!(v.span(), "id value must be unspecified, or \"client_authoritative\"");
                                };
                                if s.value() != "client_authoritative" {
                                    abort!(v.span(), "must be \"client_authoritative\" or unspecified for default server authority")
                                }
                            }
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
                        "parent" => {
                            parent_fields.push(Field {
                                attrs: not_our_attrs(field.attrs.iter()).cloned().collect(),
                                ..field.clone()
                            });
                            is_other = false;
                            is_parent = true;
                        }
                        "server_authoritative" => {
                            let stripped = Field {
                                attrs: not_our_attrs(field.attrs.iter()).cloned().collect(),
                                ..field.clone()
                            };
                            // `server_authoritative = "mutable"`: the server may change the
                            // field after creation, so update events carry it.
                            if let Ok(v) = meta.value() {
                                let Ok(s) = v.parse::<LitStr>() else {
                                    abort!(v.span(), "server_authoritative value must be unspecified, or \"mutable\"");
                                };
                                if s.value() != "mutable" {
                                    abort!(v.span(), "must be \"mutable\" or unspecified for a field that never changes after creation")
                                }
                                server_mutable_fields.push(stripped.clone());
                            }
                            server_authoritative_fields.push(stripped);
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
        if is_parent && (is_id || is_permanent || is_server_authoritative || is_secret) {
            abort!(
                field.span(),
                "parent fields are implicitly permanent and are supplied through the URL path; \
                do not combine parent with any other annotation"
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
