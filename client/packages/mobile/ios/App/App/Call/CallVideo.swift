import AVFoundation
import AVKit
import CoreMedia
import Foundation
import UIKit
import WebRTC

/// A hole the page reports for a picture (`TileReport` in the app's `nativeCall.ts`): which
/// surface (a track id) it shows, where, in points of the web view, and how the picture is drawn
/// in it. `tile` is the page's id for the hole, unique for the page's life; two tiles may show
/// one surface.
struct CallVideoTile: Equatable {
    let tile: String
    let surface: TrackId
    let rect: CGRect
    let visible: Bool
    /// The self preview, shown as a mirror would while the camera faces its user.
    let mirrored: Bool
    /// Cropped to fill the hole, rather than fitted whole within it.
    let cover: Bool
    /// The tile fills the screen: the picture takes the container's bounds, which it has before
    /// the page's layout after a turn.
    let fullScreen: Bool
    /// The one picture the system's picture in picture shows.
    let pictureInPicture: Bool
}

/// The shape of a surface's picture as drawn, upright: a phone's camera held upright is taller
/// than wide. `rotation` is the sender's, in degrees clockwise, already applied to the size.
struct CallVideoShape: Equatable {
    let surface: TrackId
    let width: Int
    let height: Int
    let rotation: Int
}

/// Draws a video track into an `AVSampleBufferDisplayLayer`: each frame WebRTC renders is
/// wrapped as a sample buffer shown at once. The camera's frames and the hardware decoder's
/// arrive as NV12 pixel buffers and are shown as they are; a software decoder's I420 planes are
/// written into an NV12 buffer from a pool. A frame's rotation (the camera's own frames carry
/// the phone's; received frames arrive upright, the sender having turned them) is applied as the
/// layer's transform, as is the mirror of a self preview, so the layer is laid out in the tile's
/// box turned to fit. A sample buffer layer rather than Metal, since the system's picture in
/// picture draws from one (`CallPictureInPicture`).
final class SampleBufferRenderer: NSObject, RTCVideoRenderer {
    let surface: TrackId
    let layer = AVSampleBufferDisplayLayer()
    private let lock = NSLock()
    private var pool: CVPixelBufferPool?
    private var poolSize = (width: 0, height: 0)
    private var shape: CallVideoShape?
    /// The frame rotation the layer is turned by now, in degrees clockwise.
    private var rotation = 0
    private var mirrored = false
    private var box = CGRect.zero
    /// The track the layer draws, held so the renderer can be taken off it; weak, so a track
    /// gone with its connection leaves the renderer free to draw one of the same id again.
    weak var track: RTCVideoTrack?
    /// Told on the main queue of the picture's shape, as it first arrives and whenever it changes.
    var onShape: ((CallVideoShape) -> Void)?
    /// How many frames have been drawn; a proof reads it.
    private(set) var framesRendered = 0

    init(surface: TrackId) {
        self.surface = surface
        super.init()
        layer.videoGravity = .resizeAspect
        layer.backgroundColor = UIColor.black.cgColor
    }

    // MARK: Layout

    /// Lays the layer out to fill `box`, turned by the latest frame's rotation and mirrored where
    /// asked, drawing the picture `cover`ed or fitted. Main queue.
    func place(in box: CGRect, cover: Bool, mirrored: Bool) {
        self.box = box
        self.mirrored = mirrored
        layer.videoGravity = cover ? .resizeAspectFill : .resizeAspect
        relayout()
    }

    private func relayout() {
        let turned = rotation == 90 || rotation == 270
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        layer.bounds = CGRect(
            x: 0, y: 0, width: turned ? box.height : box.width, height: turned ? box.width : box.height)
        layer.position = CGPoint(x: box.midX, y: box.midY)
        var transform = CATransform3DMakeRotation(CGFloat(rotation) * .pi / 180, 0, 0, 1)
        if mirrored {
            transform = CATransform3DScale(transform, -1, 1, 1)
        }
        layer.transform = transform
        CATransaction.commit()
    }

    // MARK: RTCVideoRenderer

    func setSize(_ size: CGSize) {}

