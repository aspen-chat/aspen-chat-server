# Usernames

Code: `app::user::validate_username` (the rules), `app::user::named` (lookup by name), the `user_name_key` index.

## What a username may hold

A username is 1 to 32 characters. It may use any script, digits, emoji, and punctuation, except:

- `@`, which sets a tag apart.
- Whitespace, including the line and paragraph separators.
- Control characters (Cc).
- Format characters (Cf): zero-width spaces and joiners, direction marks and isolates, the soft hyphen, tags.

It must be in NFC. The client composes a new name to NFC before sending it.

**Why:** two names that look alike must not differ by an invisible character, and one name has one spelling. See [design notes](design-notes.md#usernames).

## Which names the rules apply to

- They are checked only when a name is chosen. A name already held that breaks them is kept, and signs in as it is.
- A rename follows them.
- Bots' names follow them.
- The names foreign users' homes give follow them.

The system account's name, and names that look like it, are reserved separately (`app::user::validate_new_username`; see [The system account](../threads-and-dms/system-account.md)).

## Case

Usernames are unique regardless of case: `user_name_key` indexes `lower(name)`.

- A password sign-in finds its account however the name is capitalized (`app::user::named`).
- The operator commands that take a username use `app::user::named` too.
- The per-username sign-in rate limits count every capitalization of a name as one (see [Password sign-in](password-sign-in.md#per-username-rate-limits)).
