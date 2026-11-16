import AVFoundation
import Foundation

/// The call's sounds, as the engine cues them (`CallObserver.playSound`), played on the call's
/// own audio session: each cue is a recording of the Aspen Sound Effects Pack, bundled as the
/// folder `aspen-sound-effects-pack` (the same files the web client plays), played by an
/// `AVAudioPlayer`, which renders into the session CallKit activated for the call (play and
/// record, voice chat), so a sound follows the call's route (receiver, loudspeaker, headset),
/// takes the call's volume, and plays with the app in the background, mixed with the voices.
///
/// Nothing here activates the session: `AVAudioPlayer.play` would, were the session inactive,
/// and CallKit's audio must be activated by CallKit alone (`CallSystem`). So a sound plays only
/// between `sessionActivated` and `sessionDeactivated` (`CallAudio.activated` and `deactivated`,
/// and an interruption's start and end), and a cue arriving before the activation (the engine
/// connecting before CallKit has the session up) waits for it, up to `activationWait`, as the web
/// client holds a sound for a speaker that has not started; one older than that is dropped, and
/// so is one that arrives while the session is inactive for good. At most `maxPlayingPerCue` of
/// one cue play at once, as the web client plays at most two of one sound; a cue past that is
/// not heard rather than queued. The engine already spaces repeats of a cue.
///
/// `play` may be called from any thread (the engine's); everything else runs on the main queue,
/// where CallKit's activation arrives too.
final class CallSounds: NSObject, AVAudioPlayerDelegate, @unchecked Sendable {
    /// How long a cue waits for the session's activation before it is dropped: the web client's
    /// `START_LIMIT_MS`, past which a sound held for a speaker would be stale.
    static let activationWait: TimeInterval = 2
    /// How many of one cue play at once: the web client's `MAX_PLAYING_PER_SOUND`.
    static let maxPlayingPerCue = 2
    /// The bundled folder the recordings are in.
    static let folder = "aspen-sound-effects-pack"

    /// A cue the layer has started playing, for the proof: the player, which is playing as it
    /// is given, and the session it plays on.
    struct Played {
        let cue: SoundCue
        let player: AVAudioPlayer
        let category: AVAudioSession.Category
        let mode: AVAudioSession.Mode
    }

    typealias Listener = (Played) -> Void

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

    private let bundle: Bundle
    /// Whether the system has the session active for the call.
    private var active = false
    /// Cues that arrived while the session was inactive, each with when, for `activationWait`.
    private var waiting: [(cue: SoundCue, since: Date)] = []
    /// The players made for each cue, at most `maxPlayingPerCue`; one that is not playing is used again.
    private var players: [SoundCue: [AVAudioPlayer]] = [:]
    private var listeners: [UUID: Listener] = [:]

    init(bundle: Bundle = .main) {
        self.bundle = bundle
    }

    /// The recording's name in the folder, without its extension: the web client's `SOUND_FILES`.
    static func file(of cue: SoundCue) -> String {
        switch cue {
        case .callJoined: return "call-joined"
        case .callLeft: return "call-left"
        case .disconnected: return "disconnected"
        case .reconnected: return "reconnected"
        case .streamStarted: return "stream-started"
        case .transferAccepted: return "transfer-accepted"
        case .transferComplete: return "transfer-complete"
        }
    }

    /// Plays `cue` on the call's session, now or once the system activates it. Any thread.
    func play(_ cue: SoundCue) {
        DispatchQueue.main.async {
            if self.active {
                self.start(cue)
            } else {
                self.waiting.append((cue, Date()))
            }
        }
    }

    /// The system activated the call's session: what waited for it within `activationWait` plays.
    func sessionActivated() {
        active = true
        let now = Date()
        let due = waiting.filter { now.timeIntervalSince($0.since) <= Self.activationWait }
        waiting.removeAll()
        for (cue, _) in due {
            start(cue)
        }
    }

    /// The system deactivated the session, or another session interrupted it: nothing plays on
    /// it from now, and a sound under way stops with it.
    func sessionDeactivated() {
        active = false
        waiting.removeAll()
        for player in players.values.joined() {
            player.stop()
        }
    }

    /// How many sounds play now.
    var playing: Int {
        players.values.joined().filter(\.isPlaying).count
    }

    /// Calls `listener` on the main queue with each cue as it starts playing, until the
    /// subscription is cancelled or released.
    func listen(_ listener: @escaping Listener) -> Subscription {
        let id = UUID()
        listeners[id] = listener
        return Subscription { [weak self] in
            self?.listeners[id] = nil
        }
    }

    private func start(_ cue: SoundCue) {
        guard let player = player(for: cue) else {
            return
        }
        player.currentTime = 0
        guard player.play() else {
            NSLog("The call's sound %@ did not start", String(describing: cue))
            return
        }
        let session = AVAudioSession.sharedInstance()
        let played = Played(cue: cue, player: player, category: session.category, mode: session.mode)
        for listener in listeners.values {
            listener(played)
        }
    }

    /// A player for `cue` that is not playing, made when every one made so far plays and there
    /// is room for another; `nil` past `maxPlayingPerCue`, or when the recording is not bundled.
    private func player(for cue: SoundCue) -> AVAudioPlayer? {
        var made = players[cue] ?? []
        if let idle = made.first(where: { !$0.isPlaying }) {
            return idle
        }
        guard made.count < Self.maxPlayingPerCue else {
            return nil
        }
        guard let url = bundle.url(forResource: Self.file(of: cue), withExtension: "wav", subdirectory: Self.folder) else {
            NSLog("The call's sound %@ is not bundled", Self.file(of: cue))
            return nil
        }
        do {
            let player = try AVAudioPlayer(contentsOf: url)
            player.delegate = self
            player.prepareToPlay()
            made.append(player)
            players[cue] = made
            return player
        } catch {
            NSLog("The call's sound %@ could not be read: %@", Self.file(of: cue), String(describing: error))
            return nil
        }
    }

    func audioPlayerDecodeErrorDidOccur(_ player: AVAudioPlayer, error: Error?) {
        NSLog("The call's sound could not be decoded: %@", String(describing: error))
    }
}
