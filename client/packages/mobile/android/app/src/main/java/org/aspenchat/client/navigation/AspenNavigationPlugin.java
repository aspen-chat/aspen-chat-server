package org.aspenchat.client.navigation;

import android.net.Uri;
import com.getcapacitor.Plugin;
import com.getcapacitor.annotation.CapacitorPlugin;
import java.util.Locale;
import java.util.Set;

/**
 * What the web view may load and what it hands to the system. Capacitor loads the app's own
 * pages in the web view and hands every other address to whichever app claims it; this lets only
 * web and mail addresses leave (`EXTERNAL_SCHEMES`), so a page (a plugin's view, a message's
 * link) cannot open another app through a scheme of its own (`intent:`, `market:`, and the like).
 * It has no methods; Capacitor asks it about each load (`shouldOverrideLoad`).
 */
@CapacitorPlugin(name = "AspenNavigation")
public class AspenNavigationPlugin extends Plugin {

    /** The schemes an address may leave the app with, to the system's handler for it. */
    static final Set<String> EXTERNAL_SCHEMES = Set.of("http", "https", "mailto");

    /** Schemes the web view keeps to itself, which Capacitor loads in place. */
    static final Set<String> IN_PLACE_SCHEMES = Set.of("data", "blob", "about");

    /**
     * Whether a load of an address with `scheme` is cancelled outright: anything but a web or
     * mail address, or one the web view keeps to itself.
     */
    static boolean refused(String scheme) {
        if (scheme == null) {
            return true;
        }
        String lower = scheme.toLowerCase(Locale.ROOT);
        return !EXTERNAL_SCHEMES.contains(lower) && !IN_PLACE_SCHEMES.contains(lower);
    }

    /** Cancels a refused load; leaves every other to Capacitor (`null`). */
    @Override
    public Boolean shouldOverrideLoad(Uri url) {
        return refused(url.getScheme()) ? Boolean.TRUE : null;
    }
}
