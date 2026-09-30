import type { ReactNode } from "react";

/** One completion offered at the caret. */
export interface Suggestion {
  readonly key: string;
  readonly label: string;
  readonly detail: string | null;
  /** A line beneath, such as what a command does. */
  readonly description?: string;
  readonly icon: ReactNode;
}

/**
 * The completions offered above a message box: a listbox the box names with `aria-controls`,
 * its chosen option named by `aria-activedescendant` (`${id}-${index}`), focus staying in the
 * box. `header` goes above the options, and shows when there are none.
 */
export function SuggestionList<S extends Suggestion>({
  id,
  label,
  suggestions,
  current,
  onPick,
  header,
}: {
  id: string;
  label: string;
  suggestions: readonly S[];
  current: number;
  onPick: (suggestion: S, box: HTMLTextAreaElement | null) => void;
  header?: ReactNode;
}) {
  if (suggestions.length === 0 && header === undefined) {
    return null;
  }
  return (
    <div className="absolute bottom-full start-0 z-20 mb-1 flex max-h-72 w-80 max-w-full flex-col overflow-y-auto rounded-md border border-line bg-surface-raised p-1 shadow-lg">
      {header}
      {suggestions.length > 0 && (
        <ul id={id} role="listbox" aria-label={label} className="flex flex-col">
          {suggestions.map((suggestion, index) => (
            <li
              key={suggestion.key}
              id={`${id}-${String(index)}`}
              role="option"
              aria-selected={index === current}
              // Read as a name and what it is, without the picture's initials between them.
              aria-label={[suggestion.label, suggestion.detail, suggestion.description]
                .filter((part) => part != null && part !== "")
                .join(", ")}
              // Picking keeps focus in the message box.
              onMouseDown={(event) => {
                event.preventDefault();
              }}
              onClick={() => {
                onPick(
                  suggestion,
                  document.querySelector<HTMLTextAreaElement>(`[aria-controls="${id}"]`),
                );
              }}
              className={
                "flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-sm " +
                (index === current ? "bg-accent-soft text-accent-strong" : "hover:bg-surface-hover")
              }
            >
              <span aria-hidden="true" className="flex shrink-0">
                {suggestion.icon}
              </span>
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate">{suggestion.label}</span>
                {suggestion.description !== undefined && (
                  <span
                    className={
                      "truncate text-xs " +
                      (index === current ? "text-accent-strong" : "text-ink-muted")
                    }
                  >
                    {suggestion.description}
                  </span>
                )}
              </span>
              {suggestion.detail !== null && (
                <span
                  className={
                    "shrink-0 truncate text-xs " +
                    (index === current ? "text-accent-strong" : "text-ink-muted")
                  }
                >
                  {suggestion.detail}
                </span>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
