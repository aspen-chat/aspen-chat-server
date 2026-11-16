import AVFoundation
import XCTest
@testable import App

/// The call's routes on the simulator, which plays through the Mac and so shows one output, its
/// loudspeaker, with no receiver port behind the iPhone's idiom and no headset; the kinds each
/// port type is read as; and the proximity sensor following the receiver, whose path a choice of
/// it runs here though the simulator's route never becomes the receiver.
final class CallRoutesTests: XCTestCase {
    func testKindsFollowPortTypes() {
        XCTAssertEqual(CallRoutes.kind(of: .builtInReceiver), .receiver)
        XCTAssertEqual(CallRoutes.kind(of: .builtInSpeaker), .loudspeaker)
        XCTAssertEqual(CallRoutes.kind(of: .bluetoothHFP), .bluetooth)
        XCTAssertEqual(CallRoutes.kind(of: .bluetoothA2DP), .bluetooth)
        XCTAssertEqual(CallRoutes.kind(of: .bluetoothLE), .bluetooth)
        XCTAssertEqual(CallRoutes.kind(of: .headphones), .wiredHeadset)
        XCTAssertEqual(CallRoutes.kind(of: .headsetMic), .wiredHeadset)
        XCTAssertEqual(CallRoutes.kind(of: .carAudio), .car)
        XCTAssertEqual(CallRoutes.kind(of: .usbAudio), .other)
        XCTAssertEqual(CallRoutes.kind(of: .airPlay), .other)
    }

    func testProximityWantsTheReceiverAlone() {
        func route(_ kind: CallRouteKind, inUse: Bool) -> CallRoute {
            CallRoute(kind: kind, name: nil, uid: nil, inUse: inUse)
        }
        XCTAssertTrue(CallRoutes.proximityWanted([route(.receiver, inUse: true), route(.loudspeaker, inUse: false)]))
        XCTAssertFalse(CallRoutes.proximityWanted([route(.receiver, inUse: false), route(.loudspeaker, inUse: true)]))
        XCTAssertFalse(CallRoutes.proximityWanted([route(.receiver, inUse: false), route(.bluetooth, inUse: true)]))
        XCTAssertFalse(CallRoutes.proximityWanted([]))
    }

    @MainActor
    func testListsTheSimulatorsLoudspeakerAndHoldsAChoice() throws {
        var sensor: [Bool] = []
        let routes = CallRoutes(proximity: { sensor.append($0) })
        let listed = routes.current
        XCTAssertTrue(listed.hasReceiver, "an iPhone has a receiver")
        XCTAssertTrue(listed.routes.contains { $0.kind == .receiver && !$0.inUse }, "\(listed.routes)")
        let loudspeaker = try XCTUnwrap(listed.routes.first { $0.kind == .loudspeaker }, "\(listed.routes)")
        XCTAssertTrue(loudspeaker.inUse, "the simulator plays through its loudspeaker: \(listed.routes)")
        XCTAssertEqual(listed.routes.filter(\.inUse).count, 1, "one route plays: \(listed.routes)")
        XCTAssertNil(listed.chosen)
        XCTAssertFalse(routes.proximityOn)

        // A choice stands until reset: the loudspeaker, then the receiver, which the simulator
        // has no port for, so the route in use stays the loudspeaker and the sensor stays off.
        try AVAudioSession.sharedInstance().setCategory(.playAndRecord, mode: .voiceChat, options: [.allowBluetoothHFP])
        routes.select(kind: .loudspeaker)
        XCTAssertEqual(routes.current.chosen, .loudspeaker)
        routes.select(kind: .receiver)
        XCTAssertEqual(routes.current.chosen, .receiver)
        XCTAssertTrue(routes.current.routes.contains { $0.kind == .loudspeaker && $0.inUse }, "\(routes.current.routes)")
        XCTAssertFalse(routes.proximityOn)
        XCTAssertEqual(sensor, [], "the sensor is asked for only when the route in use changes to or from the receiver")
        routes.reset()
        XCTAssertNil(routes.current.chosen)
        print("call-routes-test: the simulator lists \(routes.current.routes.map { "\($0.kind.rawValue)\($0.inUse ? " (in use)" : "")\($0.name.map { " \"\($0)\"" } ?? "")" }); receiver \(routes.current.hasReceiver); proximity \(routes.proximityOn)")
    }
}
