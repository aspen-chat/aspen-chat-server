import { createContext } from "react";

/**
 * Opens the message's actions in their sheet from the bottom of a touch screen
 * (`MessageActionSheet`), for whatever inside the message offers more of them, as a picture's
 * menu does; `null` where the message has no sheet (with a pointer, or while it is edited).
 */
export const OpenMessageActionsContext = createContext<(() => void) | null>(null);
