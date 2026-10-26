# Usernames

Code: `app::user::validate_username` (the rules), `app::user::named` (lookup by name), the
`user_name_key` index.

## What a username may hold

A username is 1 to 32 characters. It may use any script, digits, emoji, and punctuation, except:

- `@`, which sets a tag apart.
- Whitespace, including the line and paragraph separators.
- Control characters (Cc).
- Format characters (Cf): zero-width spaces and joiners, direction marks and isolates, the soft
  hyphen, tags.

It must be in NFC. The client composes a name to NFC before registering it.

**Why:** two names that look alike must not differ by an invisible character, and one name has one
spelling. See [design notes](design-notes.md#usernames).

## Which names the rules apply to

- They are checked only when a name is chosen. A name already held that breaks them is kept, and
  signs in as it is.
- Bots' names follow them.
- The names foreign users' homes give follow them.

The system account's name, and names that look like it, are reserved separately
(`app::user::validate_new_username`; see
[The system account](../threads-and-dms/system-account.md)).

## Changing a username

Nobody changes their own username, or their bot's: it is chosen when the account is made. The
profile update (`PATCH /users/{user}`) does not take `name`, and a request giving it is refused with
`validation` (`usernameFixed`) rather than ignored (`api::extract::refusing`). The server alone
renames, announcing it as the user's `update` (`User.name` is `permanent = "server_mutable"`):

- A moderator resolving a report of a profile resets the name to a placeholder (`app::report`,
  through `app::user::apply_profile_update`'s `rename`).
- A foreign user's home gives a new name with a sign-in, kept when it follows the rules above
  (`app::federation::abroad`).

## Case

Usernames are unique regardless of case: `user_name_key` indexes `lower(name)`.

- A password sign-in finds its account however the name is capitalized (`app::user::named`).
- The operator commands that take a username use `app::user::named` too.
- The per-username sign-in rate limits count every capitalization of a name as one (see
  [Password sign-in](password-sign-in.md#per-username-rate-limits)).
