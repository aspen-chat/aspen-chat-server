import { Link, type ErrorComponentProps } from "@tanstack/react-router";
import { Button } from "react-aria-components";
import { useMessages } from "@/i18n/context";
import { linkButtonClass, outlineButtonClass } from "@/features/auth/styles";

/**
 * What a screen shows in place of a route that failed to draw: the router puts it where that
 * route would be, so the layout around it (the rail, the channel list) stays usable.
 */
export function RouteError({ reset }: ErrorComponentProps) {
  const m = useMessages();
  return (
    <main className="flex flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
      <h1 className="text-xl font-semibold">{m.routeErrorHeading}</h1>
      <Button className={outlineButtonClass} onPress={reset}>
        {m.retry}
      </Button>
      <Link to="/" className={linkButtonClass}>
        {m.backHome}
      </Link>
    </main>
  );
}
