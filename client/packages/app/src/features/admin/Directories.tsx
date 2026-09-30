import {
  ApiProblemError,
  type Channel,
  type AdminCommunityEntry,
  type AdminListQuery,
  type AdminUserEntry,
  type CommunitySort,
  type UserSort,
} from "@aspen/protocol";
import {
  CaretDownIcon,
  CaretUpDownIcon,
  CaretUpIcon,
  CheckIcon,
  MagnifyingGlassIcon,
  PencilSimpleIcon,
} from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import {
  Button,
  CheckboxButton,
  CheckboxField,
  Dialog,
  DialogTrigger,
  Input,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
  SearchField,
  Select,
  SelectValue,
} from "react-aria-components";
import { useDeploymentCan, useMe, useSync, useIdWizard } from "@/api/hooks";
import { rankOf, type DeploymentRoles } from "@/features/admin/deploymentRoles";
import { useDmTitle } from "@/features/dms/useDmTitle";
import { markClass } from "@/features/layout/choices";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { Cell, Table, type Heading } from "@/features/admin/FleetHealth";
import { useFigures } from "@/features/admin/format";
import { fieldClass, inputClass, labelClass } from "@/features/auth/styles";
import { Avatar } from "@/features/communities/Avatar";
import {
  dangerButtonClass,
  optionClass,
  secondaryButtonClass,
  selectButtonClass,
  selectPopoverClass,
} from "@/features/invites/dialog";
import { BotBadge } from "@/features/users/BotBadge";
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
 * The deployment's users, searched by username or display name and sortable, with the
 * deployment roles each holds, and bots marked. Those who may manage deployment roles change
 * them here, moderators open someone's DMs, and those who may manage bots delete a bot whose
 * owner is gone.
 */
export function UserDirectory({ roles }: { roles: DeploymentRoles | undefined }) {
  const m = useMessages();
  const { day } = useFigures();
  const sync = useSync();
  const manage = useDeploymentCan("manageDeploymentRoles");
  const moderator = useDeploymentCan("moderateCommunities");
  const manageBots = useDeploymentCan("manageBots");
  const load = useCallback((query: AdminListQuery<UserSort>) => sync.adminUsers(query), [sync]);
  return (
    <Directory<AdminUserEntry, UserSort>
      idThing="user"
      id="admin-users"
      title={m.admin.users}
      searchLabel={m.admin.searchUsers}
      load={load}
      defaultSort="-createdAt"
      columns={[
        {
          heading: m.admin.name,
          sort: { ascending: "name", descending: "-name", first: "ascending" },
          cell: (user) => (
            <span className="flex items-center gap-2">
              <Avatar name={user.displayName ?? user.name} iconId={user.icon} size="sm" />
              <span className="min-w-0">
                <span className="flex items-center gap-1.5">
                  <span className="truncate font-medium">{user.displayName ?? user.name}</span>
                  {user.bot && <BotBadge />}
                </span>
                <span className="block truncate text-xs text-ink-muted">
                  {user.homeDomain == null ? user.name : `${user.name}@${user.homeDomain}`}
                </span>
              </span>
            </span>
          ),
        },
        {
          heading: m.admin.joined,
          sort: { ascending: "createdAt", descending: "-createdAt", first: "descending" },
          cell: (user) => day(user.createdAt),
        },
        {
          heading: m.admin.invite,
          cell: (user) => <code className="font-mono text-xs">{user.registeredWith ?? ""}</code>,
        },
        {
          heading: m.admin.rolesColumn,
          cell: (user) => <UserRoles user={user} roles={roles} manage={manage} />,
        },
        ...(moderator || manageBots
          ? [
              {
                heading: m.admin.actions,
                cell: (user: AdminUserEntry) => (
                  <span className="flex flex-wrap items-center gap-2">
                    {moderator && <UserDms user={user} />}
                    {moderator && user.homeDomain != null && <BanForeignUser user={user} />}
                    {manageBots && user.bot && user.botOwner == null && (
                      <DeleteOwnerlessBot user={user} />
                    )}
                  </span>
                ),
              },
            ]
          : []),
      ]}
    />
  );
}

/**
 * Bans a user of another server from this one, after a confirming second press, or lifts the
 * ban: a banned user's sessions here end and they cannot sign in here again.
 */
