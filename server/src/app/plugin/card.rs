//! Cards: what a message a plugin's account posts shows beneath its text, its fields and buttons
//! (`spec/plugins.md`, Cards). A card is part of its message (`message.card`, naming its
//! plugin), so whoever may read the message sees it, and an update is announced as the
//! message's. Pressing a button calls the plugin's route `aspen/cards/{message}/{button}` as the
//! person who pressed it, who must be able to read the message.

use super::PluginText;
use super::host::wit;
use super::route::{self, Answer};
use crate::api::message_enum::server_event::{MessageEvent, ServerEvent};
use crate::app::context::GlobalServerContext;
use crate::app::message::Message as MessageRow;
use crate::app::permissions::channel_access;
use crate::app::{self, ChannelId, EventScope, MessageId, UserId, publish_event};
use aspen_schema::message;
pub use aspen_wire::plugin::card::{ButtonStyle, Card, CardButton, CardField, CardValue};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};

/// The most fields a card has.
const MAX_FIELDS: usize = 25;
/// The most buttons a card has.
const MAX_BUTTONS: usize = 5;
/// The longest a plain value may be, in characters.
const MAX_PLAIN_CHARS: usize = 1000;

fn invalid(why: &str) -> wit::Error {
    wit::Error::Invalid(why.to_string())
}

fn name_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// `card` as the plugin `plugin` gave it, checked.
pub fn from_wit(plugin: &str, card: wit::Card) -> Result<Card, wit::Error> {
    if card.fields.len() > MAX_FIELDS || card.buttons.len() > MAX_BUTTONS {
        return Err(invalid(&format!(
            "a card has at most {MAX_FIELDS} fields and {MAX_BUTTONS} buttons"
        )));
    }
    let fields = card
        .fields
        .into_iter()
        .map(|field| {
            let value = match field.value {
                wit::CardValue::Plain(text) => {
                    if text.chars().count() > MAX_PLAIN_CHARS {
                        return Err(invalid(&format!(
                            "a card's text is at most {MAX_PLAIN_CHARS} characters"
                        )));
                    }
                    CardValue::Plain { text }
                }
                wit::CardValue::Time(at) => CardValue::Time {
                    at: DateTime::parse_from_rfc3339(&at)
                        .map_err(|_| invalid("a card's time is RFC 3339"))?
                        .with_timezone(&Utc),
                },
                wit::CardValue::Count(count) => CardValue::Count { count },
                wit::CardValue::Person(user) => CardValue::Person {
                    user: UserId(
                        uuid::Uuid::parse_str(&user).map_err(|_| invalid("not a person"))?,
                    ),
                },
                wit::CardValue::Link((url, text)) => {
                    if !url::Url::parse(&url).is_ok_and(|u| u.scheme() == "https") {
                        return Err(invalid("a card's link is an https URL"));
                    }
                    CardValue::Link {
                        url,
                        text: text.into(),
                    }
                }
            };
            Ok(CardField {
                label: field.label.into(),
                value,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut buttons = Vec::with_capacity(card.buttons.len());
    for button in card.buttons {
        if !name_ok(&button.id) || buttons.iter().any(|b: &CardButton| b.id == button.id) {
            return Err(invalid(
                "a button's id is 1 to 64 letters, digits, dots, hyphens, and underscores, \
                 each once",
            ));
        }
        buttons.push(CardButton {
            id: button.id,
            label: button.label.into(),
            style: match button.style {
                wit::ButtonStyle::Primary => ButtonStyle::Primary,
                wit::ButtonStyle::Secondary => ButtonStyle::Secondary,
                wit::ButtonStyle::Danger => ButtonStyle::Danger,
            },
        });
    }
    Ok(Card {
        plugin: plugin.to_string(),
        title: card.title.map(PluginText::from),
        fields,
        buttons,
    })
}

/// Changes or removes the card of `message`, which `principal` must have posted, and announces
/// it as the message's update.
pub async fn update(
    state: &GlobalServerContext,
    principal: UserId,
    message_id: MessageId,
    card: Option<Card>,
) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let changed = diesel::update(
                message::table.filter(
                    message::id
                        .eq(message_id)
                        .and(message::author.eq(principal))
                        .and(message::deleted_at.is_null()),
                ),
            )
            .set(message::card.eq(&card))
            .execute(conn.as_mut())
            .await?;
            if changed == 0 {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            publish_event(
                state,
                conn.as_mut(),
                EventScope::Message(message_id),
                &ServerEvent::Message(MessageEvent::Update {
                    id: message_id,
                    content: None,
                    attachments: None,
                    edited_at: None,
                    link_previews: None,
                    thread: None,
                    mentions: None,
                    linked_messages: None,
                    altered_by: None,
                    card: Some(card),
                    echo: None,
                }),
            )
            .await
        }
        .scope_boxed()
    })
    .await
}

/// `caller` pressing `button` on `message_id`'s card: the card's plugin answers at
/// `aspen/cards/{message}/{button}` as them. They must be able to read the message, and the
/// plugin must run where it is.
pub async fn press(
    state: &GlobalServerContext,
    caller: UserId,
    message_id: MessageId,
    button: &str,
) -> app::Result<Answer> {
    let not_found = || app::Error::Diesel(diesel::result::Error::NotFound);
    let mut conn = state.connection_pool.get().await?;
    let row: MessageRow = message::table
        .select(MessageRow::as_select())
        .filter(
            message::id
                .eq(message_id)
                .and(message::deleted_at.is_null()),
        )
        .first(conn.as_mut())
        .await?;
    let channel: ChannelId = *row.channel.id();
    channel_access(state, conn.as_mut(), caller, channel).await?;
    let card = row.card.ok_or_else(not_found)?;
    if !card.buttons.iter().any(|b| b.id == button) {
        return Err(not_found());
    }
    let home = app::events::channel_home(state, conn.as_mut(), channel).await?;
    state
        .plugins
        .runs_at(conn.as_mut(), &card.plugin, home)
        .await?
        .ok_or_else(not_found)?;
    drop(conn);
    let body = serde_json::json!({
        "message": message_id,
        "button": button,
        "channel": channel,
    });
    route::answer_host(
        state,
        &card.plugin,
        caller,
        route::Request {
            method: "POST".into(),
            path: format!("aspen/cards/{}/{button}", message_id.0),
            query: String::new(),
            content_type: Some("application/json".into()),
            body: body.to_string().into_bytes(),
        },
    )
    .await
}
