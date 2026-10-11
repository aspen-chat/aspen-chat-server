import { groupChannels, type Channel } from "@aspen/protocol";
import {
  DotsSixVerticalIcon,
  FolderSimpleIcon,
  HashIcon,
  PencilSimpleIcon,
  PlusIcon,
  SpeakerHighIcon,
  TrashIcon,
} from "@phosphor-icons/react";
import { useRef, useState, type ReactNode } from "react";
import {
  Button,
  Dialog,
  DropIndicator,
  GridList,
  GridListItem,
  Modal,
  ModalOverlay,
  useDragAndDrop,
} from "react-aria-components";
import { useCan, useCategories, useChannels, usePluginKind, useSync } from "@/api/hooks";
import { problemText } from "@/api/problemText";
import { alertClass, hintClass } from "@/features/auth/styles";
import { DeleteCategoryDialog, RenameCategoryDialog } from "@/features/channels/CategoryDialogs";
import { DeleteChannelDialog, RenameChannelDialog } from "@/features/channels/ChannelDialogs";
import { CreateCategoryForm } from "@/features/channels/CreateCategoryForm";
import {
  layoutRows,
  planMove,
  type LayoutMove,
  type LayoutRow,
} from "@/features/community-settings/layoutOrder";
import {
  dialogClass,
  modalClass,
  overlayClass,
  planeSurfaceClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useReorderGlide } from "@/features/layout/motion";
import type { DropAt } from "@/features/layout/reorder";
import { Tooltip } from "@/features/layout/Tooltip";
import { PluginGlyph } from "@/features/plugins/PluginChannel";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** The drag type the layout's rows carry, so they accept only each other. */
const LAYOUT_DRAG_TYPE = "application/x-aspen-layout-row";

const iconButtonClass =
  "rounded p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pointer-coarse:p-2 focus-visible:ring-2 focus-visible:ring-accent/50";

/** The dialog a row's rename or delete button opened, kept while it closes. */
interface RowDialog {
  kind: "renameChannel" | "deleteChannel" | "renameCategory" | "deleteCategory";
  id: string;
  open: boolean;
}

/**
 * A community's channels and categories as one list to arrange: the top level's channels, then
 * each category with its own. Dragging a row, with the pointer or from its handle with the
 * keyboard, moves it as `planMove` says and saves the new arrangement at once. Holders of
 * Manage channels move, rename, and delete channels; holders of Manage categories do the same
 * to categories and make new ones.
 */
