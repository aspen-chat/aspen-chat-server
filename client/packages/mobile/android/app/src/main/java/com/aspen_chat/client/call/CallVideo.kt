package com.aspen_chat.client.call

import android.graphics.Rect
import android.os.Handler
import android.os.Looper
import android.view.View
import android.widget.FrameLayout
import com.aspen_chat.client.voice.TrackId
import org.webrtc.EglBase
import org.webrtc.RendererCommon
import org.webrtc.SurfaceViewRenderer
import org.webrtc.VideoFrame
import org.webrtc.VideoSink
import org.webrtc.VideoTrack
import kotlin.math.roundToInt

/**
 * A hole the page reports for a picture (`TileReport` in the app's `nativeCall.ts`): which
 * surface (a track id) it shows, where, in the page's CSS pixels, and how the picture is drawn in
 * it. `tile` is the page's id for the hole, unique for the page's life; two tiles may show one
 * surface.
 */
data class CallVideoTile(
    val tile: String,
    val surface: TrackId,
    val x: Double,
    val y: Double,
    val width: Double,
    val height: Double,
    val visible: Boolean,
    /** The self preview, shown as a mirror would while the camera faces its user. */
    val mirrored: Boolean,
    /** Cropped to fill the hole, rather than fitted whole within it. */
    val cover: Boolean,
    /** The tile fills the screen: the picture takes the container's bounds, which it has before the page's layout after a turn. */
    val fullScreen: Boolean,
    /** The one picture the system's picture in picture shows. */
    val pictureInPicture: Boolean,
)

/**
 * The shape of a surface's picture as drawn, upright: a phone's camera held upright is taller
 * than wide. `rotation` is the sender's, in degrees clockwise, already applied to the size.
 */
data class CallVideoShape(val surface: TrackId, val width: Int, val height: Int, val rotation: Int)

/**
 * Where the call's pictures are drawn: a container behind the web view (`video_container` in
 * `capacitor_bridge_layout_main.xml`, attached by `AspenCallPlugin` as it loads), holding one
 * `SurfaceViewRenderer` per hole the page reports (`apply`), each laid out where the page says its
 * hole is, in device pixels of the page's CSS pixels, and drawing the surface's track from
 * `CallTracks`. Surface views lie behind the window by design, so the page's transparent web view
 * shows them through its holes and draws names, buttons, and dialogs over them; the self preview
 * (a mirrored tile) is ordered above the other pictures (`setZOrderMediaOverlay`), still beneath
 * the window. A full-screen tile takes the container's own bounds, which it has before the page's
 * layout after a turn. Each picture's shape is reported as its first frame arrives and whenever
 * it changes (`RendererEvents.onFrameResolutionChanged`), and the frames drawn are counted for a
 * proof. Everything runs on the main thread; the renderers' frames arrive on WebRTC's threads.
 */
class CallVideo(private val tracks: CallTracks) {
    fun interface ShapeListener {
        fun changed(shape: CallVideoShape)
    }

    /** Stops the listener it was given for. */
    inner class Subscription internal constructor(private val listener: ShapeListener) {
        fun cancel() {
            shapeListeners.remove(listener)
        }
    }

    /** One hole's renderer, counting the frames it is handed and keeping the picture's shape. */
    inner class Renderer(val tile: String, val surface: TrackId, val view: SurfaceViewRenderer) : VideoSink, RendererCommon.RendererEvents {
        /** The retained track the view draws, so the sink can be taken off it. */
        var track: VideoTrack? = null
        @Volatile
        var framesRendered = 0
            private set
        @Volatile
        var firstFrameRendered = false
            private set
        @Volatile
        var shape: CallVideoShape? = null
            private set

        override fun onFrame(frame: VideoFrame) {
            framesRendered += 1
            view.onFrame(frame)
        }

        override fun onFirstFrameRendered() {
            firstFrameRendered = true
        }

        override fun onFrameResolutionChanged(videoWidth: Int, videoHeight: Int, rotation: Int) {
            val turned = rotation == 90 || rotation == 270
            val seen = CallVideoShape(surface, if (turned) videoHeight else videoWidth, if (turned) videoWidth else videoHeight, rotation)
            if (seen == shape) {
                return
            }
            shape = seen
            main.post {
                for (listener in shapeListeners.toList()) {
                    listener.changed(seen)
                }
            }
        }
    }

    private val main = Handler(Looper.getMainLooper())
    private var container: FrameLayout? = null
    private var webView: View? = null
    private var eglContext: EglBase.Context? = null
    private var density = 1f
    var tiles: List<CallVideoTile> = emptyList()
        private set
    /** By the page's tile id. */
    private val renderers = HashMap<String, Renderer>()
    private val shapeListeners = ArrayList<ShapeListener>()
    private val layoutListener = View.OnLayoutChangeListener { _, _, _, _, _, _, _, _, _ -> layout() }

