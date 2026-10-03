import { ApiProblemError, type AdminListQuery } from "@aspen/protocol";
import {
  CaretDownIcon,
  CaretUpDownIcon,
  CaretUpIcon,
  MagnifyingGlassIcon,
} from "@phosphor-icons/react";
import { useEffect, useState, type ReactNode } from "react";
import {
  Button,
  Input,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
  SearchField,
  Select,
  SelectValue,
} from "react-aria-components";
import { useIdWizard } from "@/api/hooks";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { Cell, Table, type Heading } from "@/features/admin/FleetHealth";
import { useFigures } from "@/features/admin/format";
import { fieldClass, inputClass, labelClass } from "@/features/auth/styles";
import {
  optionClass,
  secondaryButtonClass,
  selectButtonClass,
  selectPopoverClass,
} from "@/features/invites/dialog";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { CopyIdButton, type IdThing } from "@/features/layout/CopyId";

/** How long typing pauses before the search is sent. */
const SEARCH_DELAY_MS = 300;
/** The page sizes offered; the first is the default. */
const PAGE_SIZES = [15, 30, 50, 100] as const;

/**
 * A column of a list: its heading, its cell, and, when it sorts, the two orders it sorts by and
 * which it tries first (names A to Z, dates and counts largest first).
 */
export interface Column<T, S extends string> {
  heading: string;
  numeric?: boolean;
  sort?: { ascending: S; descending: S; first: "ascending" | "descending" };
  cell: (item: T) => ReactNode;
}

/**
 * A searchable, sortable list, a page at a time. What the search field holds is sent once
 * typing pauses; a sortable heading sorts by its column, and again the other way; searching,
 * sorting, or a new page size goes back to the first page. Each page asks for one row more
 * than it shows, to know whether there is a next. A new `version` reads the page again where
 * it is, as after a change to its rows.
 */