    func renderFrame(_ frame: RTCVideoFrame?) {
        guard let frame, let pixelBuffer = pixelBuffer(of: frame) else {
            return
        }
        var description: CMVideoFormatDescription?
        guard CMVideoFormatDescriptionCreateForImageBuffer(
            allocator: kCFAllocatorDefault, imageBuffer: pixelBuffer, formatDescriptionOut: &description) == noErr,
            let description
        else {
            return
        }
        var timing = CMSampleTimingInfo(
            duration: .invalid, presentationTimeStamp: CMClockGetTime(CMClockGetHostTimeClock()), decodeTimeStamp: .invalid)
        var sample: CMSampleBuffer?
        guard CMSampleBufferCreateReadyWithImageBuffer(
            allocator: kCFAllocatorDefault, imageBuffer: pixelBuffer, formatDescription: description,
            sampleTiming: &timing, sampleBufferOut: &sample) == noErr, let sample
        else {
            return
        }
        // Shown as it arrives: WebRTC already paced the frame to its sound.
        if let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: true) as? [NSMutableDictionary],
           let first = attachments.first {
            first[kCMSampleAttachmentKey_DisplayImmediately] = true
        }
        if layer.status == .failed {
            layer.flush()
        }
        layer.enqueue(sample)
        lock.lock()
        framesRendered += 1
        let degrees = Int(frame.rotation.rawValue)
        let turned = degrees == 90 || degrees == 270
        let seen = CallVideoShape(
            surface: surface, width: turned ? Int(frame.height) : Int(frame.width),
            height: turned ? Int(frame.width) : Int(frame.height), rotation: degrees)
        let changed = seen != shape
        shape = seen
        lock.unlock()
        if changed {
            DispatchQueue.main.async { [self] in
                if rotation != degrees {
                    rotation = degrees
                    relayout()
                }
                onShape?(seen)
            }
        }
    }

    /// The frame's pixels as a buffer the display layer draws: the frame's own where it is one
    /// already and whole, otherwise its I420 planes written as NV12.
    private func pixelBuffer(of frame: RTCVideoFrame) -> CVPixelBuffer? {
        if let buffer = frame.buffer as? RTCCVPixelBuffer, !buffer.requiresCropping() {
            let format = CVPixelBufferGetPixelFormatType(buffer.pixelBuffer)
            if Self.displayable.contains(format) {
                return buffer.pixelBuffer
            }
        }
        let i420 = frame.buffer.toI420()
        let width = Int(i420.width)
        let height = Int(i420.height)
        guard let pool = pool(width: width, height: height) else {
            return nil
        }
        var made: CVPixelBuffer?
        guard CVPixelBufferPoolCreatePixelBuffer(kCFAllocatorDefault, pool, &made) == kCVReturnSuccess, let made else {
            return nil
        }
        CVPixelBufferLockBaseAddress(made, [])
        defer { CVPixelBufferUnlockBaseAddress(made, []) }
        if let y = CVPixelBufferGetBaseAddressOfPlane(made, 0) {
            let stride = CVPixelBufferGetBytesPerRowOfPlane(made, 0)
            for row in 0..<height {
                memcpy(y + row * stride, i420.dataY + row * Int(i420.strideY), width)
            }
        }
        if let uv = CVPixelBufferGetBaseAddressOfPlane(made, 1) {
            let stride = CVPixelBufferGetBytesPerRowOfPlane(made, 1)
            let chromaWidth = Int(i420.chromaWidth)
            let chromaHeight = Int(i420.chromaHeight)
            for row in 0..<chromaHeight {
                let u = i420.dataU + row * Int(i420.strideU)
                let v = i420.dataV + row * Int(i420.strideV)
                let out = (uv + row * stride).assumingMemoryBound(to: UInt8.self)
                for x in 0..<chromaWidth {
                    out[2 * x] = u[x]
                    out[2 * x + 1] = v[x]
                }
            }
        }
        return made
    }

    /// The pixel formats a sample buffer layer shows without conversion: the camera's and the
    /// hardware decoder's NV12, and BGRA.
    private static let displayable: Set<OSType> = [
        kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
        kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
        kCVPixelFormatType_32BGRA,
    ]

    private func pool(width: Int, height: Int) -> CVPixelBufferPool? {
        if let pool, poolSize == (width, height) {
            return pool
        }
        let attributes: [CFString: Any] = [
            kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
            kCVPixelBufferWidthKey: width,
            kCVPixelBufferHeightKey: height,
            kCVPixelBufferIOSurfacePropertiesKey: [:] as [CFString: Any],
        ]
        var made: CVPixelBufferPool?
        CVPixelBufferPoolCreate(kCFAllocatorDefault, nil, attributes as CFDictionary, &made)
        pool = made
        poolSize = (width, height)
        return made
    }

    /// The shape last drawn, if any frame has been.
    var lastShape: CallVideoShape? {
        lock.lock()
        defer { lock.unlock() }
        return shape
    }
}

