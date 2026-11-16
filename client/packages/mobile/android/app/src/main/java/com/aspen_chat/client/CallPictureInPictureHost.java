package com.aspen_chat.client;

/**
 * What the activity tells the call plugin of the system's picture-in-picture mode, which belongs
 * to the activity: that the user is leaving (on which, before Android 12, the plugin enters the
 * mode itself), and that the mode changed. In this package rather than the call layer's, which a
 * build without WebRTC leaves out ({@code app/build.gradle}), so {@code MainActivity} can name
 * it; the plugin implements it.
 */
public interface CallPictureInPictureHost {
    void onUserLeaveHint();

    void onPictureInPictureModeChanged(boolean inPictureInPictureMode);
}
