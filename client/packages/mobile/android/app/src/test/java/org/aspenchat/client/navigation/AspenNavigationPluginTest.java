package org.aspenchat.client.navigation;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public class AspenNavigationPluginTest {

    @Test
    public void webAndMailAddressesAndTheWebViewsOwnLoadAsCapacitorDecides() {
        for (String scheme : new String[] { "http", "https", "HTTPS", "mailto", "data", "blob", "about" }) {
            assertFalse(scheme, AspenNavigationPlugin.refused(scheme));
        }
    }

    @Test
    public void otherAppsSchemesAreRefused() {
        for (String scheme : new String[] { "intent", "market", "tel", "sms", "file", "content", "javascript", "aspen", "" }) {
            assertTrue(scheme, AspenNavigationPlugin.refused(scheme));
        }
        assertTrue(AspenNavigationPlugin.refused(null));
    }
}
