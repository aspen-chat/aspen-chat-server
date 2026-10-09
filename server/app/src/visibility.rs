//! Which channels of a community each member may view, answered from memory.
//!
//! A `CommunityModel` holds what decides it: the community's owner, its roles' permissions,
//! every channel's category, and the channel and category overrides. It answers with the same
//! resolver the request handlers use (`CommunityAccess`), so the event stream and REST never
//! disagree about who sees a channel. The event feed keeps one per community its connections
//! read, loaded from the database and then kept current by the events that change it, which
//! `ModelChange::read` recognises in an event's payload. Every change is a value to set (a
//! role's permissions, an override, a channel's category), so applying a change the model
//! already reflects, or replaying a stretch of events over a model loaded partway through it,
//! ends in the same state as the database.
//!
//! What a member holds besides everyone's role is not part of the model: the feed tracks it per
//! connection, from the member's own `userCommunity` events.

use crate::permissions::{
    CommunityAccess, Override, Permission, Permissions, RoleGrant, from_names,
};
use crate::{CategoryId, ChannelId, CommunityId, RoleId, UserId};
use aspen_schema::{
    category_override, channel, channel_override, community, community_member_role, community_role,
    community_user,
};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

/// What decides who may view each channel of one community.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommunityModel {
    community: CommunityId,
    owner: Option<UserId>,
    /// Every role: its permissions, and whether it is everyone's.
    roles: HashMap<RoleId, (Permissions, bool)>,
    /// Every channel's category. A thread is not listed: it is viewed as its parent is.
    categories: HashMap<ChannelId, Option<CategoryId>>,
    channel_overrides: HashMap<ChannelId, Vec<Override>>,
    category_overrides: HashMap<CategoryId, Vec<Override>>,
}

/// One change an event makes to a `CommunityModel`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelChange {
    Owner(Option<UserId>),
    /// A role made or changed; `None` fields are unchanged.
    Role {
        id: RoleId,
        permissions: Option<Permissions>,
        everyone: Option<bool>,
    },
    RoleDeleted(RoleId),
    /// A channel deleted, and its overrides with it.
    ChannelDeleted(ChannelId),
    /// A channel made or moved.
    ChannelCategory {
        channel: ChannelId,
        category: Option<CategoryId>,
    },
    /// An override set, or with `None` cleared.
    ChannelOverride {
        channel: ChannelId,
        role: RoleId,
        set: Option<(Permissions, Permissions)>,
    },
    CategoryOverride {
        category: CategoryId,
        role: RoleId,
        set: Option<(Permissions, Permissions)>,
    },
    /// A category deleted, and its overrides with it, which its deletion's event alone tells of.
    CategoryDeleted(CategoryId),
}

/// Deserializes a list of permission names, leaving out any this version does not know, so a
/// change published by a newer server still applies what it can be understood to say.
fn known_permissions<'de, D>(deserializer: D) -> Result<Option<Vec<Permission>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<Vec<String>>::deserialize(deserializer)?
        .map(|names| names.iter().filter_map(|name| name.parse().ok()).collect()))
}

/// Deserializes a field that may be absent (unchanged), `null`, or a value.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

