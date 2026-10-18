# Collapsed categories

Which categories a user has folded in their channel list is theirs alone and follows them between devices.

| Part | Where |
| --- | --- |
| Logic | `app::category_collapse` |
| Table | `category_collapse`: the user and the category, both cascading |
| Event | Custom `categoryCollapseChanged`, on the user's own subject |
| Sideload | `include=collapses` on community reads, as `included.categoryCollapses` |

## Endpoints

| Endpoint | Does |
| --- | --- |
| `PUT /categories/{category}/collapses/@me` | Folds one (`201`, or `200` when it was folded already) |
| `DELETE /categories/{category}/collapses/@me` | Unfolds it |
| `PUT /communities/{community}/category-collapses/@me` | Folds every category of the community the user may learn of, and answers with them |
| `DELETE /communities/{community}/category-collapses/@me` | Unfolds them all |

## Rules

- Only a category the user may learn of can be folded (`app::category::viewed_category`; see [Roles and permissions](roles-and-permissions/index.md)).
- Community reads leave out folded categories the user may no longer learn of.
- Each change is published to the user's own subject as `categoryCollapseChanged`, which names
  the categories that changed: one, or each of a community's when they are folded or unfolded
  all at once (`app::category_collapse::set_all`). A community holds at most
  `MAX_CHANNELS_AND_CATEGORIES`, so that is one bounded statement and one event.
- Community reads sideload the caller's folded categories with `include=collapses`.
