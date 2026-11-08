package com.aspen_chat.client.audio;

import android.content.Context;
import android.media.AudioDeviceCallback;
import android.media.AudioDeviceInfo;
import android.media.AudioManager;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;
import java.util.List;
import java.util.concurrent.Executor;

/**
 * Whether a call plays through the phone's receiver or its loudspeaker (`AspenAudioRoute` in the
 * app's `src/api/audioRoute.ts`, which iOS's plugin of the same name answers too), which the
 * web view's own WebRTC cannot choose. The app holds the route from a call's start to its end,
 * and only while nothing else that takes a call on its own (headphones, a headset) is connected.
 */
@CapacitorPlugin(name = "AspenAudioRoute")
public class AspenAudioRoutePlugin extends Plugin {

    /** What a phone can play a call through, as the app's `PhoneOutputs` names it. */
    static final class Outputs {
        final boolean receiver;
        final boolean speaker;
        final boolean other;

        Outputs(boolean receiver, boolean speaker, boolean other) {
            this.receiver = receiver;
            this.speaker = speaker;
            this.other = other;
        }

        /** From the types of the devices connected (`AudioDeviceInfo.TYPE_*`). */
        static Outputs of(int[] types) {
            boolean receiver = false;
            boolean speaker = false;
            boolean other = false;
            for (int type : types) {
                receiver |= type == AudioDeviceInfo.TYPE_BUILTIN_EARPIECE;
                speaker |= type == AudioDeviceInfo.TYPE_BUILTIN_SPEAKER;
                other |= takesCalls(type);
            }
            return new Outputs(receiver, speaker, other);
        }

        JSObject toJs() {
            JSObject js = new JSObject();
            js.put("receiver", receiver);
            js.put("speaker", speaker);
            js.put("other", other);
            return js;
        }
    }

    /** Whether a device of this type is one the system sends a call to once it is connected. */
    static boolean takesCalls(int type) {
        switch (type) {
            case AudioDeviceInfo.TYPE_WIRED_HEADSET:
            case AudioDeviceInfo.TYPE_WIRED_HEADPHONES:
            case AudioDeviceInfo.TYPE_BLUETOOTH_SCO:
            case AudioDeviceInfo.TYPE_BLUETOOTH_A2DP:
            case AudioDeviceInfo.TYPE_USB_HEADSET:
            case AudioDeviceInfo.TYPE_USB_DEVICE:
            case AudioDeviceInfo.TYPE_HEARING_AID:
            case AudioDeviceInfo.TYPE_BLE_HEADSET:
            case AudioDeviceInfo.TYPE_BLE_SPEAKER:
                return true;
            default:
                return false;
        }
    }

    private AudioManager audio;
    /** Whether the route is the app's now, and so whether `release` has anything to give back. */
    private boolean held = false;
    /** The loudspeaker (`true`) or the receiver, while `held`. */
    private boolean wanted = true;
    private int modeBefore = AudioManager.MODE_NORMAL;
    private boolean speakerphoneBefore = false;

    private final AudioDeviceCallback devicesChanged = new AudioDeviceCallback() {
        @Override
        public void onAudioDevicesAdded(AudioDeviceInfo[] added) {
            notifyListeners("outputsChange", outputs().toJs());
        }

        @Override
        public void onAudioDevicesRemoved(AudioDeviceInfo[] removed) {
            notifyListeners("outputsChange", outputs().toJs());
        }
    };

    /**
     * The web view's WebRTC moves the call between the receiver and the loudspeaker itself when
     * it opens the microphone; while the app holds the route, such a move is put back. A move to
     * anything else (a headset connected) is the system's and left alone.
     */
    private AudioManager.OnCommunicationDeviceChangedListener communicationChanged;

    @Override
    public void load() {
        audio = (AudioManager) getContext().getSystemService(Context.AUDIO_SERVICE);
        Handler main = new Handler(Looper.getMainLooper());
        audio.registerAudioDeviceCallback(devicesChanged, main);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            communicationChanged = device -> {
                if (!held || device == null) {
                    return;
                }
                int type = device.getType();
                boolean builtIn =
                    type == AudioDeviceInfo.TYPE_BUILTIN_EARPIECE ||
                    type == AudioDeviceInfo.TYPE_BUILTIN_SPEAKER;
                if (builtIn && type != wantedType()) {
                    route();
                }
            };
            Executor onMain = main::post;
            audio.addOnCommunicationDeviceChangedListener(onMain, communicationChanged);
        }
    }

    @Override
    protected void handleOnDestroy() {
        audio.unregisterAudioDeviceCallback(devicesChanged);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && communicationChanged != null) {
            audio.removeOnCommunicationDeviceChangedListener(communicationChanged);
        }
    }

    /** Answers `{ receiver, speaker, other }`. */
    @PluginMethod
    public void outputs(PluginCall call) {
        call.resolve(outputs().toJs());
    }

    /** Plays the call through the loudspeaker (`on`) or the receiver until `release`. */
    @PluginMethod
    @SuppressWarnings("deprecation")
    public void setSpeakerphone(PluginCall call) {
        boolean on = Boolean.TRUE.equals(call.getBoolean("on", true));
        if (!held) {
            modeBefore = audio.getMode();
            speakerphoneBefore = audio.isSpeakerphoneOn();
            held = true;
        }
        wanted = on;
        // The receiver plays a call only in the communication mode.
        audio.setMode(AudioManager.MODE_IN_COMMUNICATION);
        route();
        call.resolve();
    }

    /** Gives the route back to the system, as it was before the app held it. */
    @PluginMethod
    @SuppressWarnings("deprecation")
    public void release(PluginCall call) {
        if (held) {
            held = false;
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                audio.clearCommunicationDevice();
            } else {
                audio.setSpeakerphoneOn(speakerphoneBefore);
            }
            // A mode the web view set for its own call is its to undo.
            if (
                modeBefore != AudioManager.MODE_IN_COMMUNICATION &&
                audio.getMode() == AudioManager.MODE_IN_COMMUNICATION
            ) {
                audio.setMode(modeBefore);
            }
        }
        call.resolve();
    }

    private int wantedType() {
        return wanted ? AudioDeviceInfo.TYPE_BUILTIN_SPEAKER : AudioDeviceInfo.TYPE_BUILTIN_EARPIECE;
    }

    @SuppressWarnings("deprecation")
    private void route() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            for (AudioDeviceInfo device : audio.getAvailableCommunicationDevices()) {
                if (device.getType() == wantedType()) {
                    audio.setCommunicationDevice(device);
                    return;
                }
            }
        } else {
            audio.setSpeakerphoneOn(wanted);
        }
    }

    private Outputs outputs() {
        AudioDeviceInfo[] devices;
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            List<AudioDeviceInfo> available = audio.getAvailableCommunicationDevices();
            devices = available.toArray(new AudioDeviceInfo[0]);
        } else {
            devices = audio.getDevices(AudioManager.GET_DEVICES_OUTPUTS);
        }
        int[] types = new int[devices.length];
        for (int i = 0; i < devices.length; i += 1) {
            types[i] = devices[i].getType();
        }
        return Outputs.of(types);
    }
}
