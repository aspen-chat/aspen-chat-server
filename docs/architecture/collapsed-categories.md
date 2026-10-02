# Collapsed categories

Which categories a user has folded in their channel list is theirs alone and follows them between devices (`app::category_collapse`, table `category_collapse`: the user and the category, both cascading). `PUT /categories/{category}/collapses/@me` folds one (`201`, or `200` when it was folded already) and `DELETE` on the same path unfolds it; each change is published to the user's own subject as the custom `categoryCollapseChanged` event, and community reads sideload the caller's folded categories with `include=collapses`, as `included.categoryCollapses`.