impl ModelChange {
    /// The change a community event makes, if any, read from its payload. Only the few kinds
    /// that can change one are parsed; the rest are passed over by their tag.
    pub fn read(payload: &str) -> Option<ModelChange> {
        const KINDS: [&str; 6] = [
            r#""serverEvent":"community""#,
            r#""serverEvent":"role""#,
            r#""serverEvent":"channel""#,
            r#""serverEvent":"category""#,
            r#""serverEvent":"channelOverride""#,
            r#""serverEvent":"categoryOverride""#,
        ];
        if !KINDS.iter().any(|kind| payload.contains(kind)) {
            return None;
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Glance {
            server_event: String,
            #[serde(rename = "type")]
            kind: String,
            id: Option<uuid::Uuid>,
            #[serde(default, deserialize_with = "present")]
            owner: Option<Option<UserId>>,
            #[serde(default, deserialize_with = "known_permissions")]
            permissions: Option<Vec<Permission>>,
            everyone: Option<bool>,
            #[serde(default, deserialize_with = "present")]
            parent_category: Option<Option<CategoryId>>,
            #[serde(default, deserialize_with = "present")]
            parent_channel: Option<Option<ChannelId>>,
            #[serde(default, deserialize_with = "present")]
            community: Option<Option<CommunityId>>,
            channel: Option<ChannelId>,
            category: Option<CategoryId>,
            role: Option<RoleId>,
            #[serde(default, deserialize_with = "known_permissions")]
            allow: Option<Vec<Permission>>,
            #[serde(default, deserialize_with = "known_permissions")]
            deny: Option<Vec<Permission>>,
        }
        let g: Glance = serde_json::from_str(payload).ok()?;
        let overridden = |g: &Glance| match (g.kind.as_str(), &g.allow, &g.deny) {
            ("delete", _, _) => Some(None),
            (_, Some(allow), Some(deny)) => Some(Some((from_names(allow), from_names(deny)))),
            _ => None,
        };
        match g.server_event.as_str() {
            "community" if g.kind == "update" => g.owner.map(ModelChange::Owner),
            "role" => {
                let id = RoleId(g.id?);
                match g.kind.as_str() {
                    "delete" => Some(ModelChange::RoleDeleted(id)),
                    _ => Some(ModelChange::Role {
                        id,
                        permissions: g.permissions.as_deref().map(from_names),
                        everyone: g.everyone,
                    }),
                }
            }
            "channel" => {
                let channel = ChannelId(g.id?);
                match g.kind.as_str() {
                    // A thread is viewed as its parent is, and a DM has no community.
                    "create"
                        if matches!(g.parent_channel, Some(None) | None)
                            && matches!(g.community, Some(Some(_))) =>
                    {
                        Some(ModelChange::ChannelCategory {
                            channel,
                            category: g.parent_category.flatten(),
                        })
                    }
                    "update" => g
                        .parent_category
                        .map(|category| ModelChange::ChannelCategory { channel, category }),
                    "delete" => Some(ModelChange::ChannelDeleted(channel)),
                    _ => None,
                }
            }
            "category" if g.kind == "delete" => {
                Some(ModelChange::CategoryDeleted(CategoryId(g.id?)))
            }
            "channelOverride" => Some(ModelChange::ChannelOverride {
                channel: g.channel?,
                role: g.role?,
                set: overridden(&g)?,
            }),
            "categoryOverride" => Some(ModelChange::CategoryOverride {
                category: g.category?,
                role: g.role?,
                set: overridden(&g)?,
            }),
            _ => None,
        }
    }
}

fn set_override(list: &mut Vec<Override>, role: RoleId, set: Option<(Permissions, Permissions)>) {
    list.retain(|o| o.role != role);
    if let Some((allow, deny)) = set {
        list.push(Override { role, allow, deny });
    }
}

impl CommunityModel {
    /// A community with no owner, roles, or channels, which its changes then fill in.
    pub fn new(community: CommunityId) -> Self {
        CommunityModel {
            community,
            owner: None,
            roles: HashMap::new(),
            categories: HashMap::new(),
            channel_overrides: HashMap::new(),
            category_overrides: HashMap::new(),
        }
    }

