import { ScreenShareError } from "@aspen/protocol";
import { ScreencastIcon } from "@phosphor-icons/react";
import { useEffect, useState } from "react";
import { Button, Menu, MenuItem, MenuTrigger, Popover } from "react-aria-components";
import { useSync, useVoiceCall } from "@/api/hooks";
import { GameCaptureDialog } from "@/features/voice/GameCaptureDialog";
import { gameCaptureBridge, type GameCaptureBridge } from "@/features/voice/gameCapture";
import { Tooltip } from "@/features/layout/Tooltip";
import { callBarButtonClass } from "@/features/voice/callBarButton";
import { useMessages } from "@/i18n/context";

/**
 * The screen-share control, shared by the call bar and the voice channel screen so both offer
 * the same choices. In a browser it shares the screen straight away; in the mobile apps the
 * phone captures its own screen through the system's consent, which is the choice of what to
 * share, so no picker of the page's is shown; on the desktop shell, where the game-capture
 * helper is present, it opens a menu to share the screen or a game. It is offered only where
 * the call can capture a screen (`Call.hasScreenCapture`: a browser with `getDisplayMedia`, or a
 * native call). While a share is running it becomes a stop control. `variant` picks the
 * presentation: `bar` is the icon-only button in the call bar, `panel` the labelled button in
 * the channel header. Why a share failed is the call's `shareError`, which `ShareProblem` shows
 * under the bar and in the channel's header; declining the system's picker is silent.
 */
export function ShareControl({ variant }: { variant: "bar" | "panel" }) {
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
    try {
      await sync.voice.startScreenShare();
    } catch (error) {
      // What failed is the call's `shareError`, shown by `ShareProblem`; anything else thrown
      // is the call's own and not an outcome of the user's.
      if (!(error instanceof ScreenShareError)) {
        throw error;
      }
    }
  };

  const bar = variant === "bar";
  const triggerClass = bar
    ? callBarButtonClass
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
        <ScreencastIcon size={bar ? 24 : 18} aria-hidden="true" />
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

  // A call that does not allow sharing offers none.
  if (call.channelId !== null && !call.canShare && call.status === "connected") {
    return null;
  }

  if (gameBridge === null) {
    // A mobile browser cannot capture the screen; there is nothing to offer.
    if (!sync.voice.hasScreenCapture) {
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
        <ScreencastIcon size={bar ? 24 : 18} aria-hidden="true" />
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
      <ScreencastIcon size={bar ? 24 : 18} aria-hidden="true" />
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
