import type { Key } from "react-aria-components";
import { useState } from "react";

/** The key of the option that chooses no one, which no user's id (a UUID) can be. */
const ANYONE = "anyone";

/** An option of a person picker's list: a person, or the one that chooses no one. */
export interface PersonOption<T> {
  id: string;
  person: T | null;
}

/**
 * The option of a person picker's combo box that chooses no one, so a filter by a person can be
 * widened again from its list as well as by emptying the field. It is offered first while someone
 * is chosen and the list is not narrowed by typing, when `offersAnyone`.
 *
 * Once chosen, it stays the combo box's value, so the combo box closes the list as it does on
 * choosing a person: with no value it would leave the list open. Emptying the field leaves the
 * value empty instead, keeping the list open while the person types.
 */
export function useAnyoneOption<T extends { id: string }>({
  offersAnyone,
  chosen,
  narrowed,
  found,
}: {
  offersAnyone: boolean;
  chosen: T | null;
  /** Whether the list shows what the field's text found rather than everyone. */
  narrowed: boolean;
  found: readonly T[];
}) {
  const [anyoneChosen, setAnyoneChosen] = useState(false);
  const options: readonly PersonOption<T>[] = [
    ...(offersAnyone && chosen !== null && !narrowed ? [{ id: ANYONE, person: null }] : []),
    ...found.map((person) => ({ id: person.id, person })),
  ];
  return {
    options,
    value: chosen?.id ?? (anyoneChosen ? ANYONE : null),
    /** The person a key the combo box chose names, or `null` for no one. */
    personOf: (key: Key | null): T | null => {
      setAnyoneChosen(key === ANYONE);
      return found.find((p) => p.id === key) ?? null;
    },
  };
}
