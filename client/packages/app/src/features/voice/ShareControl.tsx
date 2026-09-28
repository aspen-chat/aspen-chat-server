import { ScreencastIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import { Button, Menu, MenuItem, MenuTrigger, Popover } from "react-aria-components";
import { useSync, useVoiceCall } from "@/api/hooks";
import { GameCaptureDialog } from "@/features/voice/GameCaptureDialog";
import { gameCaptureBridge, type GameCaptureBridge } from "@/features/voice/gameCapture";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMessages } from "@/i18n/context";

/**
 * Whether this browser can capture a screen: mobile browsers cannot, and a page not served
 * over HTTPS has no media devices at all.
 */
function canShareScreen(): boolean {
  return (
    "mediaDevices" in navigator && typeof navigator.mediaDevices.getDisplayMedia === "function"
  );
}

/**
 * The screen-share control, shared by the call bar and the voice channel screen so both offer
 * the same choices. In a browser it shares the screen straight away; on the desktop shell,
 * where the game-capture helper is present, it opens a menu to share the screen or a game.
 * While a share is running it becomes a stop control. `variant` picks the presentation: `bar`
 * is the icon-only button in the call bar, `panel` the labelled button in the channel header.
 * A share error is reported through `onError` so the caller can place the message; declining
 * the browser's own picker is silent.
 */
export function ShareControl({
  variant,
  onError,
}: {
  variant: "bar" | "panel";
  onError?: (message: string | null) => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const call = useVoiceCall();
  const [gameBridge, setGameBridge] = useState<GameCaptureBridge | null>(null);
  const [choosingGame, setChoosingGame] = useState(false);

  // The desktop shell can share games; the option appears once it reports a way to: a game
  // capture kind, or an application's sound to go with a screen share.
  useEffect(() => {
    const bridge = gameCaptureBridge();
    if (bridge === null) {
      return;
    }
    let cancelled = false;
    bridge.kinds().then(
      (catalogue) => {
        if (
          !cancelled &&
          (catalogue.kinds.length > 0 || catalogue.applicationAudio !== null || import.meta.env.DEV)
        ) {
          setGameBridge(bridge);
        }
      },
      () => undefined,
    );
    return () => {
      cancelled = true;
    };
  }, []);

  const share = async () => {
    onError?.(null);
    try {
      await sync.voice.startScreenShare();
    } catch (error) {
      // Declining the browser's picker is an ordinary outcome, not an error to show.
      if (!(error instanceof DOMException && error.name === "NotAllowedError")) {
        onError?.(error instanceof Error ? error.message : String(error));
      }
    }
  };

  const bar = variant === "bar";
  const triggerClass = bar
    ? "rounded-md p-1.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
      "pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 pointer-coarse:p-2.5"
    : "flex items-center gap-1.5 rounded-md px-2 py-1 text-sm text-ink-muted outline-none " +
      "hover:bg-surface-hover hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50";
  const menuItemClass =
    "cursor-default rounded px-2 py-1 text-sm outline-none focus:bg-surface-hover";

  if (call.sharingScreen) {
    const stop = (
      <Button
        aria-label={m.voice.stopSharing}
        aria-pressed
        onPress={() => {
          sync.voice.stopScreenShare();
        }}
        className={
          bar
            ? triggerClass + " text-accent"
            : "flex items-center gap-1.5 rounded-md px-2 py-1 text-sm outline-none " +
              "bg-danger-soft text-danger hover:bg-danger-soft/80 focus-visible:ring-2 focus-visible:ring-accent/50"
        }
      >
        <ScreencastIcon size={18} aria-hidden="true" />
        {!bar && m.voice.stopSharing}
      </Button>
    );
    return bar ? <Tooltip text={m.voice.stopSharing}>{stop}</Tooltip> : stop;
  }

  const dialog = choosingGame && gameBridge !== null && (
    <GameCaptureDialog
      bridge={gameBridge}
      onClose={() => {
        setChoosingGame(false);
      }}
    />
  );

  if (gameBridge === null) {
    // Mobile browsers cannot capture the screen; there is nothing to offer.
    if (!canShareScreen()) {
      return null;
    }
    const button = (
      <Button
        aria-label={m.voice.shareScreen}
        onPress={() => {
          void share();
        }}
        className={triggerClass}
      >
        <ScreencastIcon size={18} aria-hidden="true" />
        {!bar && m.voice.shareScreen}
      </Button>
    );
    return (
      <>
        {bar ? <Tooltip text={m.voice.shareScreen}>{button}</Tooltip> : button}
        {dialog}
      </>
    );
  }

  const trigger = (
    <Button aria-label={m.voice.shareMenu} className={triggerClass}>
      <ScreencastIcon size={18} aria-hidden="true" />
      {!bar && m.voice.shareMenu}
    </Button>
  );
  return (
    <>
      <MenuTrigger>
        {bar ? <Tooltip text={m.voice.shareMenu}>{trigger}</Tooltip> : trigger}
        <Popover className="rounded-md border border-line bg-surface-raised p-1 shadow-lg">
          <Menu
            className="min-w-40 outline-none"
            onAction={(key) => {
              if (key === "screen") {
                void share();
              } else if (key === "game") {
                setChoosingGame(true);
              }
            }}
          >
            <MenuItem id="screen" className={menuItemClass}>
              {m.voice.shareYourScreen}
            </MenuItem>
            <MenuItem id="game" className={menuItemClass}>
              {m.voice.shareGame}
            </MenuItem>
          </Menu>
        </Popover>
      </MenuTrigger>
      {dialog}
    </>
  );
}