    pub fn apply(&mut self, change: &ModelChange) {
        match change {
            ModelChange::Owner(owner) => self.owner = *owner,
            ModelChange::Role {
                id,
                permissions,
                everyone,
            } => {
                let entry = self
                    .roles
                    .entry(*id)
                    .or_insert((Permissions::empty(), false));
                if let Some(permissions) = permissions {
                    entry.0 = *permissions;
                }
                if let Some(everyone) = everyone {
                    entry.1 = *everyone;
                }
            }
            ModelChange::RoleDeleted(id) => {
                self.roles.remove(id);
                for list in self
                    .channel_overrides
                    .values_mut()
                    .chain(self.category_overrides.values_mut())
                {
                    list.retain(|o| o.role != *id);
                }
            }
            ModelChange::ChannelDeleted(channel) => {
                self.categories.remove(channel);
                self.channel_overrides.remove(channel);
            }
            ModelChange::CategoryDeleted(category) => {
                self.category_overrides.remove(category);
            }
            ModelChange::ChannelCategory { channel, category } => {
                self.categories.insert(*channel, *category);
            }
            ModelChange::ChannelOverride { channel, role, set } => {
                set_override(
                    self.channel_overrides.entry(*channel).or_default(),
                    *role,
                    *set,
                );
            }
            ModelChange::CategoryOverride {
                category,
                role,
                set,
            } => set_override(
                self.category_overrides.entry(*category).or_default(),
                *role,
                *set,
            ),
        }
    }

    /// Whether `channel` is a live channel of the community, as opposed to one not yet made (or
    /// deleted), which nobody viewed.
    pub fn knows(&self, channel: ChannelId) -> bool {
        self.categories.contains_key(&channel)
    }

    /// Whether `user` owns the community, and so views every channel.
    pub fn is_owner(&self, user: UserId) -> bool {
        self.owner == Some(user)
    }

    /// What `user`, holding `roles` besides everyone's, may do across the community.
    fn access_of(&self, user: UserId, roles: &[RoleId]) -> CommunityAccess {
        let grants = self
            .roles
            .iter()
            .filter(|(id, (_, everyone))| *everyone || roles.contains(id))
            .map(|(id, (permissions, everyone))| RoleGrant {
                id: *id,
                // Rank plays no part in what they may see.
                position: 0,
                permissions: *permissions,
                everyone: *everyone,
            })
            .collect();
        CommunityAccess::resolve(user, self.community, self.owner == Some(user), grants)
    }

    /// Whether `user`, holding `roles` besides everyone's, holds a community permission.
    pub fn holds(&self, user: UserId, roles: &[RoleId], permission: Permissions) -> bool {
        self.access_of(user, roles).has(permission)
    }

    /// Whether `user`, holding `roles` besides everyone's, may view `channel`, a channel of
    /// this community that is not a thread.
    pub fn can_view(&self, user: UserId, roles: &[RoleId], channel: ChannelId) -> bool {
        let access = self.access_of(user, roles);
        let none = Vec::new();
        let category = self
            .categories
            .get(&channel)
            .copied()
            .flatten()
            .and_then(|category| self.category_overrides.get(&category))
            .unwrap_or(&none);
        let own = self.channel_overrides.get(&channel).unwrap_or(&none);
        access
            .in_channel(category, own)
            .contains(Permissions::VIEW_CHANNEL)
    }

    /// Whether `user`, holding `roles` besides everyone's, may learn of `category`: whether its
    /// own overrides, applied to what they may do across the community, leave them View channel
    /// there. A category the model holds no overrides of is decided by the community's
    /// permissions alone. What its channels' own overrides allow does not reveal it: a channel
    /// one may view in a category one may not is listed under no category they know.
    pub fn can_view_category(&self, user: UserId, roles: &[RoleId], category: CategoryId) -> bool {
        let none = Vec::new();
        let overrides = self.category_overrides.get(&category).unwrap_or(&none);
        self.access_of(user, roles)
            .in_channel(overrides, &none)
            .contains(Permissions::VIEW_CHANNEL)
    }

