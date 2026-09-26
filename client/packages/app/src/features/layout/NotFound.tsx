import { Link } from "@tanstack/react-router";
import { useMessages } from "@/i18n/context";
import { linkButtonClass } from "@/features/auth/styles";

export function NotFound() {
  const m = useMessages();
  return (
    <main className="flex flex-1 flex-col items-center justify-center gap-2 p-6 text-center">
      <h1 className="text-xl font-semibold">{m.notFoundHeading}</h1>
      <Link to="/" className={linkButtonClass}>
        {m.backHome}
      </Link>
    </main>
  );
}
