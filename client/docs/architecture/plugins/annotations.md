# Annotations

Annotations are what plugins say about a message or a person.

## On messages

| Part | Name |
| --- | --- |
| Store | `RecordStore.annotations` |
| Topic | `annotations:<messageId>` |
| Hook | `useAnnotations` |
| Drawn by | `MessageAnnotations` (`src/features/plugins/Annotations.tsx`) |

How they arrive:

1. Message reads ask for `annotations`.
2. `setAnnotations` gives each message read (and each echoed reply the read names) exactly the
   annotations listed for it.
3. `messageAnnotation` events add, patch, and remove them. An `update` or `delete` finds its message
   by `RecordStore`'s note of which message each annotation is about.

Rules:

- A message's annotations go with it when it is deleted or leaves its window.
- The store passes over annotations of plugins the catalogue lacks. A plugin turned off or removed
  takes its notes with it.

### Drawing

`MessageAnnotations` draws them under a message as chips coloured by severity. Pressing one opens a
popover saying which plugin said it, with its detail and link, so nothing depends on hover.

## On people

| Part | Name |
| --- | --- |
| Read | when their card first shows them: `useUserAnnotations`, `AspenSync.loadUserAnnotations` |
| Topic | `userAnnotations:<userId>` |
| Kept current by | `userAnnotation` events |
| Drawn by | `UserAnnotations` on `ProfileCard` |

## Changed by a plugin

`MessageBody` marks a message whose `alteredBy` names plugins with `AlteredBy`, "(changed by …)",
beside the edited mark.
