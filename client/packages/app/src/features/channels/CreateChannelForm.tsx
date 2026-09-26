import { ApiProblemError, type ChannelType } from "@aspen/protocol";
import { CaretDownIcon } from "@phosphor-icons/react";
import { useNavigate } from "@tanstack/react-router";
import { useState, type SyntheticEvent } from "react";
import {
  Button,
  FieldError,
  Form,
  Input,
  Label,
  ListBox,
  ListBoxItem,
  Popover,
  Select,
  SelectValue,
  TextField,
} from "react-aria-components";
import { useCategories, useSync } from "@/api/hooks";
import {
  alertClass,
  fieldClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import { formString } from "@/forms";
import { useMessages } from "@/i18n/context";

const NO_CATEGORY = "none";

const selectButtonClass =
  "flex justify-between rounded-md border border-line bg-surface px-3 py-2 text-left outline-none " +
  "hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50";
const popoverClass =
  "min-w-(--trigger-width) rounded-md border border-line bg-surface-raised p-1 shadow-lg";
const optionClass =
  "cursor-default rounded px-2 py-1 text-sm outline-none focus:bg-surface-hover selected:font-medium selected:text-accent";

/**
 * Names a new channel of a fixed type, files it under a category (a given one, or one chosen
 * here when the community has any), and opens it if it is a text channel.
 */
export function CreateChannelForm({
  communityId,
  ty,
  parentCategory,
  onDone,
}: {
  communityId: string;
  ty: ChannelType;
  /** Files the channel under this category and hides the choice; absent, the form offers one. */
  parentCategory?: string;
  onDone: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const categories = useCategories(communityId);
  const [category, setCategory] = useState<string>(parentCategory ?? NO_CATEGORY);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    const name = formString(new FormData(event.currentTarget), "name").trim();
    if (name.length === 0) {
      return;
    }
    setPending(true);
    setError(null);
    try {
      const channel = await sync.createChannel(communityId, {
        name,
        ty,
        parentCategory: category === NO_CATEGORY ? null : category,
      });
      onDone();
      if (channel.ty === "Text") {
        await navigate({
          to: "/communities/$communityId/channels/$channelId",
          params: { communityId, channelId: channel.id },
        });
      }
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
      setPending(false);
    }
  }

  return (
    <Form
      onSubmit={(e) => {
        void submit(e);
      }}
      className="flex flex-col gap-3"
    >
      <TextField name="name" isRequired autoFocus className={fieldClass}>
        <Label className={labelClass}>{m.channelNameLabel}</Label>
        <Input className={inputClass} />
        <FieldError className="text-sm text-danger" />
      </TextField>
      {parentCategory === undefined && categories.length > 0 && (
        <Select
          value={category}
          onChange={(key) => {
            if (typeof key === "string") {
              setCategory(key);
            }
          }}
          className={fieldClass}
        >
          <Label className={labelClass}>{m.categoryLabel}</Label>
          <Button className={selectButtonClass}>
            <SelectValue />
            <CaretDownIcon size={14} aria-hidden="true" />
          </Button>
          <Popover className={popoverClass}>
            <ListBox className="outline-none">
              <ListBoxItem id={NO_CATEGORY} textValue={m.noCategory} className={optionClass}>
                {m.noCategory}
              </ListBoxItem>
              {categories.map((c) => (
                <ListBoxItem key={c.id} id={c.id} textValue={c.name} className={optionClass}>
                  {c.name}
                </ListBoxItem>
              ))}
            </ListBox>
          </Popover>
        </Select>
      )}
      {error !== null && (
        <p role="alert" className={alertClass}>
          {error}
        </p>
      )}
      <Button type="submit" isDisabled={pending} className={primaryButtonClass}>
        {pending ? m.creating : m.create}
      </Button>
    </Form>
  );
}
