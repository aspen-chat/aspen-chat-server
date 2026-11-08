package com.aspen_chat.client.audio;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import android.media.AudioDeviceInfo;
import org.junit.Test;

public class AspenAudioRoutePluginTest {

    @Test
    public void aPhoneAloneHasItsReceiverAndLoudspeaker() {
        AspenAudioRoutePlugin.Outputs outputs = AspenAudioRoutePlugin.Outputs.of(
            new int[] { AudioDeviceInfo.TYPE_BUILTIN_EARPIECE, AudioDeviceInfo.TYPE_BUILTIN_SPEAKER }
        );
        assertTrue(outputs.receiver);
        assertTrue(outputs.speaker);
        assertFalse(outputs.other);
    }

    @Test
    public void aHeadsetIsSomethingElse() {
        AspenAudioRoutePlugin.Outputs outputs = AspenAudioRoutePlugin.Outputs.of(
            new int[] {
                AudioDeviceInfo.TYPE_BUILTIN_EARPIECE,
                AudioDeviceInfo.TYPE_BUILTIN_SPEAKER,
                AudioDeviceInfo.TYPE_BLUETOOTH_SCO,
            }
        );
        assertTrue(outputs.other);
    }

    @Test
    public void aTabletHasNoReceiver() {
        AspenAudioRoutePlugin.Outputs outputs = AspenAudioRoutePlugin.Outputs.of(
            new int[] { AudioDeviceInfo.TYPE_BUILTIN_SPEAKER, AudioDeviceInfo.TYPE_TELEPHONY }
        );
        assertFalse(outputs.receiver);
        assertFalse(outputs.other);
    }
}
