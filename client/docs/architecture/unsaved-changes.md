# Unsaved changes in modals

A modal with a Save button holds work the reader has not yet committed: a profile's display name, a
channel's new name, a role's permissions. Every modal closes from its X, Escape, a click beside it,
and whatever its owner decides, and each of those would otherwise throw that work away in silence.
`UnsavedChangesGuard` (`src/features/layout/UnsavedChangesGuard.tsx`) stands between the modal and
its open state: a request to close while something inside has unsaved changes opens a question
instead, _Discard your changes?_, drawn with `ConfirmModal` as an `alertdialog` over the modal, and
the modal closes only when the reader presses _Discard changes_. Dismissing the question (its X,
Escape, a click beside it) keeps editing, with everything as it was; as every modal, it has no
button that only closes it. A modal nothing has changed in closes as it always did.

## Where the open state lives

The guard owns the modal's open state, or follows `isOpen` and `onOpenChange` when the modal's owner
holds it (a dialog opened from a menu, as `RenameChannelDialog` is), and hands the modal what to
open and close by through a render prop:

```tsx
<UnsavedChangesGuard
  dirty={name.trim() !== channel.name}
  isOpen={isOpen}
  onOpenChange={onOpenChange}
>
  {(guard) => (
    <ModalOverlay isOpen={guard.isOpen} onOpenChange={guard.onOpenChange} isDismissable>
      …
    </ModalOverlay>
  )}
</UnsavedChangesGuard>
```

A modal opened by a `DialogTrigger` gives the trigger `guard.isOpen` and `guard.onOpenChange` the
same way; React Aria uses the state it is given over the trigger's own, so the trigger's button
still opens it and the `Dialog`'s `close` slot, which the X in `DialogHeading` presses, reaches the
guard like every other way out. The owner's `onOpenChange` hears of a close only once it happens.

## Who says what is dirty

A modal that is one form says so itself, through the guard's `dirty` prop, compared against the
record as held: the name typed against the channel's, the mana fields against the channel's. A modal
of several forms, where each plane saves on its own (Community settings, Sign-in and security,
Email, channel access), has each form report through `useUnsavedChanges(dirty)`
(`src/features/layout/unsavedChanges.ts`), which registers with the nearest guard under a key of its
own and withdraws when the form unmounts, so a form that is put away (a rename cancelled) no longer
counts. The guard asks while any registration stands. Outside a guard the hook does nothing, so a
form that also lives somewhere other than a modal (`NicknameForm` on a profile card, `SettingsForm`
on the Administration Dashboard) needs no second version.

Dirtiness is derived, never set: each form compares what it holds to the record it was opened from,
so a change that arrives from elsewhere and makes them equal reads as clean, and a saved change
reads as clean once the server's event has updated the record. A dialog that stays mounted while it
is closed (the channel sidebar's rename and mana dialogs) starts each opening from the record again,
so a draft discarded at the last opening is gone rather than waiting to be asked about twice. The
registry is a plain set outside React state, since whether a form is dirty changes with every
keystroke and nothing is drawn differently for it until a close is requested.

## Closing after a save

Between a successful save and the event that updates the record, the form still differs from it, and
a close in that moment would ask about changes that are already saved. So a form closes after saving
through the guard rather than the overlay: `guard.close()` from the render prop, or the function
`useUnsavedChanges` returns, which tells the guard this form is clean at once, before the `Dialog`'s
`close` is called in the same handler.

## The modals guarded

Edit profile (a user's own and a bot's), a file's description in the message box, a federated
deployment's note in the dashboard, renaming a channel, a category, or a rail folder, a channel's
mana, Community settings (its name, the reader's nickname, a role, an emoji's name, a plugin's
settings), channel and category access (a preset chosen and not applied, an override not saved),
Sign-in and security (a password being changed, a passkey being renamed), and Email (an address
being typed). Forms whose button creates or sends something (an invite, a poll, a report, a
scheduled message) are not guarded: what they hold is not an edit of a record.

`e2e/modals.spec.ts` checks the question on Edit profile and on Community settings, and
`src/features/layout/UnsavedChangesGuard.test.tsx` the guard alone.