export function LayoutPanel({ communityId }: { communityId: string }) {
  const m = useMessages();
  const sync = useSync();
  const channels = useChannels(communityId);
  const categories = useCategories(communityId);
  const manageChannels = useCan(communityId, "manageChannels");
  const manageCategories = useCan(communityId, "manageCategories");
  const [error, setError] = useState<string | null>(null);
  const [dialog, setDialog] = useState<RowDialog | null>(null);
  const [creating, setCreating] = useState(false);
  const { topLevel, byCategory } = groupChannels(channels, categories);
  const rows = layoutRows(topLevel, categories, byCategory);
  const movable = (row: LayoutRow) =>
    row.kind === "channel" ? manageChannels : row.kind === "category" && manageCategories;

  /** What dropping the row `key` at `target` would change, if the caller may move that row. */
  function plan(key: string, target: DropAt): LayoutMove | null {
    const row = rows.find((r) => r.key === key);
    return row !== undefined && movable(row) ? planMove(rows, key, target) : null;
  }

  function drop(key: string, target: DropAt) {
    const move = plan(key, target);
    if (move === null) {
      return;
    }
    setError(null);
    const saved =
      move.kind === "channels"
        ? sync.arrangeChannels(move.order, move.parentCategory)
        : sync.arrangeCategories(move.order);
    saved.catch((e: unknown) => {
      setError(problemText(e));
    });
  }

  /** The row being dragged, which decides where it may be dropped. */
  const dragging = useRef<string | null>(null);
  const { dragAndDropHooks } = useDragAndDrop({
    getItems: (keys) => Array.from(keys, (key) => ({ [LAYOUT_DRAG_TYPE]: String(key) })),
    acceptedDragTypes: [LAYOUT_DRAG_TYPE],
    onDragStart: (event) => {
      dragging.current = Array.from(event.keys, String)[0] ?? null;
    },
    onDragEnd: () => {
      dragging.current = null;
    },
    // Every row can be picked up, the top level's heading and rows the caller may not move
    // among them, so a row that may not move is one with nowhere to be dropped.
    getDropOperation: (target) =>
      dragging.current !== null &&
      target.type === "item" &&
      plan(dragging.current, { key: String(target.key), dropPosition: target.dropPosition }) !==
        null
        ? "move"
        : "cancel",
    onReorder: (event) => {
      const key = Array.from(event.keys, String)[0];
      if (key !== undefined) {
        drop(key, { key: String(event.target.key), dropPosition: event.target.dropPosition });
      }
    },
    onItemDrop: (event) => {
      if (dragging.current !== null) {
        drop(dragging.current, { key: String(event.target.key), dropPosition: "on" });
      }
    },
    renderDropIndicator: (target) => (
      <DropIndicator
        target={target}
        className="mx-2 h-0.5 rounded-full bg-transparent drop-target:bg-accent drop-target:forced-fill"
      />
    ),
  });
  const list = useRef<HTMLDivElement>(null);
  useReorderGlide(list, rows.map((row) => row.key).join(" "));

  const openDialog = (kind: RowDialog["kind"], id: string) => {
    setDialog({ kind, id, open: true });
  };
  const closeDialog = (open: boolean) => {
    if (!open) {
      setDialog((current) => current && { ...current, open: false });
    }
  };
  const dialogChannel =
    dialog?.kind === "renameChannel" || dialog?.kind === "deleteChannel"
      ? channels.find((c) => c.id === dialog.id)
      : undefined;
  const dialogCategory =
    dialog?.kind === "renameCategory" || dialog?.kind === "deleteCategory"
      ? categories.find((c) => c.id === dialog.id)
      : undefined;

  return (
    <div className="flex flex-col gap-3">
      <p className={hintClass}>{m.channelLayout.hint}</p>
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <GridList
        ref={list}
        aria-label={m.channelLayout.listLabel}
        items={rows}
        // A row's rendering is cached by its record, and what it offers follows these too.
        dependencies={[manageChannels, manageCategories]}
        selectionMode="none"
        dragAndDropHooks={dragAndDropHooks}
        className={planeSurfaceClass + " flex flex-col gap-0.5 outline-none"}
      >
        {(row) => (
          <GridListItem
            id={row.key}
            textValue={
              row.kind === "topLevel"
                ? m.channelLayout.topLevel
                : row.kind === "category"
                  ? row.category.name
                  : row.channel.name
            }
            className={
              "flex items-center gap-1.5 rounded-md px-2 py-1 text-sm outline-none " +
              "focus-visible:ring-2 focus-visible:ring-accent/50 dragging:opacity-50 " +
              "drop-target:bg-accent-soft drop-target:forced-outline " +
              (row.kind === "channel" ? "ps-7 text-ink-muted" : "mt-1 font-semibold first:mt-0")
            }
          >
            {row.kind === "topLevel" ? (
              <>
                <NoDragHandle />
                <span className="min-w-0 flex-1 truncate text-ink-muted">
                  {m.channelLayout.topLevel}
                </span>
              </>
            ) : row.kind === "category" ? (
              <>
                {manageCategories ? (
                  <DragHandle
                    label={format(m.channelLayout.dragCategory, { category: row.category.name })}
                  />
                ) : (
                  <NoDragHandle />
                )}
                <FolderSimpleIcon size={16} aria-hidden="true" className="shrink-0" />
                <span className="min-w-0 flex-1 truncate">{row.category.name}</span>
                {manageCategories && (
                  <>
                    <RowButton
                      label={format(m.channelLayout.renameCategory, {
                        category: row.category.name,
                      })}
                      onPress={() => {
                        openDialog("renameCategory", row.key);
                      }}
                    >
                      <PencilSimpleIcon size={14} aria-hidden="true" />
                    </RowButton>
                    <RowButton
                      label={format(m.channelLayout.deleteCategory, {
                        category: row.category.name,
                      })}
                      onPress={() => {
                        openDialog("deleteCategory", row.key);
                      }}
                    >
                      <TrashIcon size={14} aria-hidden="true" />
                    </RowButton>
                  </>
                )}
              </>
            ) : (
              <>
                {manageChannels ? (
                  <DragHandle label={format(m.dragChannel, { channel: row.channel.name })} />
                ) : (
                  <NoDragHandle />
                )}
                <ChannelGlyph channel={row.channel} />
                <span className="min-w-0 flex-1 truncate">{row.channel.name}</span>
                {manageChannels && (
                  <>
                    <RowButton
                      label={format(m.channelLayout.renameChannel, { channel: row.channel.name })}
                      onPress={() => {
                        openDialog("renameChannel", row.key);
                      }}
                    >
                      <PencilSimpleIcon size={14} aria-hidden="true" />
                    </RowButton>
                    <RowButton
                      label={format(m.channelLayout.deleteChannel, { channel: row.channel.name })}
                      onPress={() => {
                        openDialog("deleteChannel", row.key);
                      }}
                    >
                      <TrashIcon size={14} aria-hidden="true" />
                    </RowButton>
                  </>
                )}
              </>
            )}
          </GridListItem>
        )}
      </GridList>
      {manageCategories && (
        <Button
          onPress={() => {
            setCreating(true);
          }}
          className={secondaryButtonClass + " flex items-center gap-1 self-start"}
        >
          <PlusIcon size={14} aria-hidden="true" />
          {m.newCategory}
        </Button>
      )}
      <ModalOverlay
        isOpen={creating}
        onOpenChange={setCreating}
        isDismissable
        className={overlayClass}
      >
        <Modal className={modalClass}>
          <Dialog className={dialogClass}>
            <DialogHeading>{m.newCategory}</DialogHeading>
            <CreateCategoryForm
              communityId={communityId}
              onDone={() => {
                setCreating(false);
              }}
            />
          </Dialog>
        </Modal>
      </ModalOverlay>
      {dialogChannel !== undefined && dialog?.kind === "renameChannel" && (
        <RenameChannelDialog
          key={dialogChannel.id}
          channel={dialogChannel}
          isOpen={dialog.open}
          onOpenChange={closeDialog}
        />
      )}
      {dialogChannel !== undefined && dialog?.kind === "deleteChannel" && (
        <DeleteChannelDialog
          key={dialogChannel.id}
          channel={dialogChannel}
          isOpen={dialog.open}
          onOpenChange={closeDialog}
        />
      )}
      {dialogCategory !== undefined && dialog?.kind === "renameCategory" && (
        <RenameCategoryDialog
          key={dialogCategory.id}
          category={dialogCategory}
          isOpen={dialog.open}
          onOpenChange={closeDialog}
        />
      )}
      {dialogCategory !== undefined && dialog?.kind === "deleteCategory" && (
        <DeleteCategoryDialog
          key={dialogCategory.id}
          category={dialogCategory}
          isOpen={dialog.open}
          onOpenChange={closeDialog}
        />
      )}
    </div>
  );
}