function BanForeignUser({ user }: { user: AdminUserEntry }) {
  const m = useMessages();
  const sync = useSync();
  const [banned, setBanned] = useState(user.banned);
  const [confirming, setConfirming] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const name = user.displayName ?? user.name;
  const change = (next: boolean) => {
    setPending(true);
    setError(null);
    sync.setForeignUserBanned(user.id, next).then(
      () => {
        setBanned(next);
        setConfirming(false);
        setPending(false);
      },
      (e: unknown) => {
        setError(e instanceof ApiProblemError ? e.message : String(e));
        setPending(false);
      },
    );
  };
  return (
    <span className="flex flex-col gap-1">
      {banned ? (
        <>
          <span className="text-xs text-danger">{m.deployments.banned}</span>
          <Button
            isDisabled={pending}
            aria-label={format(m.deployments.liftBanLabel, { name })}
            onPress={() => {
              change(false);
            }}
            className={secondaryButtonClass + " self-start"}
          >
            {m.deployments.liftBan}
          </Button>
        </>
      ) : (
        <Button
          isDisabled={pending}
          aria-label={format(m.deployments.banLabel, { name })}
          onPress={() => {
            if (confirming) {
              change(true);
            } else {
              setConfirming(true);
            }
          }}
          className={
            (confirming ? dangerButtonClass : secondaryButtonClass + " text-danger") + " self-start"
          }
        >
          {confirming ? format(m.deployments.banConfirm, { name }) : m.deployments.ban}
        </Button>
      )}
      {error !== null && (
        <span role="alert" className="text-xs text-danger">
          {error}
        </span>
      )}
    </span>
  );
}

/** Deletes a bot whose owner deleted their account, after a confirming second press. */
function DeleteOwnerlessBot({ user }: { user: AdminUserEntry }) {
  const m = useMessages();
  const sync = useSync();
  const [confirming, setConfirming] = useState(false);
  const [state, setState] = useState<"idle" | "pending" | "deleted">("idle");
  const [error, setError] = useState<string | null>(null);
  if (state === "deleted") {
    return <span className="text-xs text-ink-muted">{m.bots.deleted}</span>;
  }
  return (
    <span className="flex flex-col gap-1">
      <span className="text-xs text-ink-muted">{m.bots.ownerGone}</span>
      <Button
        onPress={() => {
          if (!confirming) {
            setConfirming(true);
            return;
          }
          if (state === "pending") {
            return;
          }
          setState("pending");
          setError(null);
          sync.deleteBot(user.id).then(
            () => {
              setState("deleted");
            },
            (e: unknown) => {
              setError(e instanceof ApiProblemError ? e.message : String(e));
              setState("idle");
            },
          );
        }}
        className={
          (confirming ? dangerButtonClass : secondaryButtonClass + " text-danger") + " self-start"
        }
      >
        {confirming
          ? format(m.bots.deleteNamed, { name: user.displayName ?? user.name })
          : m.bots.delete}
      </Button>
      {error !== null && (
        <span role="alert" className="text-xs text-danger">
          {error}
        </span>
      )}
    </span>
  );
}

/**
 * The deployment roles someone holds, and, for those who may manage deployment roles, a picker
 * of the roles below the caller's highest to give or take.
 */
function UserRoles({
  user,
  roles,
  manage,
}: {
  user: AdminUserEntry;
  roles: DeploymentRoles | undefined;
  manage: boolean;
}) {
  const m = useMessages();
  const sync = useSync();
  const [held, setHeld] = useState<readonly string[]>(user.roles);
  const [error, setError] = useState<string | null>(null);
  const all = roles?.roles ?? [];
  const rank = roles === undefined ? 0 : rankOf(roles);
  const theirRank = Math.max(0, ...all.filter((r) => held.includes(r.id)).map((r) => r.position));
  const me = useMe();
  // Anyone may change their own roles below their highest; others must rank below them.
  const mayChange = manage && (me?.id === user.id || theirRank < rank);
  const name = user.displayName ?? user.name;
  const label = format(m.admin.deploymentRoleOf, { name });
  return (
    <span className="flex flex-wrap items-center gap-1">
      {all
        .filter((r) => held.includes(r.id))
        .reverse()
        .map((r) => (
          <span key={r.id} className="rounded-full border border-line px-2 py-0.5 text-xs">
            {r.name}
          </span>
        ))}
      {mayChange && (
        <DialogTrigger>
          <Button aria-label={label} className={secondaryButtonClass + " py-0.5 text-xs"}>
            <PencilSimpleIcon size={12} aria-hidden="true" />
          </Button>
          <Popover
            placement="bottom end"
            className="w-64 rounded-md border border-line bg-surface-raised p-2 shadow-lg"
          >
            <Dialog aria-label={label} className="flex flex-col gap-1 outline-none">
              {[...all].reverse().map((role) => (
                <CheckboxField
                  key={role.id}
                  isSelected={held.includes(role.id)}
                  isDisabled={role.position >= rank}
                  onChange={(selected) => {
                    setError(null);
                    sync.setUserDeploymentRole(user.id, role.id, selected).then(
                      () => {
                        setHeld((now) =>
                          selected ? [...now, role.id] : now.filter((id) => id !== role.id),
                        );
                      },
                      (e: unknown) => {
                        setError(e instanceof ApiProblemError ? e.message : String(e));
                      },
                    );
                  }}
                >
                  <CheckboxButton className="group flex items-center gap-2 rounded px-2 py-1 text-sm outline-none hover:bg-surface-hover disabled:opacity-50 focus-visible:ring-2 focus-visible:ring-accent/50">
                    <span className={markClass + " mt-0"}>
                      <CheckIcon
                        size={12}
                        weight="bold"
                        aria-hidden="true"
                        className="hidden group-selected:block"
                      />
                    </span>
                    <span className="truncate">{role.name}</span>
                  </CheckboxButton>
                </CheckboxField>
              ))}
              {error !== null && (
                <p role="alert" className="text-xs text-danger">
                  {error}
                </p>
              )}
            </Dialog>
          </Popover>
        </DialogTrigger>
      )}
    </span>
  );
}

