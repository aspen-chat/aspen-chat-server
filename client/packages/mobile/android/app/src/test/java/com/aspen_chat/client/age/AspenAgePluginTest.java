package com.aspen_chat.client.age;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import org.junit.Test;

public class AspenAgePluginTest {

    @Test
    public void aBandEndingBelowEighteenIsUnderage() {
        assertTrue(AspenAgePlugin.underage(12));
        assertTrue(AspenAgePlugin.underage(15));
        assertTrue(AspenAgePlugin.underage(17));
    }

    @Test
    public void theHighestBandIsNot() {
        assertFalse(AspenAgePlugin.underage(null));
        assertFalse(AspenAgePlugin.underage(18));
    }
}
