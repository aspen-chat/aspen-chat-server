import { createContext } from "react";

/**
 * Whether the actions inside are a menu's items, as in a context menu (`ContextMenu`), so the
 * same actions are offered by a row of buttons, a touch screen's sheet, and a right click.
 */
export const ActionsAsMenuItems = createContext(false);
