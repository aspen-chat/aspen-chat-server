/**
 * Mirrors the server's password rule so a form can refuse obviously short passwords before a
 * round trip. The server remains the authority and reports `passwordRequirementsNotMet` when
 * the two disagree.
 */
export const PASSWORD_MIN_LENGTH = 8;
