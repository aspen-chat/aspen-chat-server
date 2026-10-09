# Reading out arrivals

Where the reader asks (`ANNOUNCE_MESSAGES`, kept with the device, under Settings, Accessibility),
`useAnnounceArrivals` reads out each message that arrives in a list on screen.

1. A message counts as arrived when `RecordStore.arrivedAt` is within the last ten seconds.
2. It is described as its author's name and its text (`describe`, as notifications word it).
3. `announce` (`src/features/layout/announce.ts`) adds a node to the one polite live region that
   `Announcer` keeps at the root.

Left unsaid:

- the reader's own messages;
- messages of people they blocked;
- history read in.
