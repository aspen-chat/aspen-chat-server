import { ApiProblemError, type ReportCategory } from "@aspen/protocol";
import { ArrowDownIcon, ArrowUpIcon } from "@phosphor-icons/react";
import { useCallback, useState } from "react";
import { Button, Form, Input, Label, TextField } from "react-aria-components";
import { useSync } from "@/api/hooks";
import { ReadFailed, Section } from "@/features/admin/AdminDashboard";
import { useAdminRead } from "@/features/admin/useAdminRead";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { Skeleton } from "@/features/layout/Skeleton";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** The longest name and description a category takes, as the server counts them. */
const NAME_MAX = 64;
const DESCRIPTION_MAX = 200;

/**
 * The categories people report in, for holders of Manage report categories: the built-in ones,
 * which may be hidden and shown again (Other excepted), and the server's own, which may also be
 * renamed, described, and put in order, and to which more may be added. A hidden category is no
 * longer offered; reports made in it keep it.
 */
export function ReportCategoriesSection() {
  const m = useMessages();
  const sync = useSync();
  const load = useCallback(() => sync.admin.allReportCategories(), [sync]);
  const read = useAdminRead(load);
  const [error, setError] = useState<string | null>(null);
  const categories = read.data;
  const own = (categories ?? []).filter((c) => c.builtin == null);

  const run = (action: Promise<unknown>) => {
    setError(null);
    action.then(read.reload, (e: unknown) => {
      setError(e instanceof ApiProblemError ? e.message : String(e));
    });
  };
  const move = (id: string, by: -1 | 1) => {
    const order = own.map((c) => c.id);
    const at = order.indexOf(id);
    const to = at + by;
    if (at < 0 || to < 0 || to >= order.length) {
      return;
    }
    [order[at], order[to]] = [order[to] ?? id, order[at] ?? id];
    run(sync.admin.orderReportCategories(order));
  };

  return (
    <Section
      id="admin-report-categories"
      title={m.reports.categoriesTitle}
      hint={m.reports.categoriesHint}
    >
      {read.error !== null && <ReadFailed error={read.error} onRetry={read.reload} />}
      {categories === undefined ? (
        read.error === null && (
          <div aria-busy="true" className="flex flex-col gap-2">
            <Skeleton className="h-10 w-full" />
            <Skeleton className="h-10 w-full" />
            <Skeleton className="h-10 w-full" />
          </div>
        )
      ) : (
        <ul className="flex flex-col divide-y divide-line rounded-md border border-line">
          {categories.map((category) => (
            <li key={category.id}>
              <CategoryRow
                category={category}
                place={own.findIndex((c) => c.id === category.id)}
                ownCount={own.length}
                onHidden={(hidden) => {
                  run(sync.admin.updateReportCategory(category.id, { hidden }));
                }}
                onMove={(by) => {
                  move(category.id, by);
                }}
                onSave={(name, description) => {
                  run(sync.admin.updateReportCategory(category.id, { name, description }));
                }}
              />
            </li>
          ))}
        </ul>
      )}
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <AddCategory onAdded={read.reload} />
    </Section>
  );
}