/** For a moderator: someone's DMs, each a link to read it, which the server logs. */
function UserDms({ user }: { user: AdminUserEntry }) {
  const m = useMessages();
  const sync = useSync();
  const [dms, setDms] = useState<readonly Channel[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const name = user.displayName ?? user.name;
  return (
    <DialogTrigger
      onOpenChange={(open) => {
        if (open) {
          setError(null);
          sync.userDms(user.id).then(setDms, (e: unknown) => {
            setError(e instanceof ApiProblemError ? e.message : String(e));
          });
        }
      }}
    >
      <Button className={secondaryButtonClass + " py-0.5 text-xs"}>{m.admin.userDms}</Button>
      <Popover
        placement="bottom end"
        className="max-h-80 w-72 overflow-y-auto rounded-md border border-line bg-surface-raised p-2 shadow-lg"
      >
        <Dialog
          aria-label={format(m.admin.userDmsHeading, { name })}
          className="flex flex-col gap-1 outline-none"
        >
          <h3 className="px-1 text-sm font-semibold">{format(m.admin.userDmsHeading, { name })}</h3>
          <p className="px-1 text-xs text-ink-muted">{m.admin.userDmsHint}</p>
          {error !== null && <p className="px-1 text-xs text-danger">{error}</p>}
          {dms !== null && dms.length === 0 && (
            <p className="px-1 text-sm text-ink-muted">{m.admin.noDms}</p>
          )}
          {dms?.map((dm) => (
            <DmLink key={dm.id} dm={dm} />
          ))}
        </Dialog>
      </Popover>
    </DialogTrigger>
  );
}

function DmLink({ dm }: { dm: Channel }) {
  const title = useDmTitle(dm);
  return (
    <Link
      to="/dms/$channelId"
      params={{ channelId: dm.id }}
      className="truncate rounded px-2 py-1 text-sm outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      {title}
    </Link>
  );
}

/**
 * The deployment's communities, searched by name and sortable. A moderator opens any of them.
 */
export function CommunityDirectory() {
  const m = useMessages();
  const { count, day } = useFigures();
  const sync = useSync();
  const moderator = useDeploymentCan("moderateCommunities");
  const load = useCallback(
    (query: AdminListQuery<CommunitySort>) => sync.adminCommunities(query),
    [sync],
  );
  return (
    <Directory<AdminCommunityEntry, CommunitySort>
      idThing="community"
      id="admin-communities"
      title={m.admin.communities}
      searchLabel={m.admin.searchCommunities}
      load={load}
      defaultSort="-createdAt"
      columns={[
        {
          heading: m.admin.name,
          sort: { ascending: "name", descending: "-name", first: "ascending" },
          cell: (community) => (
            <span className="flex items-center gap-2">
              <Avatar name={community.name} iconId={community.icon} size="sm" />
              <span className="truncate font-medium">{community.name}</span>
            </span>
          ),
        },
        {
          heading: m.admin.members,
          numeric: true,
          sort: { ascending: "members", descending: "-members", first: "descending" },
          cell: (community) => count(community.members),
        },
        {
          heading: m.admin.created,
          sort: { ascending: "createdAt", descending: "-createdAt", first: "descending" },
          cell: (community) => day(community.createdAt),
        },
        ...(moderator
          ? [
              {
                heading: m.admin.actions,
                cell: (community: AdminCommunityEntry) => (
                  <Link
                    to="/communities/$communityId"
                    params={{ communityId: community.id }}
                    aria-label={format(m.admin.openCommunityLabel, { community: community.name })}
                    className={secondaryButtonClass + " inline-block py-0.5 text-xs"}
                  >
                    {m.admin.openCommunity}
                  </Link>
                ),
              },
            ]
          : []),
      ]}
    />
  );
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
}: {
  id: string;
  title: string;
  searchLabel: string;
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
      <SearchField value={typed} onChange={setTyped} className={fieldClass + " max-w-sm"}>
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