    /// The models of `communities` as the database has them. Five queries however many there
    /// are.
    pub async fn load(
        conn: &mut AsyncPgConnection,
        communities: &[CommunityId],
    ) -> crate::Result<HashMap<CommunityId, CommunityModel>> {
        let mut models: HashMap<CommunityId, CommunityModel> = HashMap::new();
        if communities.is_empty() {
            return Ok(models);
        }
        let ids = communities.to_vec();
        let owners: Vec<(CommunityId, Option<UserId>)> = community::table
            .select((community::id, community::owner))
            .filter(community::id.eq_any(&ids))
            .load(conn)
            .await?;
        for (id, owner) in owners {
            models.insert(
                id,
                CommunityModel {
                    owner,
                    ..CommunityModel::new(id)
                },
            );
        }
        let roles: Vec<(CommunityId, RoleId, Permissions, bool)> = community_role::table
            .select((
                community_role::community,
                community_role::id,
                community_role::permissions,
                community_role::everyone,
            ))
            .filter(community_role::community.eq_any(&ids))
            .filter(community_role::deleted_at.is_null())
            .load(conn)
            .await?;
        for (community, id, permissions, everyone) in roles {
            if let Some(model) = models.get_mut(&community) {
                model.roles.insert(id, (permissions, everyone));
            }
        }
        let channels: Vec<(ChannelId, Option<CommunityId>, Option<CategoryId>)> = channel::table
            .select((channel::id, channel::community, channel::parent_category))
            .filter(
                channel::community
                    .eq_any(ids.iter().map(|c| Some(*c)))
                    .and(channel::parent_channel.is_null())
                    .and(channel::deleted_at.is_null()),
            )
            .load(conn)
            .await?;
        for (id, community, category) in channels {
            if let Some(model) = community.and_then(|c| models.get_mut(&c)) {
                model.categories.insert(id, category);
            }
        }
        let channel_overrides: Vec<(Option<CommunityId>, ChannelId, Override)> =
            channel_override::table
                .inner_join(channel::table)
                .select((
                    channel::community,
                    channel_override::channel,
                    (
                        channel_override::role,
                        channel_override::allow,
                        channel_override::deny,
                    ),
                ))
                // Only live channels' overrides count (threads carry none), found through
                // `channel_community_live`.
                .filter(
                    channel::community
                        .eq_any(ids.iter().map(|c| Some(*c)))
                        .and(channel::parent_channel.is_null())
                        .and(channel::deleted_at.is_null()),
                )
                .load(conn)
                .await?;
        for (community, channel, entry) in channel_overrides {
            if let Some(model) = community.and_then(|c| models.get_mut(&c)) {
                model
                    .channel_overrides
                    .entry(channel)
                    .or_default()
                    .push(entry);
            }
        }
        let category_overrides: Vec<(CommunityId, CategoryId, Override)> = category_override::table
            .inner_join(aspen_schema::category::table)
            .select((
                aspen_schema::category::community,
                category_override::category,
                (
                    category_override::role,
                    category_override::allow,
                    category_override::deny,
                ),
            ))
            .filter(
                aspen_schema::category::community
                    .eq_any(&ids)
                    .and(aspen_schema::category::deleted_at.is_null()),
            )
            .load(conn)
            .await?;
        for (community, category, entry) in category_overrides {
            if let Some(model) = models.get_mut(&community) {
                model
                    .category_overrides
                    .entry(category)
                    .or_default()
                    .push(entry);
            }
        }
        Ok(models)
    }
}

/// The roles `user` holds in each of `communities` besides everyone's.
pub async fn member_roles(
    conn: &mut AsyncPgConnection,
    user: UserId,
    communities: &[CommunityId],
) -> crate::Result<HashMap<CommunityId, Vec<RoleId>>> {
    let rows: Vec<(CommunityId, RoleId)> = community_member_role::table
        .select((
            community_member_role::community,
            community_member_role::role,
        ))
        .filter(
            community_member_role::user
                .eq(user)
                .and(community_member_role::community.eq_any(communities.to_vec())),
        )
        .load(conn)
        .await?;
    let mut held: HashMap<CommunityId, Vec<RoleId>> = HashMap::new();
    for (community, role) in rows {
        held.entry(community).or_default().push(role);
    }
    Ok(held)
}

/// Which channels of some communities one user may view, loaded once for a read that lists
/// several kinds of thing in them. Only `load` makes one, so a reader that takes a `Visibility`
/// cannot be handed channels nobody checked; the readers of what lies in a community's channels
/// take one and keep only what it lets the user see.
pub struct Visibility {
    user: UserId,
    /// The communities it was loaded for, which its readers read.
    listed: Vec<CommunityId>,
    /// Whether the user moderates the deployment, and so views every channel.
    moderator: bool,
    models: HashMap<CommunityId, CommunityModel>,
    roles: HashMap<CommunityId, Vec<RoleId>>,
    /// Each listed channel's community.
    communities: HashMap<ChannelId, CommunityId>,
}

impl Visibility {
    pub async fn load(
        state: &crate::context::GlobalServerContext,
        user: UserId,
        communities: &[CommunityId],
    ) -> crate::Result<Self> {
        let mut conn = state.connection_pool.get().await?;
        Self::load_on(conn.as_mut(), user, communities).await
    }

