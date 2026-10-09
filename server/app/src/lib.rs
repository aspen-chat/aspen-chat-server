//! The API server's business logic (the `app` layer): permissions, the operations each endpoint
//! carries out, the database and the events published for them, and the background work every
//! server shares. `aspen_api` is its HTTP front end.

use diesel::Queryable;
use diesel::deserialize::FromSql;
use diesel::expression::{AsExpression, TypedExpressionType};
use diesel::pg::sql_types::Uuid as PgUuid;
use diesel::pg::{Pg, PgValue};
use diesel::serialize::ToSql;
use diesel::sql_types::SqlType;
use rand::SeedableRng as _;
use rand::rngs::SysRng;
use rand_chacha::ChaCha20Rng;
use std::cell::RefCell;
use std::error::Error as StdError;
use std::fmt::Debug;
use std::result::Result as StdResult;

pub mod activity;
pub mod admin;
pub mod aspen_config;
pub mod attachment;
pub mod ban;
pub mod benchmark;
pub mod block;
pub mod bot;
pub mod bot_command;
pub mod category;
pub mod category_collapse;
pub mod channel;
pub mod channel_mute;
pub mod channel_presence;
pub mod community;
pub mod context;
pub mod custom_emoji;
pub mod deployment;
pub mod deployment_role;
pub mod deployment_settings;
pub mod device_link;
pub mod dm;
pub mod email;
pub mod ephemeral_token;
mod error;
pub mod event_feed;
pub mod everyone_limit;
pub mod federation;
pub mod file_transfer;
pub mod fleet;
pub mod icon;
pub mod invite;
pub mod jobs;
pub mod link_preview;
pub use aspen_locale as locale;
pub mod login;
pub mod markdown;
pub mod media_store;
pub mod mention;
pub mod message;
pub mod message_link;
pub mod moderation_log;
pub mod notification_setting;
pub mod open_graph;
pub use aspen_outbound as outbound;
pub mod passkey;
pub mod permissions;
pub mod plugin;
pub mod poll;
pub mod preferences;
pub mod push;
pub mod rate_limit;
pub mod react;
pub mod read_state;
pub mod recent;
pub mod registration_invite;
pub mod report;
pub mod role;
pub mod saved_message;
pub mod search;
pub mod server_secret;
pub mod system_account;
pub mod thread;
pub mod thread_follow;
pub mod two_factor;
pub mod typing;
pub mod upload_quota;
pub mod user;
pub mod user_ban;
pub mod user_status;
pub mod visibility;
pub mod voice;
use crate::context::GlobalServerContext;
// The catalogue `t!` reads, which `aspen_locale` holds.
use aspen_locale::{_rust_i18n_try_translate, t};

thread_local! {
    pub static CHACHA_RNG: RefCell<ChaCha20Rng> = RefCell::new(ChaCha20Rng::try_from_rng(&mut SysRng).expect("failed to initialize system randomness"));
}
pub use error::Error;
pub use error::PasswordRequirement;
pub use error::Result;

// The helper macros and IDs are `aspen_wire`'s, named here as they always are.
pub use aspen_wire::{
    AnnotationId, AttachmentId, CategoryId, ChannelId, CommunityId, CustomEmojiId,
    DeploymentRoleId, FederationKeyId, HeldMessageId, IconId, LinkPreviewImageId, MessageId,
    NewsletterPostId, PasskeyId, PluginNoticeId, PollId, PushKeyId, PushSubscriptionId,
    ReportCaseId, ReportCategoryId, ReportId, RoleId, SavedMessageId, UserId, VoiceServerId,
    VoiceSessionId,
};
pub use aspen_wire::{jsonb_sql_traits, text_sql_traits, wire_name_traits};

#[derive(Debug, Clone)]
pub enum MaybeLoaded<T: Loadable> {
    Loaded(T),
    NotLoaded(T::Id),
}

impl<T: Loadable> MaybeLoaded<T> {
    pub fn id(&self) -> &T::Id {
        match self {
            MaybeLoaded::Loaded(l) => l.id(),
            MaybeLoaded::NotLoaded(id) => id,
        }
    }

    pub fn from_id(id: T::Id) -> Self {
        MaybeLoaded::NotLoaded(id)
    }

    pub async fn get(&mut self, state: &GlobalServerContext) -> anyhow::Result<&mut T> {
        match self {
            MaybeLoaded::Loaded(v) => Ok(v),
            MaybeLoaded::NotLoaded(id) => {
                let v = T::load_from_db(state, id.clone()).await?;
                *self = MaybeLoaded::Loaded(v);
                self.get(state).await
            }
        }
    }
}

pub trait Loadable: Sized {
    type Id: Clone;
    fn load_from_db(
        state: &GlobalServerContext,
        id: Self::Id,
    ) -> impl Future<Output = crate::Result<Self>> + Send;

    fn id(&self) -> &Self::Id;
}

impl<T: Loadable> Queryable<PgUuid, Pg> for MaybeLoaded<T>
where
    T::Id: From<uuid::Uuid>,
{
    type Row = uuid::Uuid;

    fn build(row: Self::Row) -> diesel::deserialize::Result<Self> {
        Ok(MaybeLoaded::NotLoaded(row.into()))
    }
}

impl<SqlType, T: Loadable> FromSql<SqlType, Pg> for MaybeLoaded<T>
where
    T::Id: FromSql<SqlType, Pg>,
{
    fn from_sql(v: PgValue) -> StdResult<Self, Box<dyn StdError + Send + Sync + 'static>> {
        Ok(Self::NotLoaded(T::Id::from_sql(v)?))
    }
}

impl<SqlType, T: Loadable + Debug> ToSql<SqlType, Pg> for MaybeLoaded<T>
where
    T::Id: Debug + ToSql<SqlType, Pg>,
{
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Pg>,
    ) -> diesel::serialize::Result {
        self.id().to_sql(out)
    }
}

impl<SqlTy: SqlType + TypedExpressionType, T: Loadable> AsExpression<SqlTy> for MaybeLoaded<T>
where
    T::Id: AsExpression<SqlTy>,
{
    type Expression = <<T as Loadable>::Id as AsExpression<SqlTy>>::Expression;

    fn as_expression(self) -> Self::Expression {
        self.id().clone().as_expression()
    }
}

impl<SqlTy: SqlType + TypedExpressionType, T: Loadable> AsExpression<SqlTy> for &MaybeLoaded<T>
where
    T::Id: AsExpression<SqlTy>,
{
    type Expression = <<T as Loadable>::Id as AsExpression<SqlTy>>::Expression;

    fn as_expression(self) -> Self::Expression {
        self.id().clone().as_expression()
    }
}

impl<SqlTy: SqlType + TypedExpressionType, T: Loadable> AsExpression<SqlTy> for &mut MaybeLoaded<T>
where
    T::Id: AsExpression<SqlTy>,
{
    type Expression = <<T as Loadable>::Id as AsExpression<SqlTy>>::Expression;

    fn as_expression(self) -> Self::Expression {
        self.id().clone().as_expression()
    }
}

pub const ASPEN_NATS_STREAM_NAME: &str = "aspen_omni_stream";

pub mod events;
pub use events::{EventScope, publish_event};
