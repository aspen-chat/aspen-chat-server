import {
  ApiProblemError,
  SEARCH_PAGE,
  type Message,
  type MessageHolding,
  type MessageSearch,
  type User,
} from "@aspen/protocol";
import { MagnifyingGlassIcon } from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import { useContext, useMemo, useState, type SyntheticEvent } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Form,
  Input,
  Label,
  Modal,
  ModalOverlay,
  Text,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
} from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { SourceScope } from "@/api/deployments";
import { ScopeDomainContext } from "@/api/deploymentsContext";
import { useSources, type Source } from "@/api/everywhere";
import { useChannel, useCommunity, useSync, useUser } from "@/api/hooks";
import {
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { MemberPicker } from "@/features/community-settings/MemberPicker";
import { listModalClass, overlayClass, dialogClass } from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { Tooltip } from "@/features/layout/Tooltip";
import { decodeTags } from "@/features/mentions/tags";
import { messageLink, threadLink, type ChannelHome } from "@/features/messages/links";
import { mergeResults, type SourceResults } from "@/features/search/merge";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";
import { PersonName } from "@/features/users/PersonName";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";

/** Where a search looks: the channel it was opened from, its community, its server, or all. */
type Scope = "channel" | "community" | "server" | "everywhere";

const HOLDINGS: readonly MessageHolding[] = ["attachment", "image", "poll"];

const chipClass =
  "rounded-md border border-line px-3 py-1 text-sm text-ink-muted outline-none hover:bg-surface-hover " +
  "pressed:bg-surface-hover selected:border-accent selected:bg-accent-soft selected:text-accent-strong " +
  "focus-visible:ring-2 focus-visible:ring-accent/50";

const TIME: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

/**
 * The header's search control and the search it opens, of messages the user may read: in the
 * channel or DM it was opened from, its community, this server, or every server they use.
 */
export function SearchButton({
  channelId,
  channelName,
  communityId,
}: {
  channelId: string;
  channelName: string;
  /** The channel's community; `null` for a DM. */
  communityId: string | null;
}) {
  const m = useMessages();
  return (
    <DialogTrigger>
      <Tooltip text={m.search.open}>
        <Button
          aria-label={m.search.open}
          className="rounded-md p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          <MagnifyingGlassIcon size={20} aria-hidden="true" />
        </Button>
      </Tooltip>
      <ModalOverlay isDismissable className={overlayClass}>
        <Modal className={listModalClass}>
          <Dialog className={dialogClass}>
            {({ close }) => (
              <>
                <DialogHeading>{m.search.heading}</DialogHeading>
                <SearchPanel
                  channelId={channelId}
                  channelName={channelName}
                  communityId={communityId}
                  onJump={close}
                />
              </>
            )}
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

/** One deployment's results so far, and what searching it said when it failed. */
interface Searched extends SourceResults<Message> {
  readonly source: Source;
  readonly problem: string | null;
}

function SearchPanel({
  channelId,
  channelName,
  communityId,
  onJump,
}: {
  channelId: string;
  channelName: string;
  communityId: string | null;
  onJump: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const client = useAspenClient();
  const domain = useContext(ScopeDomainContext);
  const sources = useSources();
  const community = useCommunity(communityId ?? "");
  const here: Source = useMemo(() => ({ domain, client, sync }), [domain, client, sync]);
  const [text, setText] = useState("");
  const [scope, setScope] = useState<Scope>(communityId === null ? "channel" : "community");
  const [has, setHas] = useState<ReadonlySet<MessageHolding>>(new Set());
  const [author, setAuthor] = useState<User | null>(null);
  const [taggingMe, setTaggingMe] = useState(false);
  const [searched, setSearched] = useState<readonly Searched[] | null>(null);
  const [pending, setPending] = useState(false);
  // Who wrote a message is asked only within a community, where there are members to pick.
  const byAuthor = communityId !== null && (scope === "channel" || scope === "community");
  const ready = text.trim() !== "" || has.size > 0 || taggingMe || (byAuthor && author !== null);

  const scopes: { id: Scope; label: string }[] = [
    {
      id: "channel",
      label:
        communityId === null
          ? m.search.scopeConversation
          : format(m.search.scopeChannel, { channel: channelName }),
    },
    ...(communityId === null
      ? []
      : [
          {
            id: "community" as const,
            label: format(m.search.scopeCommunity, { community: community?.name ?? "" }),
          },
        ]),
    { id: "server", label: m.search.scopeServer },
    ...(sources.length > 1 ? [{ id: "everywhere" as const, label: m.search.scopeEverywhere }] : []),
  ];

  /** The search as `source` is asked it: the tagged user is its own account there. */
  function queryFor(source: Source, before: string | undefined): MessageSearch {
    const me = source.client.session?.userId;
    const trimmed = text.trim();
    return {
      ...(trimmed === "" ? {} : { text: trimmed }),
      ...(has.size === 0 ? {} : { has: [...has] }),
      ...(byAuthor && author !== null ? { author: author.id } : {}),
      ...(taggingMe && me !== undefined ? { mentions: me } : {}),
      ...(scope === "channel" ? { channel: channelId } : {}),
      ...(scope === "community" && communityId !== null ? { community: communityId } : {}),
      ...(before === undefined ? {} : { before }),
    };
  }

  async function page(source: Source, before: string | undefined, earlier: readonly Message[]) {
    const key = source.domain ?? "";
    try {
      const found = await source.sync.searchMessages(queryFor(source, before));
      return {
        key,
        source,
        messages: [...earlier, ...found],
        exhausted: found.length < SEARCH_PAGE,
        problem: null,
      };
    } catch (e) {
      return {
        key,
        source,
        messages: earlier,
        exhausted: true,
        problem: e instanceof ApiProblemError ? e.message : String(e),
      };
    }
  }

  async function search(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!ready) {
      return;
    }
    setPending(true);
    const asked = scope === "everywhere" ? sources : [here];
    setSearched(await Promise.all(asked.map((source) => page(source, undefined, []))));
    setPending(false);
  }

  async function more() {
    if (searched === null) {
      return;
    }
    setPending(true);
    setSearched(
      await Promise.all(
        searched.map((s) =>
          s.exhausted ? Promise.resolve(s) : page(s.source, s.messages.at(-1)?.id, s.messages),
        ),
      ),
    );
    setPending(false);
  }

  const results = searched === null ? [] : mergeResults(searched);
  const bySource = new Map((searched ?? []).map((s) => [s.key, s.source]));
  const failures = (searched ?? []).filter((s) => s.problem !== null);

  return (
    <div className="flex flex-col gap-4">
      <Form
        onSubmit={(event) => {
          void search(event);
        }}
        className="flex flex-col gap-3"
      >
        <TextField value={text} onChange={setText} className={fieldClass}>
          <Label className={labelClass}>{m.search.text}</Label>
          <Input type="search" autoFocus className={inputClass} />
          <Text slot="description" className="text-xs text-ink-muted">
            {m.search.textHint}
          </Text>
        </TextField>
        <div className="flex flex-col gap-1">
          <span id="search-scope" className={labelClass}>
            {m.search.scope}
          </span>
          <ToggleButtonGroup
            aria-labelledby="search-scope"
            selectionMode="single"
            disallowEmptySelection
            selectedKeys={[scope]}
            onSelectionChange={(keys) => {
              const [next] = Array.from(keys);
              if (next !== undefined) {
                setScope(next as Scope);
              }
            }}
            className="flex flex-wrap gap-1"
          >
            {scopes.map((s) => (
              <ToggleButton key={s.id} id={s.id} className={chipClass}>
                {s.label}
              </ToggleButton>
            ))}
          </ToggleButtonGroup>
        </div>
        <div className="flex flex-col gap-1">
          <span id="search-has" className={labelClass}>
            {m.search.has}
          </span>
          <div className="flex flex-wrap gap-1">
            <ToggleButtonGroup
              aria-labelledby="search-has"
              selectionMode="multiple"
              selectedKeys={has}
              onSelectionChange={(keys) => {
                setHas(new Set(Array.from(keys) as MessageHolding[]));
              }}
              className="flex flex-wrap gap-1"
            >
              {HOLDINGS.map((h) => (
                <ToggleButton key={h} id={h} className={chipClass}>
                  {h === "attachment"
                    ? m.search.hasAttachment
                    : h === "image"
                      ? m.search.hasImage
                      : m.search.hasPoll}
                </ToggleButton>
              ))}
            </ToggleButtonGroup>
            <ToggleButton isSelected={taggingMe} onChange={setTaggingMe} className={chipClass}>
              {m.search.taggingMe}
            </ToggleButton>
          </div>
        </div>
        {byAuthor && (
          <MemberPicker communityId={communityId} label={m.search.from} onChange={setAuthor} />
        )}
        <Button type="submit" isDisabled={!ready || pending} className={primaryButtonClass}>
          {pending ? m.search.searching : m.search.submit}
        </Button>
      </Form>
      {failures.map((s) => (
        <p
          key={s.key}
          role="alert"
          className="rounded-md bg-danger-soft px-3 py-2 text-sm text-danger"
        >
          {format(m.search.failedOn, {
            domain: s.source.domain ?? m.search.scopeServer,
            problem: s.problem ?? "",
          })}
        </p>
      ))}
      {pending && searched === null && <ResultsSkeleton count={4} />}
      {searched !== null && (
        <section aria-label={m.search.results} className="flex flex-col gap-2">
          {results.length === 0 && failures.length === 0 ? (
            <p className={hintClass}>{m.search.none}</p>
          ) : (
            <ul className="flex flex-col gap-1">
              {results.map(({ key, message }) => {
                const source = bySource.get(key);
                return source === undefined ? null : (
                  <SourceScope key={`${key}/${message.id}`} source={source}>
                    <SearchResult message={message} domain={source.domain} onJump={onJump} />
                  </SourceScope>
                );
              })}
            </ul>
          )}
          {pending && <ResultsSkeleton count={2} />}
          {searched.some((s) => !s.exhausted) && (
            <Button
              onPress={() => {
                void more();
              }}
              isDisabled={pending}
              className="self-center rounded-md px-3 py-1.5 text-sm text-accent outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
            >
              {m.search.more}
            </Button>
          )}
        </section>
      )}
    </div>
  );
}

/**
 * One message found: who wrote it and when, where, and its text with its tags as names,
 * linking to it in place (in its thread, for a reply). Run in its deployment's scope.
 */
function SearchResult({
  message,
  domain,
  onJump,
}: {
  message: Message;
  domain: string | null;
  onJump: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const time = useDateFormat(TIME);
  const author = useUser(message.author);
  const channel = useChannel(message.channelId);
  const parent = useChannel(channel?.parentChannel ?? "");
  const place = parent ?? channel;
  const community = useCommunity(place?.community ?? "");
  const home: ChannelHome = { domain, community: place?.community ?? null };
  const link =
    channel?.parentChannel != null
      ? threadLink(home, channel.parentChannel, channel.id)
      : messageLink(home, message.channelId, message.id);
  const where =
    place === undefined
      ? ""
      : place.ty === "dm"
        ? m.search.inDm
        : place.ty === "groupDm"
          ? m.search.inGroupDm
          : channel?.parentChannel != null
            ? format(m.search.inThread, { thread: channel.name, channel: place.name })
            : format(m.search.inChannel, { channel: place.name, community: community?.name ?? "" });
  const roles = place?.community == null ? [] : sync.store.roles(place.community);
  const { text } = decodeTags(
    message.content,
    (id) => sync.store.user(id)?.name,
    (id) => roles.find((role) => role.id === id)?.name,
  );
  return (
    <li>
      <Link
        {...link}
        onClick={onJump}
        aria-label={`${m.search.jump}: ${author === undefined ? m.unknownUser : displayNameOf(author)}, ${where}`}
        className="flex flex-col gap-0.5 rounded-md px-2 py-1.5 text-sm outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
      >
        <span className="flex flex-wrap items-baseline gap-x-2">
          <span className="font-medium">
            <PersonName id={message.author} />
          </span>
          <span className="text-xs text-ink-faint">{time.format(new Date(message.timestamp))}</span>
        </span>
        <span className="text-xs text-ink-muted">
          {where}
          {domain !== null && ` · ${format(m.search.onDomain, { domain })}`}
        </span>
        {text !== "" && <span className="line-clamp-3 whitespace-pre-wrap">{text}</span>}
        {message.kind === "poll" && <span className="text-ink-muted">{m.search.poll}</span>}
        {message.attachments.length > 0 && (
          <span className="text-xs text-ink-muted">
            {format(m.search.attachments, { count: String(message.attachments.length) })}
          </span>
        )}
      </Link>
    </li>
  );
}

/** Results on their way, shaped like `SearchResult`: who and when, where, and a line or two. */
function ResultsSkeleton({ count }: { count: number }) {
  return (
    <div aria-busy="true" className="flex flex-col gap-1">
      <LoadingLabel />
      {Array.from({ length: count }, (_, index) => (
        <div key={index} className="flex flex-col gap-1.5 px-2 py-1.5">
          <div className="flex items-center gap-2">
            <Skeleton className={"h-3.5 " + (index % 2 === 0 ? "w-24" : "w-16")} />
            <Skeleton className="h-3 w-14" />
          </div>
          <Skeleton className="h-3 w-32" />
          <Skeleton className={"h-3.5 " + (index % 2 === 0 ? "w-5/6" : "w-2/3")} />
        </div>
      ))}
    </div>
  );
}
