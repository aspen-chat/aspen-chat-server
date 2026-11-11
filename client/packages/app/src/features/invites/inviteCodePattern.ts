/**
 * An invite code, a community's or a registration invite's, as a regular expression's source:
 * the server's rule (`app::invite`), ASCII letters and digits, 1 to 16 of them. Every place that
 * reads a code out of a link or a pasted text builds its pattern on this one.
 */
export const INVITE_CODE = "[A-Za-z0-9]{1,16}";