/** The handle keyboard and screen reader users drag a row with; pointer users drag the row. */
function DragHandle({ label }: { label: string }) {
  return (
    <Button
      slot="drag"
      aria-label={label}
      className="shrink-0 cursor-grab rounded p-0.5 text-ink-faint outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      <DotsSixVerticalIcon size={14} aria-hidden="true" />
    </Button>
  );
}

/**
 * What a row that may not be moved has for a handle: nothing anyone can see, focus, or hear
 * of. React Aria makes every row of a list with drag and drop draggable and asks each for a
 * handle; this one answers it without offering a drag that could be dropped nowhere.
 */
function NoDragHandle() {
  return <Button slot="drag" isDisabled className="hidden" />;
}

function RowButton({
  label,
  onPress,
  children,
}: {
  label: string;
  onPress: () => void;
  children: ReactNode;
}) {
  return (
    <Tooltip text={label}>
      <Button aria-label={label} onPress={onPress} className={iconButtonClass}>
        {children}
      </Button>
    </Tooltip>
  );
}

function ChannelGlyph({ channel }: { channel: Channel }) {
  const kind = usePluginKind(channel);
  return channel.ty === "text" ? (
    <HashIcon size={16} aria-hidden="true" className="shrink-0 text-ink-faint" />
  ) : channel.ty === "plugin" ? (
    <PluginGlyph glyph={kind?.kind.glyph} size={16} className="shrink-0 text-ink-faint" />
  ) : (
    <SpeakerHighIcon size={16} aria-hidden="true" className="shrink-0" />
  );
}
