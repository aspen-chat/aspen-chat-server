export const channelLayout = {
  tab: "Layout",
  hint: "Drag channels and categories to arrange them for everyone. Drop a channel under a category to move it there, or under “No category” to take it out.",
  listLabel: "Channels and categories",
  topLevel: "No category",
  dragCategory: "Drag {category}",
  renameChannel: "Rename #{channel}",
  deleteChannel: "Delete #{channel}",
  renameCategory: "Rename {category}",
  deleteCategory: "Delete {category}",
  deleteCategoryHeading: "Delete {category}?",
  deleteCategoryHint:
    "Its channels are kept and move out of it. Whatever access the category set for them goes with it, so check who can see them afterwards. This can't be undone.",
  confirmDeleteCategory: "Delete category",
} as const;
