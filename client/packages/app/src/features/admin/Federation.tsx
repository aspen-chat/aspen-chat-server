import {
  ApiProblemError,
  type FederatedDeployment,
  type FederationList,
  type FederationOverview,
  type Gate,
} from "@aspen/protocol";
import { CopyIcon, KeyIcon, WarningIcon } from "@phosphor-icons/react";
import { useCallback, useRef, useState, type ReactNode } from "react";
import {
  Button,
  Dialog,
  Form,
  Input,
  Label,
  Modal,
  ModalOverlay,
  TextField,
} from "react-aria-components";
import { useSync } from "@/api/hooks";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { Directory, type Column } from "@/features/admin/Directories";
import { Cell, Status, Table } from "@/features/admin/FleetHealth";
import { moment } from "@/features/admin/format";
import { useAdminRead, type AdminRead } from "@/features/admin/useAdminRead";
import {
  alertClass,
  fieldClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import {
  dangerButtonClass,
  dialogClass,
  modalClass,
  overlayClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { copyText } from "@/features/layout/clipboard";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { format, type Messages } from "@/i18n/messages";

/** The longest note kept on a deployment, as the server's `MAX_NOTE_CHARS`. */
const MAX_NOTE_CHARS = 200;

const smallButtonClass =
  "tap-target inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs text-ink-muted outline-none " +
  "hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50";

/** A deployment as a directory row, which keys its rows by `id`. */
type Row = FederatedDeployment & { id: string };

function problemText(e: unknown): string {
  return e instanceof ApiProblemError ? e.message : String(e);
}

/**
 * Federation, for holders of Manage federation: this deployment's domain, key, and gates as
 * aspen.toml sets them, adding other deployments (each contacted at once, pinning its key), and
 * the directory of those known, where each is checked again, put on and taken off the lists
 * the gates read, has a changed key reviewed and accepted, or is forgotten.
 */
export function FederationSection() {
  const m = useMessages();
  const sync = useSync();
  const loadOverview = useCallback(() => sync.federation(), [sync]);
  const overview = useAdminRead(loadOverview);
  const [version, setVersion] = useState(0);
  const changed = useCallback(() => {
    setVersion((n) => n + 1);
  }, []);
  const load = useCallback(
    async (query: { name?: string; offset?: number; limit?: number }) =>
      (await sync.federatedDeployments(query)).map((d): Row => ({ ...d, id: d.domain })),
    [sync],
  );
  const listsInForce = overview.data?.listsInForce ?? [];
  const columns: Column<Row, never>[] = [
    {
      heading: m.federation.domain,
      cell: (d) => (
        <span className="flex flex-col">
          <span className="font-medium break-words">{d.domain}</span>
          {d.note != null && (
            <span className="line-clamp-2 text-xs break-words text-ink-muted">{d.note}</span>
          )}
          <span className="text-xs text-ink-muted">
            {m.federation.origin[d.origin]} · {moment(d.createdAt)}
          </span>
        </span>
      ),
    },
    {
      heading: m.federation.key,
      cell: (d) => (
        <span className="flex flex-col gap-1">
          {d.publicKeyFingerprint == null ? (
            <span className="text-ink-muted">{m.federation.notContacted}</span>
          ) : (
            <code className="font-mono text-xs break-all">{d.publicKeyFingerprint}</code>
          )}
          {d.offeredKey != null && (
            <Status icon={WarningIcon} tone="text-danger" label={m.federation.newKey} />
          )}
        </span>
      ),
    },
    {
      heading: m.federation.lastContact,
      cell: (d) => (d.lastContactAt == null ? "" : moment(d.lastContactAt)),
    },
    {
      heading: m.federation.admits,
      cell: (d) => <Admits deployment={d} />,
    },
    {
      heading: m.admin.actions,
      cell: (d) => <Actions deployment={d} listsInForce={listsInForce} onChanged={changed} />,
    },
  ];
  return (
    <>
      <Section id="admin-federation" title={m.federation.title} hint={m.federation.hint}>
        <Identity read={overview} />
        {overview.data?.domain != null && <AddDeployment onAdded={changed} />}
      </Section>
      <Directory<Row, never>
        id="admin-federation-deployments"
        title={m.federation.known}
        searchLabel={m.federation.search}
        load={load}
        columns={columns}
        version={version}
      />
    </>
  );
}

/** This deployment's domain, key, and gates. */
function Identity({ read }: { read: AdminRead<FederationOverview> }) {
  const m = useMessages();
  if (read.error !== null) {
    return <ReadFailed error={read.error} onRetry={read.reload} />;
  }
  const overview = read.data;
  if (overview === undefined) {
    return null;
  }
  if (overview.domain == null) {
    return <p className="text-sm text-ink-muted">{m.federation.notFederating}</p>;
  }
  const gate = (value: Gate) => m.federation.gate[value];
  return (
    <div className="flex flex-col gap-3 rounded-lg border border-line bg-surface-raised p-4">
      <dl className="grid grid-cols-[auto_1fr] items-baseline gap-x-4 gap-y-2 text-sm">
        <dt className="text-ink-muted">{m.federation.domain}</dt>
        <dd className="font-medium break-all">{overview.domain}</dd>
        <dt className="text-ink-muted">{m.federation.key}</dt>
        <dd className="flex flex-wrap items-center gap-2">
          {overview.keyFingerprint == null ? (
            <span className="text-ink-muted">{m.federation.keyPending}</span>
          ) : (
            <>
              <code className="font-mono text-xs break-all">{overview.keyFingerprint}</code>
              <CopyFingerprint fingerprint={overview.keyFingerprint} />
            </>
          )}
        </dd>
      </dl>
      <Table
        label={m.federation.gates}
        headings={[m.federation.who, m.federation.going, m.federation.coming]}
      >
        {(
          [
            [m.federation.users, overview.users, overview.usersSharedList],
            [m.federation.bots, overview.bots, overview.botsSharedList],
          ] as const
        ).map(([who, gates, shared]) => (
          <tr key={who}>
            <Cell>
              <span className="font-medium">{who}</span>
              {shared && (
                <span className="block text-xs text-ink-muted">{m.federation.sharedList}</span>
              )}
            </Cell>
            <Cell>{gate(gates.emigration)}</Cell>
            <Cell>{gate(gates.immigration)}</Cell>
          </tr>
        ))}
      </Table>
    </div>
  );
}

function CopyFingerprint({ fingerprint }: { fingerprint: string }) {
  const m = useMessages();
  const [copied, setCopied] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  return (
    <Button
      ref={button}
      onPress={() => {
        if (button.current !== null) {
          void copyText(fingerprint, button.current).then(setCopied);
        }
      }}
      className={smallButtonClass}
    >
      <CopyIcon size={14} aria-hidden="true" />
      {copied ? m.federation.copied : m.federation.copyKey}
    </Button>
  );
}

/** Adds a deployment and contacts it at once, saying what came of it. */
function AddDeployment({ onAdded }: { onAdded: () => void }) {
  const m = useMessages();
  const sync = useSync();
  const [domain, setDomain] = useState("");
  const [note, setNote] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);

  async function add() {
    setPending(true);
    setError(null);
    setResult(null);
    let added: FederatedDeployment;
    try {
      added = await sync.addFederatedDeployment(domain, note);
    } catch (e) {
      setError(problemText(e));
      setPending(false);
      return;
    }
    setDomain("");
    setNote("");
    try {
      const contacted = await sync.contactFederatedDeployment(added.domain);
      setResult(format(m.federation.outcome[contacted.outcome], { domain: added.domain }));
    } catch (e) {
      setResult(
        format(m.federation.addedNotReached, { domain: added.domain, detail: problemText(e) }),
      );
    }
    onAdded();
    setPending(false);
  }

  return (
    <Form
      aria-label={m.federation.add}
      onSubmit={(event) => {
        event.preventDefault();
        void add();
      }}
      className="flex flex-col gap-3 rounded-lg border border-line bg-surface-raised p-4"
    >
      <div>
        <h3 className="font-semibold">{m.federation.add}</h3>
        <p className="text-sm text-ink-muted">{m.federation.addHint}</p>
      </div>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-[1fr_1fr]">
        <TextField value={domain} onChange={setDomain} isRequired className={fieldClass}>
          <Label className={labelClass}>{m.federation.domain}</Label>
          <Input
            placeholder={m.federation.domainPlaceholder}
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            className={inputClass}
          />
        </TextField>
        <TextField
          value={note}
          onChange={setNote}
          maxLength={MAX_NOTE_CHARS}
          className={fieldClass}
        >
          <Label className={labelClass}>{m.federation.note}</Label>
          <Input placeholder={m.federation.notePlaceholder} className={inputClass} />
        </TextField>
      </div>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <p role="status" className="text-sm empty:hidden">
        {result ?? ""}
      </p>
      <Button
        type="submit"
        isDisabled={pending || domain.trim() === ""}
        className={primaryButtonClass + " self-start"}
      >
        {pending ? m.federation.adding : m.federation.addButton}
      </Button>
    </Form>
  );
}

/** Who a deployment admits, as its gates and lists decide now. */
function Admits({ deployment }: { deployment: FederatedDeployment }) {
  const m = useMessages();
  const admitted = (
    Object.keys(m.federation.admission) as (keyof Messages["federation"]["admission"])[]
  ).filter((way) => deployment.admission[way]);
  if (admitted.length === 0) {
    return <span className="text-ink-muted">{m.federation.admitsNone}</span>;
  }
  return (
    <ul className="flex flex-wrap gap-1">
      {admitted.map((way) => (
        <li
          key={way}
          className="rounded bg-surface-hover px-1.5 py-0.5 text-xs whitespace-nowrap text-ink"
        >
          {m.federation.admission[way]}
        </li>
      ))}
    </ul>
  );
}

/** A deployment's actions: check it, its lists, its offered key, and forgetting it. */
function Actions({
  deployment,
  listsInForce,
  onChanged,
}: {
  deployment: FederatedDeployment;
  listsInForce: readonly FederationList[];
  onChanged: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const [checking, setChecking] = useState(false);
  const [said, setSaid] = useState<{ text: string; problem: boolean } | null>(null);
  const [dialog, setDialog] = useState<"lists" | "key" | "forget" | null>(null);
  const close = () => {
    setDialog(null);
  };
  const { domain } = deployment;
  return (
    <span className="flex flex-col items-end gap-1">
      <span className="flex flex-wrap justify-end gap-1">
        <Button
          aria-label={format(m.federation.checkLabel, { domain })}
          isDisabled={checking}
          onPress={() => {
            setChecking(true);
            setSaid(null);
            sync.contactFederatedDeployment(domain).then(
              (contacted) => {
                setSaid({
                  text: format(m.federation.outcome[contacted.outcome], { domain }),
                  problem: contacted.outcome === "keyChanged",
                });
                setChecking(false);
                onChanged();
              },
              (e: unknown) => {
                setSaid({ text: problemText(e), problem: true });
                setChecking(false);
              },
            );
          }}
          className={smallButtonClass}
        >
          {checking ? m.federation.checking : m.federation.check}
        </Button>
        <Button
          aria-label={format(m.federation.listsLabel, { domain })}
          onPress={() => {
            setDialog("lists");
          }}
          className={smallButtonClass}
        >
          {m.federation.lists}
        </Button>
        {deployment.offeredKey != null && (
          <Button
            aria-label={format(m.federation.reviewKeyLabel, { domain })}
            onPress={() => {
              setDialog("key");
            }}
            className={smallButtonClass + " text-danger hover:text-danger"}
          >
            <KeyIcon size={14} aria-hidden="true" />
            {m.federation.reviewKey}
          </Button>
        )}
        <Button
          aria-label={format(m.federation.forgetLabel, { domain })}
          onPress={() => {
            setDialog("forget");
          }}
          className={smallButtonClass + " text-danger hover:text-danger"}
        >
          {m.federation.forget}
        </Button>
      </span>
      <span
        role="status"
        className={"text-xs empty:hidden " + (said?.problem === true ? "text-danger" : "")}
      >
        {said?.text ?? ""}
      </span>
      <ListsDialog
        deployment={dialog === "lists" ? deployment : null}
        listsInForce={listsInForce}
        onClose={close}
        onChanged={onChanged}
      />
      <KeyDialog
        deployment={dialog === "key" ? deployment : null}
        onClose={close}
        onChanged={onChanged}
      />
      <ForgetDialog
        deployment={dialog === "forget" ? deployment : null}
        onClose={close}
        onChanged={onChanged}
      />
    </span>
  );
}

/** A modal dialog over the dashboard, open while it has a deployment. */
function DeploymentDialog({
  open,
  onClose,
  alert = false,
  children,
}: {
  open: boolean;
  onClose: () => void;
  alert?: boolean;
  children: ReactNode;
}) {
  return (
    <ModalOverlay
      isOpen={open}
      onOpenChange={(isOpen) => {
        if (!isOpen) {
          onClose();
        }
      }}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog role={alert ? "alertdialog" : "dialog"} className={dialogClass}>
          {children}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

/** Which subject and direction a list governs, as its hint. */
function listHint(m: Messages, list: FederationList): string {
  const who = list.startsWith("users") ? m.federation.users : m.federation.bots;
  const way = list.includes("Emigration")
    ? m.federation.going
    : list.includes("Immigration")
      ? m.federation.coming
      : m.federation.sharedList;
  const kind = list.endsWith("Allow") ? m.federation.gate.allowList : m.federation.gate.blockList;
  return `${who} · ${way} · ${kind}`;
}

/** Puts a deployment on the lists in force, or takes it off, a checkbox each. */
function ListsDialog({
  deployment,
  listsInForce,
  onClose,
  onChanged,
}: {
  deployment: FederatedDeployment | null;
  listsInForce: readonly FederationList[];
  onClose: () => void;
  onChanged: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const [on, setOn] = useState<ReadonlySet<FederationList> | null>(null);
  const [error, setError] = useState<string | null>(null);
  const lists = on ?? new Set(deployment?.lists ?? []);
  return (
    <DeploymentDialog
      open={deployment !== null}
      onClose={() => {
        setOn(null);
        setError(null);
        onClose();
      }}
    >
      <DialogHeading>
        {format(m.federation.listsHeading, { domain: deployment?.domain ?? "" })}
      </DialogHeading>
      <p className="text-sm text-ink-muted">
        {listsInForce.length === 0 ? m.federation.noListsInForce : m.federation.listsHint}
      </p>
      <div className="flex flex-col gap-2">
        {listsInForce.map((list) => (
          <ChoiceCheckbox
            key={list}
            isSelected={lists.has(list)}
            onChange={(selected) => {
              if (deployment === null) {
                return;
              }
              const next = new Set(lists);
              if (selected) {
                next.add(list);
              } else {
                next.delete(list);
              }
              setOn(next);
              setError(null);
              sync
                .setFederationListed(deployment.domain, list, selected)
                .then(onChanged, (e: unknown) => {
                  setOn(lists);
                  setError(problemText(e));
                });
            }}
            label={m.federation.listNames[list]}
            hint={listHint(m, list)}
          />
        ))}
      </div>
      {error !== null && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      <Button slot="close" className={secondaryButtonClass + " self-end"}>
        {m.federation.done}
      </Button>
    </DeploymentDialog>
  );
}

/** Shows a deployment's pinned and offered keys, and accepts the offered one. */
function KeyDialog({
  deployment,
  onClose,
  onChanged,
}: {
  deployment: FederatedDeployment | null;
  onClose: () => void;
  onChanged: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const close = () => {
    setError(null);
    onClose();
  };
  return (
    <DeploymentDialog open={deployment !== null} onClose={close} alert>
      <DialogHeading>
        {format(m.federation.keyHeading, { domain: deployment?.domain ?? "" })}
      </DialogHeading>
      <p className="text-sm text-ink-muted">{m.federation.keyHint}</p>
      <dl className="grid grid-cols-[auto_1fr] items-baseline gap-x-4 gap-y-2 text-sm">
        <dt className="text-ink-muted">{m.federation.pinnedKey}</dt>
        <dd>
          <code className="font-mono text-xs break-all">{deployment?.publicKeyFingerprint}</code>
        </dd>
        <dt className="text-ink-muted">{m.federation.offeredKey}</dt>
        <dd>
          <code className="font-mono text-xs break-all">{deployment?.offeredKeyFingerprint}</code>
        </dd>
      </dl>
      {error !== null && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      <Button
        isDisabled={pending}
        onPress={() => {
          if (deployment?.offeredKey == null) {
            return;
          }
          setPending(true);
          setError(null);
          sync.acceptFederatedDeploymentKey(deployment.domain, deployment.offeredKey).then(
            () => {
              setPending(false);
              onChanged();
              close();
            },
            (e: unknown) => {
              setPending(false);
              setError(problemText(e));
            },
          );
        }}
        className={dangerButtonClass + " self-end"}
      >
        {pending ? m.federation.accepting : m.federation.acceptKey}
      </Button>
    </DeploymentDialog>
  );
}

/** Confirms forgetting a deployment. */
function ForgetDialog({
  deployment,
  onClose,
  onChanged,
}: {
  deployment: FederatedDeployment | null;
  onClose: () => void;
  onChanged: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const close = () => {
    setError(null);
    onClose();
  };
  const domain = deployment?.domain ?? "";
  return (
    <DeploymentDialog open={deployment !== null} onClose={close} alert>
      <DialogHeading>{format(m.federation.forgetHeading, { domain })}</DialogHeading>
      <p className="text-sm text-ink-muted">{m.federation.forgetHint}</p>
      {error !== null && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      <Button
        isDisabled={pending}
        onPress={() => {
          setPending(true);
          setError(null);
          sync.removeFederatedDeployment(domain).then(
            () => {
              setPending(false);
              onChanged();
              close();
            },
            (e: unknown) => {
              setPending(false);
              setError(problemText(e));
            },
          );
        }}
        className={dangerButtonClass + " self-end"}
      >
        {pending ? m.federation.forgetting : m.federation.forget}
      </Button>
    </DeploymentDialog>
  );
}