/// The system's picture in picture of one renderer's layer (`AVPictureInPictureController` on a
/// sample buffer layer): started by the system when the app leaves the screen while the layer
/// is on it, and ended by the system or the user. The content is live, so it has no timeline to
/// seek and is never paused.
final class CallPictureInPicture: NSObject, AVPictureInPictureControllerDelegate,
    AVPictureInPictureSampleBufferPlaybackDelegate
{
    private(set) var controller: AVPictureInPictureController?
    private(set) var renderer: SampleBufferRenderer?
    /// Told on the main queue as the system starts and stops showing the picture.
    var onActive: ((Bool) -> Void)?

    static var isSupported: Bool {
        AVPictureInPictureController.isPictureInPictureSupported()
    }

    /// Shows `renderer`'s picture, or none, as the one the system may take into its picture in
    /// picture. Main queue.
    func show(_ renderer: SampleBufferRenderer?) {
        guard renderer !== self.renderer else {
            return
        }
        self.renderer = renderer
        guard let renderer, Self.isSupported else {
            controller?.stopPictureInPicture()
            controller = nil
            return
        }
        let source = AVPictureInPictureController.ContentSource(
            sampleBufferDisplayLayer: renderer.layer, playbackDelegate: self)
        if let controller {
            controller.contentSource = source
        } else {
            let made = AVPictureInPictureController(contentSource: source)
            made.delegate = self
            made.canStartPictureInPictureAutomaticallyFromInline = true
            made.requiresLinearPlayback = true
            controller = made
        }
    }

    var isPossible: Bool {
        controller?.isPictureInPicturePossible ?? false
    }

    var isActive: Bool {
        controller?.isPictureInPictureActive ?? false
    }

    func start() {
        controller?.startPictureInPicture()
    }

    func stop() {
        controller?.stopPictureInPicture()
    }

    // MARK: AVPictureInPictureControllerDelegate

    func pictureInPictureControllerDidStartPictureInPicture(_ controller: AVPictureInPictureController) {
        onActive?(true)
    }

    func pictureInPictureControllerDidStopPictureInPicture(_ controller: AVPictureInPictureController) {
        onActive?(false)
    }

    func pictureInPictureController(
        _ controller: AVPictureInPictureController, failedToStartPictureInPictureWithError error: Error
    ) {
        NSLog("Aspen could not start the picture in picture: %@", String(describing: error))
        onActive?(false)
    }

    // MARK: AVPictureInPictureSampleBufferPlaybackDelegate

    func pictureInPictureController(_ controller: AVPictureInPictureController, setPlaying playing: Bool) {}

    func pictureInPictureControllerTimeRangeForPlayback(_ controller: AVPictureInPictureController) -> CMTimeRange {
        CMTimeRange(start: .negativeInfinity, duration: .positiveInfinity)
    }

    func pictureInPictureControllerIsPlaybackPaused(_ controller: AVPictureInPictureController) -> Bool {
        false
    }

    func pictureInPictureController(
        _ controller: AVPictureInPictureController, didTransitionToRenderSize newRenderSize: CMVideoDimensions
    ) {}

    func pictureInPictureController(
        _ controller: AVPictureInPictureController, skipByInterval skipInterval: CMTime
    ) async {}
}

/// Where the call's pictures are drawn: a view beneath the web view (`RootViewController`),
/// holding one renderer per hole the page reports (`apply`), each laid out where the page says
/// its hole is and drawing the surface's track from `CallTracks`. The page draws names, buttons,
/// and dialogs over them through its transparent web view; nothing is ever drawn above it. Self
/// previews (mirrored tiles) lie above the other pictures. A full-screen tile takes the view's
/// own bounds, which it has before the page's layout after a turn. Everything runs on the main
/// queue; the renderers' frames arrive on WebRTC's threads.
final class CallVideo {
    /// The container, black so a hole shows black until its first frame, as a `<video>` with
    /// nothing to play is.
    final class ContainerView: UIView {
        var onLayout: (() -> Void)?

        override func layoutSubviews() {
            super.layoutSubviews()
            onLayout?()
        }
    }

    typealias ShapeListener = (CallVideoShape) -> Void
    typealias PictureInPictureListener = (Bool) -> Void

