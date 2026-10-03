import XCTest

/// The app driven as a person drives it, with real taps and finger drags in WebKit. The server
/// and account come from the environment, `ASPEN_TEST_SERVER` (entered when the app asks which
/// deployment to use, since the app bakes in none), `ASPEN_TEST_USER`, and
/// `ASPEN_TEST_PASSWORD`, which `xcodebuild test` passes on when given with a `TEST_RUNNER_`
/// prefix (`TEST_RUNNER_ASPEN_TEST_SERVER`, and so on); without them the tests skip. `ASPEN_TEST_CHANNEL` names the channel read back through ("general").
final class AppUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launch()
    }

    private var web: XCUIElement { app.webViews.firstMatch }

    private func environment(_ key: String) throws -> String {
        guard let value = ProcessInfo.processInfo.environment[key], !value.isEmpty else {
            throw XCTSkip("\(key) is not set")
        }
        return value
    }

    /// Names the deployment, then signs in, each when the app asks; an app signed in already
    /// goes on as it is.
    private func signIn() throws {
        let server = try environment("ASPEN_TEST_SERVER")
        let user = try environment("ASPEN_TEST_USER")
        let password = try environment("ASPEN_TEST_PASSWORD")
        defer { allowNotifications() }
        let deployment = web.textFields["Deployment URL"]
        if deployment.waitForExistence(timeout: 5) {
            type(server, into: deployment)
            web.buttons["Continue"].firstMatch.tap()
        }
        let username = web.textFields["Username"]
        guard username.waitForExistence(timeout: 10) else { return }
        type(user, into: username)
        type(password, into: web.secureTextFields["Password"])
        web.buttons["Sign in"].firstMatch.tap()
        XCTAssertTrue(
            username.waitForNonExistence(timeout: 15),
            "still asked to sign in: \(app.debugDescription)"
        )
    }

    /// Signed in, the app asks once whether it may notify; allowed, the phone registers for
    /// push as a person's would. The alert is the system's, so it is found on the springboard.
    private func allowNotifications() {
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = springboard.alerts.buttons["Allow"]
        if allow.waitForExistence(timeout: 5) {
            allow.tap()
        }
    }

    /// Types into a field of the web view once it has focus: a first tap in WebKit can land
    /// before the field takes it, so the field is tapped until the keyboard shows.
    private func type(_ text: String, into field: XCUIElement) {
        for _ in 0..<3 {
            field.tap()
            if app.keyboards.firstMatch.waitForExistence(timeout: 3) {
                break
            }
        }
        field.typeText(text)
    }

    func testSignsIn() throws {
        try signIn()
        // Signed in, the app shows the user's settings control at the foot of a list.
        XCTAssertTrue(web.buttons["Settings"].firstMatch.waitForExistence(timeout: 20))
    }

    /// Reading back through a channel with slow drags: each drag moves what is in view by
    /// exactly the finger's distance, and when the finger rests or lifts, nothing moves.
    func testReadingBackMovesOnlyWithTheFinger() throws {
        try signIn()
        let channel = ProcessInfo.processInfo.environment["ASPEN_TEST_CHANNEL"] ?? "general"
        // The row's label says when the channel is unread ("general, unread").
        let row = web.staticTexts.matching(
            NSPredicate(format: "label == %@ OR label BEGINSWITH %@", channel, channel + ",")
        ).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 20), "no channel \(channel)")
        row.tap()
        XCTAssertTrue(web.textViews["Message"].firstMatch.waitForExistence(timeout: 20))
        sleep(3)

        let drag: CGFloat = 180
        let slop: CGFloat = 12
        var strays: [String] = []
        for step in 0..<12 {
            guard let anchor = middleText() else {
                strays.append("step \(step): nothing to follow in view (\(lastSearch))")
                continue
            }
            let before = anchor.element.frame.minY
            let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3))
            let end = start.withOffset(CGVector(dx: 0, dy: drag))
            start.press(
                forDuration: 0.2, thenDragTo: end, withVelocity: XCUIGestureVelocity(300),
                thenHoldForDuration: 0.6)
            // iOS starts scrolling once the finger has moved past its slop, and does not count
            // that distance; the content moves by the rest, and by nothing else.
            let moved = anchor.element.frame.minY - before
            if moved > drag + 1 || moved < drag - slop - 1 {
                strays.append("step \(step): \"\(anchor.label)\" moved \(moved), not \(drag)")
            }
            // Lifted after resting, the finger leaves the list at rest: nothing moves now,
            // pictures settling into their sizes included.
            let rested = anchor.element.frame.minY
            sleep(1)
            let settled = anchor.element.frame.minY - rested
            if abs(settled) > 1 {
                strays.append("step \(step): \"\(anchor.label)\" moved \(settled) at rest")
            }
        }
        XCTAssertEqual(strays, [], strays.joined(separator: "\n"))
    }

    /// Reading back as a person does: a drag the finger lifts from almost at once, with the next
    /// beginning as soon as the test can, and flicks that coast. iOS scrolls in a process of its
    /// own, so a page the list shows while it thinks the list is at rest may land as the next
    /// drag begins there, and a view that jumps shows as a scroll of more than a screen, which
    /// the list counts when built with `VITE_SCROLL_DEBUG=1` (`scrollDiagnostics.ts`), and as
    /// the text followed leaving the screen or moving by other than the finger's distance.
    func testReadingBackQuicklyNeverJumps() throws {
        try signIn()
        let channel = ProcessInfo.processInfo.environment["ASPEN_TEST_CHANNEL"] ?? "general"
        // The row's label says when the channel is unread ("general, unread").
        let row = web.staticTexts.matching(
            NSPredicate(format: "label == %@ OR label BEGINSWITH %@", channel, channel + ",")
        ).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 20), "no channel \(channel)")
        row.tap()
        XCTAssertTrue(web.textViews["Message"].firstMatch.waitForExistence(timeout: 20))
        sleep(3)
        let counter = web.staticTexts.matching(
            NSPredicate(format: "label BEGINSWITH 'scroll-diagnostics'")
        ).firstMatch
        XCTAssertTrue(counter.waitForExistence(timeout: 5), "built without VITE_SCROLL_DEBUG=1")

        // The list's own count is the measure here: following something in view walks the
        // whole accessibility tree, far too slowly to keep up with a reader's pace, and the
        // jumps this reading provokes happen while the list moves, where frames show nothing.
        let drag: CGFloat = 180
        for step in 0..<48 {
            let flick = step % 4 == 3
            let start = app.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.3))
            let end = start.withOffset(CGVector(dx: 0, dy: flick ? drag * 2 : drag))
            start.press(
                forDuration: 0.05, thenDragTo: end,
                withVelocity: XCUIGestureVelocity(flick ? 2500 : 600),
                thenHoldForDuration: flick ? 0 : 0.1)
            if flick {
                sleep(3)
            }
        }
        let jumps = counter.label
        let log = web.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'scroll-jump-log'"))
            .firstMatch
        let around = log.exists ? log.label : "(no jump log)"
        XCTAssertEqual(
            jumps, "scroll-diagnostics jumps 0", "the list counted jumps: \(jumps)\n\(around)")
    }

    /// Opening a voice channel (`ASPEN_TEST_VOICE_CHANNEL`, "probe-voice") joins its call, with
    /// the microphone the simulator takes from the Mac: the call bar comes up, and Leave call
    /// takes it down. A call left over from an earlier run is left first, then joined again.
    func testJoinsAndLeavesACall() throws {
        try signIn()
        let channel = ProcessInfo.processInfo.environment["ASPEN_TEST_VOICE_CHANNEL"] ?? "probe-voice"
        let row = web.staticTexts.matching(
            NSPredicate(format: "label == %@ OR label BEGINSWITH %@", channel, channel + ",")
        ).firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 20), "no channel \(channel)")
        // WebKit names a landmark by its label and its role ("Your call, region").
        let bar = web.otherElements.matching(NSPredicate(format: "label BEGINSWITH %@", "Your call"))
            .firstMatch
        let leave = web.buttons["Leave call"].firstMatch
        let join = web.buttons["Join \(channel)"].firstMatch
        if bar.exists {
            leave.tap()
            XCTAssertTrue(bar.waitForNonExistence(timeout: 20), "the earlier call's bar stayed")
        }
        row.tap()
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let allow = springboard.alerts.buttons["Allow"]
        if allow.waitForExistence(timeout: 5) {
            allow.tap()
        }
        // A tap on the row joins; where the screen offers the way in instead, it is taken.
        if !bar.waitForExistence(timeout: 10) && join.exists {
            join.tap()
        }
        XCTAssertTrue(bar.waitForExistence(timeout: 40), "no call bar: \(app.debugDescription)")
        leave.tap()
        XCTAssertTrue(bar.waitForNonExistence(timeout: 20), "the call bar stayed")
    }

    /// A notification of a message, delivered before the test (`xcrun simctl push` with what
    /// the relay would send; `ASPEN_TEST_NOTIFICATION` is text the notification shows, the
    /// message's), opens that message in the app when tapped in Notification Center.
    func testOpensATappedNotification() throws {
        let text = try environment("ASPEN_TEST_NOTIFICATION")
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        XCUIDevice.shared.press(.home)
        let top = springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.01))
        top.press(
            forDuration: 0.1,
            thenDragTo: springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.7))
        )
        let notification = springboard.descendants(matching: .any)
            .matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
        XCTAssertTrue(notification.waitForExistence(timeout: 10), "no notification saying \(text)")
        notification.tap()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15), "the app did not open")
        let shown = web.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
        XCTAssertTrue(shown.waitForExistence(timeout: 30), "the message is not shown: \(app.debugDescription)")
    }

    /// What the last anchor search saw, for a stray that says nothing was found.
    private var lastSearch = ""

    /// Something in view, between the top and the message box, that only one element shows:
    /// a line of text, or a picture by its description. Every property read is a query of the
    /// web view, slow enough to matter, so the elements are walked from the newest, since the
    /// rows in view sit near the end of the list while every page of older history read so far
    /// lies before them (what follows the rows are a few overlays, placed anywhere), and a
    /// row's place is read before anything else about it.
    private func middleText() -> (label: String, element: XCUIElement)? {
        let screen = app.windows.firstMatch.frame
        let band = screen.minY + screen.height * 0.15...screen.minY + screen.height * 0.75
        var seen = 0
        var inBand = 0
        for kind in [XCUIElement.ElementType.staticText, .image] {
            let all = web.descendants(matching: kind)
            for element in all.allElementsBoundByIndex.reversed() {
                seen += 1
                guard band.contains(element.frame.midY) else { continue }
                inBand += 1
                let label = element.label
                guard label.count > 4 else { continue }
                let same = all.matching(NSPredicate(format: "label == %@", label))
                if same.count == 1 {
                    return (label, same.firstMatch)
                }
            }
        }
        lastSearch = "window \(screen), \(seen) read, \(inBand) in band"
        return nil
    }
}