    /** The tile marked for the picture in picture, while one is on screen. */
    val pictured: CallVideoTile?
        get() = tiles.firstOrNull { it.pictureInPicture && it.visible }

    /** The shape of the picture `tile` draws, if a frame has arrived. */
    fun shape(tile: String): CallVideoShape? = renderers[tile]?.shape

    fun renderer(tile: String): Renderer? = renderers[tile]

    /**
     * Draws into `container`, whose pixels the page's CSS pixels are `density` of, behind
     * `webView`; the tiles reported so far are laid out at once. Called as the activity's bridge
     * loads; `detach` as it is destroyed.
     */
    fun attach(container: FrameLayout, webView: View, eglContext: EglBase.Context) {
        detach()
        this.container = container
        this.webView = webView
        this.eglContext = eglContext
        density = container.resources.displayMetrics.density
        container.addOnLayoutChangeListener(layoutListener)
        apply(tiles)
    }

    fun detach() {
        val container = container ?: return
        for (renderer in renderers.values) {
            release(renderer)
        }
        renderers.clear()
        container.removeOnLayoutChangeListener(layoutListener)
        this.container = null
        webView = null
        eglContext = null
    }

    /**
     * Lays the pictures out as the page reports its holes: a renderer for each tile, made as the
     * tile first appears and released as it goes, attached to the surface's track (again on every
     * report, for a track that arrives after the tile), and placed where the page says.
     */
    fun apply(tiles: List<CallVideoTile>) {
        this.tiles = tiles
        val container = container ?: return
        val eglContext = eglContext ?: return
        val kept = tiles.map { it.tile }.toSet()
        for ((tile, renderer) in renderers.entries.toList()) {
            if (tile !in kept) {
                release(renderer)
                renderers.remove(tile)
            }
        }
        for (tile in tiles) {
            var renderer = renderers[tile.tile]
            if (renderer != null && renderer.surface != tile.surface) {
                release(renderer)
                renderers.remove(tile.tile)
                renderer = null
            }
            if (renderer == null) {
                val view = SurfaceViewRenderer(container.context)
                renderer = Renderer(tile.tile, tile.surface, view)
                view.init(eglContext, renderer)
                view.setEnableHardwareScaler(true)
                // The self preview lies over the other pictures, all of them beneath the window.
                view.setZOrderMediaOverlay(tile.mirrored)
                container.addView(view, FrameLayout.LayoutParams(0, 0))
                renderers[tile.tile] = renderer
            }
            if (renderer.track == null) {
                tracks.retainedVideoTrack(tile.surface)?.let { track ->
                    track.addSink(renderer)
                    renderer.track = track
                }
            }
            renderer.view.setMirror(tile.mirrored)
            renderer.view.setScalingType(
                if (tile.cover) RendererCommon.ScalingType.SCALE_ASPECT_FILL else RendererCommon.ScalingType.SCALE_ASPECT_FIT,
            )
            renderer.view.visibility = if (tile.visible) View.VISIBLE else View.INVISIBLE
        }
        layout()
    }

    /**
     * Places every renderer: a full-screen tile over the whole container, the rest where the page
     * put their holes, in device pixels, from where the web view lies in the container.
     */
    private fun layout() {
        val container = container ?: return
        val webView = webView ?: return
        val offset = IntArray(2).also(webView::getLocationInWindow)
        val origin = IntArray(2).also(container::getLocationInWindow)
        for (tile in tiles) {
            val renderer = renderers[tile.tile] ?: continue
            val bounds = if (tile.fullScreen) {
                Rect(0, 0, container.width, container.height)
            } else {
                val left = offset[0] - origin[0] + (tile.x * density).roundToInt()
                val top = offset[1] - origin[1] + (tile.y * density).roundToInt()
                Rect(left, top, left + (tile.width * density).roundToInt(), top + (tile.height * density).roundToInt())
            }
            val params = renderer.view.layoutParams as FrameLayout.LayoutParams
            if (params.width != bounds.width() || params.height != bounds.height() || params.leftMargin != bounds.left || params.topMargin != bounds.top) {
                params.width = bounds.width()
                params.height = bounds.height()
                params.leftMargin = bounds.left
                params.topMargin = bounds.top
                renderer.view.layoutParams = params
            }
        }
    }

    /** Where a tile's picture is drawn, in the container's pixels, for a proof. */
    fun bounds(tile: String): Rect? {
        val view = renderers[tile]?.view ?: return null
        val params = view.layoutParams as FrameLayout.LayoutParams
        return Rect(params.leftMargin, params.topMargin, params.leftMargin + params.width, params.topMargin + params.height)
    }

    private fun release(renderer: Renderer) {
        renderer.track?.let { track ->
            track.removeSink(renderer)
            tracks.releaseVideoTrack(renderer.surface, track)
        }
        renderer.track = null
        renderer.view.release()
        container?.removeView(renderer.view)
    }

    fun listenShapes(listener: ShapeListener): Subscription {
        shapeListeners.add(listener)
        return Subscription(listener)
    }
}
