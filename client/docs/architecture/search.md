# Message search

- Message search (`src/features/search/SearchDialog.tsx`) opens from the channel and DM headers
  and searches the channel, its community, its server, or every deployment the user uses
  (`useSources`), each through its own `AspenSync.searchMessages`. Several deployments' pages
  merge newest first with `mergeResults`, which shows nothing older than the oldest result of
  a deployment that has more to give, so a later page never lands above what is shown. Each
  result renders in its deployment's `SourceScope`, as a plain-text preview with tags as names
  (`decodeTags`), since a result is a link and a message's Markdown may hold links of its own.
