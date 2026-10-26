package com.aspen_chat.client.age;

import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;
import com.google.android.play.agesignals.AgeSignalsManagerFactory;
import com.google.android.play.agesignals.AgeSignalsRequest;

/**
 * Whether the device says its user is under 18 where the law has Google Play say so (`AspenAge`
 * in the app's `src/api/ageGate.ts`, which iOS's plugin of the same name answers too), through
 * Play Age Signals. Nothing is kept: the app asks each time it starts.
 */
@CapacitorPlugin(name = "AspenAge")
public class AspenAgePlugin extends Plugin {

    /** The youngest age that may use Aspen. */
    static final int ADULT = 18;

    /**
     * Whether a result's age band says its user is under {@link #ADULT}: Play gives no upper
     * bound for the highest band (18 and over) or where it has no age to share, whether no law
     * asks for one there or the user has not shared it. Aspen keeps Play's default bands, whose
     * upper bounds below 18 are 12, 15, and 17.
     */
    static boolean underage(Integer ageUpper) {
        return ageUpper != null && ageUpper < ADULT;
    }

    /**
     * Answers `{ underage }`. A device without Play, an app not installed from it, or any other
     * failure answers `false`, as a device that reports no age does.
     */
    @PluginMethod
    public void check(PluginCall call) {
        try {
            AgeSignalsManagerFactory.create(getContext())
                .checkAgeSignals(AgeSignalsRequest.builder().build())
                .addOnSuccessListener(result -> answer(call, underage(result.ageUpper())))
                .addOnFailureListener(error -> answer(call, false));
        } catch (RuntimeException error) {
            answer(call, false);
        }
    }

    private static void answer(PluginCall call, boolean underage) {
        JSObject answer = new JSObject();
        answer.put("underage", underage);
        call.resolve(answer);
    }
}
