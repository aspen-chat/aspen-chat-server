# Message search

Code: `src/features/search/SearchDialog.tsx`.

## Opening and scope

Search opens from the channel and DM headers. It searches one of:

- the channel;
- its community;
- its server;
- every deployment the user uses (`useSources`).

Each deployment is searched through its own `AspenSync.searchMessages`.

## Merging deployments

`mergeResults` merges several deployments' pages newest first.

- It shows nothing older than the oldest result of a deployment that has more to give.
- So a later page never lands above what is already shown.

## Results

- Each result renders in its deployment's `SourceScope`.
- A result is a plain-text preview with tags as names (`decodeTags`) and each spoiler as a word for
  one (`maskSpoilers`; see [Markdown](message-rendering/markdown.md#one-reading-everywhere)), since
  nothing in a result can be revealed.
- **Why:** a result is a link, and a message's Markdown may hold links of its own.