    /// As `load`, on a connection the caller holds, such as inside the transaction that acts on
    /// what it allows.
    pub async fn load_on(
        conn: &mut AsyncPgConnection,
        user: UserId,
        communities: &[CommunityId],
    ) -> crate::Result<Self> {
        // A deleted community is in no view: nothing of it is listed, searched, or read.
        let live: HashSet<CommunityId> = community::table
            .select(community::id)
            .filter(
                community::id
                    .eq_any(communities)
                    .and(community::deleted_at.is_null()),
            )
            .load::<CommunityId>(conn)
            .await?
            .into_iter()
            .collect();
        let listed: Vec<CommunityId> = communities
            .iter()
            .copied()
            .filter(|community| live.contains(community))
            .collect();
        let models = CommunityModel::load(conn, &listed).await?;
        let roles = member_roles(conn, user, &listed).await?;
        let moderator = crate::deployment::is_moderator(conn, user).await?;
        let communities = models
            .iter()
            .flat_map(|(community, model)| model.categories.keys().map(|c| (*c, *community)))
            .collect();
        Ok(Visibility {
            user,
            listed,
            moderator,
            models,
            roles,
            communities,
        })
    }

    /// The user whose view this is.
    pub fn user(&self) -> UserId {
        self.user
    }

    /// The communities it covers: those it was loaded for, less any deleted.
    pub fn communities(&self) -> &[CommunityId] {
        &self.listed
    }

    /// Every channel of the communities that the user may view, threads aside (each is viewed
    /// as its parent is).
    pub fn visible_channels(&self) -> Vec<ChannelId> {
        self.communities
            .keys()
            .copied()
            .filter(|channel| self.can_view(*channel))
            .collect()
    }

    /// Whether the user may learn of `category`, a category of `community`, one of the
    /// communities it covers (`CommunityModel::can_view_category`); a deployment moderator
    /// learns of every one.
    pub fn can_view_category(&self, community: CommunityId, category: CategoryId) -> bool {
        if self.moderator {
            return true;
        }
        let none = Vec::new();
        self.models.get(&community).is_some_and(|model| {
            model.can_view_category(
                self.user,
                self.roles.get(&community).unwrap_or(&none),
                category,
            )
        })
    }