export function Directory<T extends { id: string }, S extends string>({
  id,
  title,
  searchLabel,
  load,
  defaultSort,
  columns: given,
  version = 0,
  idThing,
  controls,
}: {
  id: string;
  title: string;
  searchLabel: string;
  /** More ways to narrow the list, shown beside the search; a change to them comes as a new `load`. */
  controls?: ReactNode;
  load: (query: AdminListQuery<S>) => Promise<T[]>;
  /** The order when no heading is chosen; the server's own when absent. */
  defaultSort?: S;
  columns: readonly Column<T, S>[];
  version?: number;
  /** What each row's id belongs to, for the ID wizard's column; none without it. */
  idThing?: IdThing;
}) {
  const m = useMessages();
  const wizard = useIdWizard();
  // The ID wizard's column comes last, after everything else a row holds.
  const columns: readonly Column<T, S>[] =
    wizard && idThing !== undefined
      ? [
          ...given,
          {
            heading: m.bots.idColumn,
            cell: (item) => <CopyIdButton id={item.id} thing={idThing} />,
          },
        ]
      : given;
  const { count } = useFigures();
  const [typed, setTyped] = useState("");
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<S | undefined>(defaultSort);
  const [pageSize, setPageSize] = useState<number>(PAGE_SIZES[0]);
  const [page, setPage] = useState(0);
  const [rows, setRows] = useState<{ items: readonly T[]; more: boolean } | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);
  // A new `load` narrows the list another way, which starts again from the first page.
  const [loaded, setLoaded] = useState(() => load);
  if (loaded !== load) {
    setLoaded(() => load);
    setPage(0);
  }

  // The search follows what is typed once typing pauses, from the first page. Only a change
  // to it goes back there: paging while the box holds what was already searched stays put.
  useEffect(() => {
    if (typed === search) {
      return;
    }
    const timer = setTimeout(() => {
      setLoading(true);
      setSearch(typed);
      setPage(0);
    }, SEARCH_DELAY_MS);
    return () => {
      clearTimeout(timer);
    };
  }, [typed, search]);

  useEffect(() => {
    let current = true;
    load({
      name: search,
      ...(sort === undefined ? {} : { sort }),
      offset: page * pageSize,
      limit: pageSize + 1,
    }).then(
      (found) => {
        if (current) {
          setRows({ items: found.slice(0, pageSize), more: found.length > pageSize });
          setError(null);
          setLoading(false);
        }
      },
      (e: unknown) => {
        if (current) {
          setError(e instanceof ApiProblemError ? e.message : String(e));
          setLoading(false);
        }
      },
    );
    return () => {
      current = false;
    };
  }, [load, search, sort, page, pageSize, attempt, version]);

  /** Moves to another page or order; the rows shown stay, faded, until it arrives. */
  function go(change: () => void) {
    setLoading(true);
    change();
  }

  const headings: Heading[] = columns.map((column) => {
    const order = column.sort;
    if (order === undefined) {
      return { content: column.heading };
    }
    const state =
      sort === order.ascending ? "ascending" : sort === order.descending ? "descending" : "none";
    const Icon =
      state === "ascending"
        ? CaretUpIcon
        : state === "descending"
          ? CaretDownIcon
          : CaretUpDownIcon;
    return {
      sort: state,
      content: (
        <Button
          onPress={() => {
            go(() => {
              setSort(
                state === "none"
                  ? order[order.first]
                  : state === "ascending"
                    ? order.descending
                    : order.ascending,
              );
              setPage(0);
            });
          }}
          className={
            "tap-target inline-flex items-center gap-1 rounded outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50" +
            (state === "none" ? "" : " text-ink")
          }
        >
          {column.heading}
          <Icon size={12} weight="bold" aria-hidden="true" />
        </Button>
      ),
    };
  });
  const numeric = columns.flatMap((column, i) => (column.numeric === true ? [i] : []));
  const first = page * pageSize + 1;
  const shownCount = rows?.items.length ?? 0;

  return (
    <Section id={id} title={title}>
      <div className="flex flex-wrap items-end gap-3">
        <SearchField value={typed} onChange={setTyped} className={fieldClass + " w-full max-w-sm"}>
          <Label className={labelClass}>{searchLabel}</Label>
          <div className="relative">
            <MagnifyingGlassIcon
              size={16}
              aria-hidden="true"
              className="pointer-events-none absolute top-1/2 start-3 -translate-y-1/2 text-ink-muted"
            />
            <Input className={inputClass + " w-full ps-9"} />
          </div>
        </SearchField>
        {controls}
      </div>
      {error !== null && (
        <ReadFailed
          error={error}
          onRetry={() => {
            go(() => {
              setAttempt((n) => n + 1);
            });
          }}
        />
      )}
      {rows !== null && rows.items.length === 0 && page === 0 ? (
        <p className="text-sm text-ink-muted">{m.admin.noMatches}</p>
      ) : (
        <Table
          label={title}
          headings={headings}
          numeric={numeric}
          dimmed={loading && rows !== null}
          skeletonRows={rows === null && error === null ? 5 : 0}
        >
          {(rows?.items ?? []).map((item) => (
            <tr key={item.id}>
              {columns.map((column, i) => (
                <Cell key={i} numeric={column.numeric === true}>
                  {column.cell(item)}
                </Cell>
              ))}
            </tr>
          ))}
        </Table>
      )}
      <div className="flex flex-wrap items-center gap-3 text-sm">
        <Select
          value={pageSize}
          onChange={(key) => {
            go(() => {
              setPageSize(Number(key));
              setPage(0);
            });
          }}
          className="flex items-center gap-2"
        >
          <Label className="whitespace-nowrap text-ink-muted">{m.admin.rowsPerPage}</Label>
          <Button className={selectButtonClass + " w-20 py-1"}>
            <SelectValue />
            <CaretDownIcon size={14} aria-hidden="true" className="text-ink-muted" />
          </Button>
          <Popover className={selectPopoverClass}>
            <ListBox>
              {PAGE_SIZES.map((size) => (
                <ListBoxItem key={size} id={size} className={optionClass}>
                  {String(size)}
                </ListBoxItem>
              ))}
            </ListBox>
          </Popover>
        </Select>
        {shownCount > 0 && (
          <span className="text-ink-muted tabular-nums">
            {format(m.admin.rowsShown, {
              first: count(first),
              last: count(first + shownCount - 1),
            })}
          </span>
        )}
        <span className="ms-auto flex gap-2">
          <Button
            isDisabled={page === 0 || loading}
            onPress={() => {
              go(() => {
                setPage((n) => Math.max(n - 1, 0));
              });
            }}
            className={secondaryButtonClass}
          >
            {m.admin.previousPage}
          </Button>
          <Button
            isDisabled={rows?.more !== true || loading}
            onPress={() => {
              go(() => {
                setPage((n) => n + 1);
              });
            }}
            className={secondaryButtonClass}
          >
            {m.admin.nextPage}
          </Button>
        </span>
      </div>
    </Section>
  );
}
