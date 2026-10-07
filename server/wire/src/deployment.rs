//! Deployment permissions: the set the database stores, and their names on the wire.

use crate::t;
use diesel::{AsExpression, FromSqlRow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

bitflags::bitflags! {
    /// A set of deployment permissions, as the bits the database stores. The values are fixed:
    /// migrations write them.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, FromSqlRow, AsExpression)]
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub struct DeploymentPermissions: i64 {
        const VIEW_DASHBOARD = 1 << 0;
        const MANAGE_REGISTRATION_INVITES = 1 << 1;
        const MANAGE_VOICE_SERVERS = 1 << 2;
        const MANAGE_DEPLOYMENT_ROLES = 1 << 3;
        const MODERATE_COMMUNITIES = 1 << 4;
        const MANAGE_FEDERATION = 1 << 6;
        const REVIEW_REPORTS = 1 << 7;
        const MANAGE_REPORT_CATEGORIES = 1 << 8;
        const BAN_USERS = 1 << 9;
        const MESSAGE_ANY_USER = 1 << 10;
        const MANAGE_DEPLOYMENT_SETTINGS = 1 << 11;
        const MANAGE_PLUGINS = 1 << 12;
        const REMOVE_CONTENT = 1 << 13;
        const SEND_NEWSLETTERS = 1 << 14;
    }
}

crate::bigint_sql_traits!(DeploymentPermissions);

impl DeploymentPermissions {
    /// The powers over what people post and who may stay: given deliberately, never by `admin
    /// grant` alone.
    pub const MODERATION: Self = Self::MODERATE_COMMUNITIES
        .union(Self::REVIEW_REPORTS)
        .union(Self::REMOVE_CONTENT)
        .union(Self::BAN_USERS)
        .union(Self::MESSAGE_ANY_USER);

    /// The permissions that open the user and community directories: viewing the dashboard,
    /// and each moderation power, whose holders browse them to find what to act on.
    pub const DIRECTORIES: Self = Self::VIEW_DASHBOARD
        .union(Self::MODERATE_COMMUNITIES)
        .union(Self::REVIEW_REPORTS)
        .union(Self::REMOVE_CONTENT)
        .union(Self::BAN_USERS);

    /// What the terminal's `admin grant` gives: everything but moderation.
    pub const ADMINISTRATOR: Self = Self::all().difference(Self::MODERATION);

    /// Every bit that names a permission, and no other.
    pub fn valid(self) -> Self {
        Self::from_bits_truncate(self.bits())
    }

    /// These and every permission they include.
    pub fn with_included(self) -> Self {
        DeploymentPermission::ALL
            .iter()
            .filter(|p| self.contains(p.bits()))
            .flat_map(|p| p.includes())
            .fold(self, |all, p| all | p.bits())
    }
}

/// Something a deployment role may allow.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    strum::VariantArray,
    strum::IntoStaticStr,
)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "camelCase")]
pub enum DeploymentPermission {
    ViewDashboard,
    ManageRegistrationInvites,
    ManageVoiceServers,
    ManageDeploymentRoles,
    ModerateCommunities,
    ManageFederation,
    ReviewReports,
    RemoveContent,
    ManageReportCategories,
    BanUsers,
    MessageAnyUser,
    ManageDeploymentSettings,
    ManagePlugins,
    SendNewsletters,
}

impl DeploymentPermission {
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    pub fn bits(self) -> DeploymentPermissions {
        match self {
            Self::ViewDashboard => DeploymentPermissions::VIEW_DASHBOARD,
            Self::ManageRegistrationInvites => DeploymentPermissions::MANAGE_REGISTRATION_INVITES,
            Self::ManageVoiceServers => DeploymentPermissions::MANAGE_VOICE_SERVERS,
            Self::ManageDeploymentRoles => DeploymentPermissions::MANAGE_DEPLOYMENT_ROLES,
            Self::ModerateCommunities => DeploymentPermissions::MODERATE_COMMUNITIES,
            Self::ManageFederation => DeploymentPermissions::MANAGE_FEDERATION,
            Self::ReviewReports => DeploymentPermissions::REVIEW_REPORTS,
            Self::ManageReportCategories => DeploymentPermissions::MANAGE_REPORT_CATEGORIES,
            Self::BanUsers => DeploymentPermissions::BAN_USERS,
            Self::MessageAnyUser => DeploymentPermissions::MESSAGE_ANY_USER,
            Self::ManageDeploymentSettings => DeploymentPermissions::MANAGE_DEPLOYMENT_SETTINGS,
            Self::ManagePlugins => DeploymentPermissions::MANAGE_PLUGINS,
            Self::RemoveContent => DeploymentPermissions::REMOVE_CONTENT,
            Self::SendNewsletters => DeploymentPermissions::SEND_NEWSLETTERS,
        }
    }

    /// The permissions holding this one gives too, being part of it. None includes one that
    /// includes others, so one step finds them all.
    pub fn includes(self) -> &'static [Self] {
        match self {
            Self::ModerateCommunities => &[Self::RemoveContent],
            _ => &[],
        }
    }

    pub fn describe(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::ViewDashboard => t!("deploymentViewDashboard"),
            Self::ManageRegistrationInvites => t!("deploymentManageRegistrationInvites"),
            Self::ManageVoiceServers => t!("deploymentManageVoiceServers"),
            Self::ManageDeploymentRoles => t!("deploymentManageDeploymentRoles"),
            Self::ModerateCommunities => t!("deploymentModerateCommunities"),
            Self::ManageFederation => t!("deploymentManageFederation"),
            Self::ReviewReports => t!("deploymentReviewReports"),
            Self::ManageReportCategories => t!("deploymentManageReportCategories"),
            Self::BanUsers => t!("deploymentBanUsers"),
            Self::MessageAnyUser => t!("deploymentMessageAnyUser"),
            Self::ManageDeploymentSettings => t!("deploymentManageDeploymentSettings"),
            Self::ManagePlugins => t!("deploymentManagePlugins"),
            Self::RemoveContent => t!("deploymentRemoveContent"),
            Self::SendNewsletters => t!("deploymentSendNewsletters"),
        }
    }
}

crate::wire_name_traits!(DeploymentPermission);

pub fn to_names(permissions: DeploymentPermissions) -> Vec<DeploymentPermission> {
    DeploymentPermission::ALL
        .iter()
        .copied()
        .filter(|p| permissions.contains(p.bits()))
        .collect()
}

pub fn from_names(names: &[DeploymentPermission]) -> DeploymentPermissions {
    names.iter().map(|p| p.bits()).collect()
}

/// The terminal names deployment permissions as the API does, and lists them in its help.
#[cfg(feature = "clap")]
impl clap::ValueEnum for DeploymentPermission {
    fn value_variants<'a>() -> &'a [Self] {
        <Self as strum::VariantArray>::VARIANTS
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(<&'static str>::from(
            self,
        )))
    }
}