    /// Whether the user may view `channel`. A channel of none of the communities (a DM, or a
    /// thread, which is answered by its parent where it is read) is not this struct's to
    /// refuse.
    pub fn can_view(&self, channel: ChannelId) -> bool {
        let Some(community) = self.communities.get(&channel) else {
            return true;
        };
        if self.moderator {
            return true;
        }
        let none = Vec::new();
        self.models.get(community).is_some_and(|model| {
            model.can_view(
                self.user,
                self.roles.get(community).unwrap_or(&none),
                channel,
            )
        })
    }
}

/// Who of a community is online, by what decides what they may view: each distinct set of roles
/// held (the owner apart), with how many hold it, beside the community's model. Asking how many
/// may view a channel then takes one check per distinct set rather than one per person
/// ([`OnlineGroups::viewers_of`]).
pub struct OnlineGroups {
    model: CommunityModel,
    /// A member holding each set, the set, and how many hold it.
    groups: Vec<(UserId, Vec<RoleId>, u32)>,
}

impl OnlineGroups {
    /// How many of them may view `place`, a channel of the community that is not a thread.
    pub fn viewers_of(&self, place: ChannelId) -> u32 {
        self.groups
            .iter()
            .filter(|(user, roles, _)| self.model.can_view(*user, roles, place))
            .map(|(_, _, count)| *count)
            .sum()
    }
}

/// `online`'s members of `community`, grouped as [`OnlineGroups`]. Three queries however many
/// there are.
pub async fn online_groups(
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    online: &HashSet<UserId>,
) -> crate::Result<Option<OnlineGroups>> {
    let listed: Vec<UserId> = online.iter().copied().collect();
    let members: Vec<UserId> = community_user::table
        .select(community_user::user)
        .filter(community_user::community.eq(community))
        .filter(community_user::user.eq_any(&listed))
        .load(conn)
        .await?;
    let Some(model) = CommunityModel::load(conn, &[community])
        .await?
        .remove(&community)
    else {
        return Ok(None);
    };
    let mut roles: HashMap<UserId, Vec<RoleId>> = HashMap::new();
    for (user, role) in community_member_role::table
        .select((community_member_role::user, community_member_role::role))
        .filter(community_member_role::community.eq(community))
        .filter(community_member_role::user.eq_any(&members))
        .load::<(UserId, RoleId)>(conn)
        .await?
    {
        roles.entry(user).or_default().push(role);
    }
    let mut by_set: HashMap<(bool, Vec<RoleId>), (UserId, u32)> = HashMap::new();
    for user in members {
        let mut held = roles.remove(&user).unwrap_or_default();
        held.sort();
        let key = (model.owner == Some(user), held);
        by_set.entry(key).or_insert((user, 0)).1 += 1;
    }
    Ok(Some(OnlineGroups {
        groups: by_set
            .into_iter()
            .map(|((_, held), (user, count))| (user, held, count))
            .collect(),
        model,
    }))
}

/// Which of `users` belong to `community` and may view `place`, a channel of it that is not a
/// thread. Three queries however many there are.
pub async fn viewers(
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    users: &HashSet<UserId>,
    place: ChannelId,
) -> crate::Result<HashSet<UserId>> {
    let listed: Vec<UserId> = users.iter().copied().collect();
    let members: Vec<UserId> = community_user::table
        .select(community_user::user)
        .filter(community_user::community.eq(community))
        .filter(community_user::user.eq_any(&listed))
        .load(conn)
        .await?;
    let Some(model) = CommunityModel::load(conn, &[community])
        .await?
        .remove(&community)
    else {
        return Ok(HashSet::new());
    };
    let mut roles: HashMap<UserId, Vec<RoleId>> = HashMap::new();
    for (user, role) in community_member_role::table
        .select((community_member_role::user, community_member_role::role))
        .filter(community_member_role::community.eq(community))
        .filter(community_member_role::user.eq_any(&members))
        .load::<(UserId, RoleId)>(conn)
        .await?
    {
        roles.entry(user).or_default().push(role);
    }
    let none = Vec::new();
    Ok(members
        .into_iter()
        .filter(|user| model.can_view(*user, roles.get(user).unwrap_or(&none), place))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn events_are_read_as_the_changes_they_make() {
        let role = RoleId(id(1));
        let channel = ChannelId(id(2));
        let category = CategoryId(id(3));
        let user = UserId(id(4));
        assert_eq!(
            ModelChange::read(&format!(
                r#"{{"serverEvent":"role","type":"create","id":"{}","community":"{}","name":"x","position":1,"permissions":["viewChannel"],"everyone":false}}"#,
                role.0,
                id(9)
            )),
            Some(ModelChange::Role {
                id: role,
                permissions: Some(Permissions::VIEW_CHANNEL),
                everyone: Some(false),
            })
        );
        assert_eq!(
            ModelChange::read(&format!(
                r#"{{"serverEvent":"role","type":"update","id":"{}","position":2}}"#,
                role.0
            )),
            Some(ModelChange::Role {
                id: role,
                permissions: None,
                everyone: None,
            })
        );
        assert_eq!(
            ModelChange::read(&format!(
                r#"{{"serverEvent":"channelOverride","type":"delete","channel":"{}","role":"{}"}}"#,
                channel.0, role.0
            )),
            Some(ModelChange::ChannelOverride {
                channel,
                role,
                set: None,
            })
        );
        assert_eq!(
            ModelChange::read(&format!(
                r#"{{"serverEvent":"channel","type":"update","id":"{}","parentCategory":"{}"}}"#,
                channel.0, category.0
            )),
            Some(ModelChange::ChannelCategory {
                channel,
                category: Some(category),
            })
        );
        assert_eq!(
            ModelChange::read(&format!(
                r#"{{"serverEvent":"channel","type":"update","id":"{}","name":"n"}}"#,
                channel.0
            )),
            None
        );
        assert_eq!(
            ModelChange::read(&format!(
                r#"{{"serverEvent":"community","type":"update","id":"{}","owner":"{}"}}"#,
                id(9),
                user.0
            )),
            Some(ModelChange::Owner(Some(user)))
        );
        let category = CategoryId(id(10));
        assert_eq!(
            ModelChange::read(&format!(
                r#"{{"serverEvent":"category","type":"delete","id":"{}"}}"#,
                category.0
            )),
            Some(ModelChange::CategoryDeleted(category))
        );
        assert_eq!(
            ModelChange::read(&format!(
                r#"{{"serverEvent":"category","type":"update","id":"{}","name":"n"}}"#,
                category.0
            )),
            None
        );
        assert_eq!(
            ModelChange::read(r#"{"serverEvent":"message","type":"create","id":"x"}"#),
            None
        );
    }

    #[test]
    fn the_model_follows_its_changes() {
        let everyone = RoleId(id(1));
        let moderator = RoleId(id(2));
        let channel = ChannelId(id(3));
        let category = CategoryId(id(4));
        let member = UserId(id(5));
        let mut model = CommunityModel::new(CommunityId(id(6)));
        model.apply(&ModelChange::Role {
            id: everyone,
            permissions: Some(Permissions::MEMBER_TEMPLATE),
            everyone: Some(true),
        });
        model.apply(&ModelChange::Role {
            id: moderator,
            permissions: Some(Permissions::empty()),
            everyone: Some(false),
        });
        model.apply(&ModelChange::ChannelCategory {
            channel,
            category: None,
        });
        assert!(model.can_view(member, &[], channel));
        let hidden = ModelChange::ChannelOverride {
            channel,
            role: everyone,
            set: Some((Permissions::empty(), Permissions::VIEW_CHANNEL)),
        };
        model.apply(&hidden);
        // Applying a change twice is applying it once.
        model.apply(&hidden);
        assert!(!model.can_view(member, &[], channel));
        // A category's overrides come before the channel's own, so a moderator allowance on
        // the category does not lift the channel's denial for everyone...
        model.apply(&ModelChange::CategoryOverride {
            category,
            role: moderator,
            set: Some((Permissions::VIEW_CHANNEL, Permissions::empty())),
        });
        model.apply(&ModelChange::ChannelCategory {
            channel,
            category: Some(category),
        });
        assert!(!model.can_view(member, &[moderator], channel));
        // ...but with the denial moved to the category, it wins over it there.
        model.apply(&ModelChange::ChannelOverride {
            channel,
            role: everyone,
            set: None,
        });
        model.apply(&ModelChange::CategoryOverride {
            category,
            role: everyone,
            set: Some((Permissions::empty(), Permissions::VIEW_CHANNEL)),
        });
        assert!(model.can_view(member, &[moderator], channel));
        model.apply(&ModelChange::RoleDeleted(moderator));
        assert!(!model.can_view(member, &[moderator], channel));
        model.apply(&ModelChange::Owner(Some(member)));
        assert!(model.can_view(member, &[], channel));
    }
}
