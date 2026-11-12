package com.aspen_chat.client.push;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public class SpoilerMarkersTest {

    @Test
    public void everySpellingOfASpoilerHoldsAMarker() {
        for (String content : new String[] {
            "the killer is ||the butler||",
            "it was >!the cook!< all along",
            "> !quoted\n> across!< lines",
            "| a |\n|---|\n| \\|\\|cell\\|\\| |",
            "||||secret||",
        }) {
            assertTrue(content, SpoilerMarkers.present(content));
        }
    }

    @Test
    public void textThatOnlyLooksLikeOneIsTakenForOne() {
        assertTrue(SpoilerMarkers.present("ls | wc || true"));
        assertTrue(SpoilerMarkers.present("`||code||`"));
    }

    @Test
    public void textWithoutAMarkerIsNot() {
        for (String content : new String[] { "", "plain", "a | b", "a > b ! c < d", "> quoted!", "wait!?" }) {
            assertFalse(content, SpoilerMarkers.present(content));
        }
    }
}