    /// Stops the listener it was given for.
    final class Subscription {
        private let stop: () -> Void

        fileprivate init(stop: @escaping () -> Void) {
            self.stop = stop
        }

        func cancel() {
            stop()
        }

        deinit {
            stop()
        }
    }

    let view = ContainerView()
    /// The web view the page's rectangles are in, set as the root view controller is made.
    weak var webView: UIView?
    private let tracks: CallTracks
    private(set) var tiles: [CallVideoTile] = []
    /// By the page's tile id.
    private(set) var renderers: [String: SampleBufferRenderer] = [:]
    let pictureInPicture = CallPictureInPicture()
    private var shapeListeners: [UUID: ShapeListener] = [:]
    private var pictureInPictureListeners: [UUID: PictureInPictureListener] = [:]

    init(tracks: CallTracks) {
        self.tracks = tracks
        view.backgroundColor = .black
        view.isUserInteractionEnabled = false
        view.onLayout = { [weak self] in self?.layout() }
        pictureInPicture.onActive = { [weak self] active in
            guard let self else {
                return
            }
            for listener in pictureInPictureListeners.values {
                listener(active)
            }
        }
    }

    // MARK: Tiles

    /// Lays the pictures out as the page reports its holes: a renderer for each tile, made as the
    /// tile first appears and taken off its track as it goes, attached to the surface's track
    /// (again on every report, for a track that arrives after the tile), placed where the page
    /// says, and the one marked for it offered to the picture in picture.
    func apply(_ tiles: [CallVideoTile]) {
        self.tiles = tiles
        let kept = Set(tiles.map(\.tile))
        for (tile, renderer) in renderers where !kept.contains(tile) {
            detach(renderer)
            renderer.layer.removeFromSuperlayer()
            renderers[tile] = nil
        }
        for tile in tiles {
            let renderer: SampleBufferRenderer
            if let held = renderers[tile.tile], held.surface == tile.surface {
                renderer = held
            } else {
                if let held = renderers[tile.tile] {
                    detach(held)
                    held.layer.removeFromSuperlayer()
                }
                renderer = SampleBufferRenderer(surface: tile.surface)
                renderer.onShape = { [weak self] shape in self?.shaped(shape) }
                renderers[tile.tile] = renderer
                view.layer.addSublayer(renderer.layer)
            }
            if renderer.track == nil, let track = tracks.track(tile.surface) as? RTCVideoTrack {
                track.add(renderer)
                renderer.track = track
            }
            renderer.layer.isHidden = !tile.visible
        }
        // Self previews over the rest.
        for tile in tiles where tile.mirrored {
            if let renderer = renderers[tile.tile] {
                renderer.layer.removeFromSuperlayer()
                view.layer.addSublayer(renderer.layer)
            }
        }
        layout()
        let pictured = tiles.first { $0.pictureInPicture && $0.visible }.flatMap { renderers[$0.tile] }
        pictureInPicture.show(pictured)
    }

    /// Places every renderer: a full-screen tile over the whole view, the rest where the page
    /// put their holes, converted from the web view's coordinates.
    private func layout() {
        for tile in tiles {
            guard let renderer = renderers[tile.tile] else {
                continue
            }
            let box: CGRect
            if tile.fullScreen {
                box = view.bounds
            } else if let webView {
                box = view.convert(tile.rect, from: webView)
            } else {
                box = tile.rect
            }
            renderer.place(in: box, cover: tile.cover, mirrored: tile.mirrored)
        }
    }

    private func detach(_ renderer: SampleBufferRenderer) {
        renderer.track?.remove(renderer)
        renderer.track = nil
    }

    /// The frame of the picture a tile is drawn in, for a proof.
    func frame(ofTile tile: String) -> CGRect? {
        renderers[tile]?.layer.frame
    }

    // MARK: Listeners

    private func shaped(_ shape: CallVideoShape) {
        for listener in shapeListeners.values {
            listener(shape)
        }
    }

    func listenShapes(_ listener: @escaping ShapeListener) -> Subscription {
        let id = UUID()
        shapeListeners[id] = listener
        return Subscription { [weak self] in self?.shapeListeners[id] = nil }
    }

    func listenPictureInPicture(_ listener: @escaping PictureInPictureListener) -> Subscription {
        let id = UUID()
        pictureInPictureListeners[id] = listener
        return Subscription { [weak self] in self?.pictureInPictureListeners[id] = nil }
    }
}