/** One category: its name and what it covers, and what may be done with it. */
function CategoryRow({
  category,
  place,
  ownCount,
  onHidden,
  onMove,
  onSave,
}: {
  category: ReportCategory;
  /** Its place among the server's own, or -1 for a built-in one. */
  place: number;
  ownCount: number;
  onHidden: (hidden: boolean) => void;
  onMove: (by: -1 | 1) => void;
  onSave: (name: string, description: string | null) => void;
}) {
  const m = useMessages();
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(category.name);
  const [description, setDescription] = useState(category.description ?? "");
  const builtIn = category.builtin != null;
  if (editing) {
    return (
      <Form
        className="flex flex-col gap-2 p-3"
        onSubmit={(e) => {
          e.preventDefault();
          onSave(name.trim(), description.trim() === "" ? null : description.trim());
          setEditing(false);
        }}
      >
        <CategoryFields
          name={name}
          description={description}
          onName={setName}
          onDescription={setDescription}
        />
        <div className="flex justify-end gap-2">
          <Button type="submit" isDisabled={name.trim() === ""} className={primaryButtonClass}>
            {m.reports.save}
          </Button>
        </div>
      </Form>
    );
  }
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 p-3">
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-center gap-2">
          <span className={"font-medium" + (category.hidden ? " text-ink-faint line-through" : "")}>
            {category.name}
          </span>
          {builtIn && (
            <span className="rounded border border-line px-1.5 text-xs text-ink-muted">
              {m.reports.builtIn}
            </span>
          )}
          {category.hidden && (
            <span className="rounded border border-line px-1.5 text-xs text-ink-muted">
              {m.reports.hidden}
            </span>
          )}
        </div>
        {category.description != null && (
          <p className="text-sm text-ink-muted">{category.description}</p>
        )}
      </div>
      <div className="flex items-center gap-1">
        {place >= 0 && (
          <>
            <Tooltip text={format(m.reports.moveUp, { name: category.name })}>
              <Button
                isDisabled={place === 0}
                aria-label={format(m.reports.moveUp, { name: category.name })}
                onPress={() => {
                  onMove(-1);
                }}
                className={secondaryButtonClass + " tap-target px-2"}
              >
                <ArrowUpIcon size={14} aria-hidden="true" />
              </Button>
            </Tooltip>
            <Tooltip text={format(m.reports.moveDown, { name: category.name })}>
              <Button
                isDisabled={place === ownCount - 1}
                aria-label={format(m.reports.moveDown, { name: category.name })}
                onPress={() => {
                  onMove(1);
                }}
                className={secondaryButtonClass + " tap-target px-2"}
              >
                <ArrowDownIcon size={14} aria-hidden="true" />
              </Button>
            </Tooltip>
            <Button
              aria-label={format(m.reports.editLabel, { name: category.name })}
              onPress={() => {
                setEditing(true);
              }}
              className={secondaryButtonClass}
            >
              {m.reports.edit}
            </Button>
          </>
        )}
        {category.builtin !== "other" && (
          <Button
            aria-label={format(category.hidden ? m.reports.showLabel : m.reports.hideLabel, {
              name: category.name,
            })}
            onPress={() => {
              onHidden(!category.hidden);
            }}
            className={secondaryButtonClass}
          >
            {category.hidden ? m.reports.show : m.reports.hide}
          </Button>
        )}
      </div>
    </div>
  );
}

function CategoryFields({
  name,
  description,
  onName,
  onDescription,
}: {
  name: string;
  description: string;
  onName: (name: string) => void;
  onDescription: (description: string) => void;
}) {
  const m = useMessages();
  return (
    <div className="grid gap-2 sm:grid-cols-2">
      <TextField
        value={name}
        onChange={onName}
        maxLength={NAME_MAX}
        isRequired
        className={fieldClass}
      >
        <Label className={labelClass}>{m.reports.nameLabel}</Label>
        <Input className={inputClass} />
      </TextField>
      <TextField
        value={description}
        onChange={onDescription}
        maxLength={DESCRIPTION_MAX}
        className={fieldClass}
      >
        <Label className={labelClass}>{m.reports.descriptionLabel}</Label>
        <Input className={inputClass} />
        <p className={hintClass}>{m.reports.descriptionHint}</p>
      </TextField>
    </div>
  );
}

/** Adds a category of the server's own, offered after the others it added. */
function AddCategory({ onAdded }: { onAdded: () => void }) {
  const m = useMessages();
  const sync = useSync();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <Form
      aria-label={m.reports.addCategory}
      className="flex flex-col gap-2"
      onSubmit={(e) => {
        e.preventDefault();
        if (name.trim() === "" || pending) {
          return;
        }
        setPending(true);
        setError(null);
        sync.admin
          .createReportCategory(name.trim(), description.trim() === "" ? null : description.trim())
          .then(
            () => {
              setName("");
              setDescription("");
              setPending(false);
              onAdded();
            },
            (failure: unknown) => {
              setError(failure instanceof ApiProblemError ? failure.message : String(failure));
              setPending(false);
            },
          );
      }}
    >
      <h3 className="text-sm font-semibold">{m.reports.addCategory}</h3>
      <CategoryFields
        name={name}
        description={description}
        onName={setName}
        onDescription={setDescription}
      />
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <Button
        type="submit"
        isDisabled={name.trim() === "" || pending}
        className={primaryButtonClass + " self-end"}
      >
        {m.reports.add}
      </Button>
    </Form>
  );
}
