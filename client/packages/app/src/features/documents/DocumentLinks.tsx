import { Link } from "@tanstack/react-router";
import { linkButtonClass } from "@/features/auth/styles";
import { useMessages } from "@/i18n/context";

/**
 * The ways to the deployment's Privacy Policy and Terms of Use, at the foot of the sign-in and
 * create-account screens and in About Aspen, which `onNavigate` lets close. Both are offered
 * whether or not the deployment has published them: the page says when it has not.
 */
export function DocumentLinks({ onNavigate }: { onNavigate?: () => void }) {
  const m = useMessages();
  return (
    <nav
      aria-label={m.documents.linksLabel}
      className="flex flex-wrap justify-center gap-x-4 gap-y-1 text-sm"
    >
      <Link to="/privacy" onClick={onNavigate} className={linkButtonClass}>
        {m.documents.privacyPolicy}
      </Link>
      <Link to="/terms" onClick={onNavigate} className={linkButtonClass}>
        {m.documents.termsOfUse}
      </Link>
    </nav>
  );
}
