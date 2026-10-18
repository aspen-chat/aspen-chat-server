import XCTest

/// The app driven as a person drives it, with real taps and finger drags in WebKit. The server
/// and account come from the environment, `ASPEN_TEST_SERVER` (entered when the app asks which
/// deployment to use, since the app bakes in none), `ASPEN_TEST_USER`, and
/// `ASPEN_TEST_PASSWORD`, which `xcodebuild test` passes on when given with a `TEST_RUNNER_`
/// prefix (`TEST_RUNNER_ASPEN_TEST_SERVER`, and so on); without them the tests skip.
/// `scripts/ui_tests.py` gives every variable the tests read (`docs/architecture/ios-app.md`
/// in `client/` lists them), and fails a run in which any test skips.
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

    /// Types into a field of the web view once it is there and has focus: a first tap in WebKit
    /// can land before the field takes it, so the field is tapped until the keyboard shows.
    private func type(_ text: String, into field: XCUIElement) {
        XCTAssertTrue(field.waitForExistence(timeout: 10), "no field \(field.description)")
        for _ in 0..<3 {
            field.tap()
            if app.keyboards.firstMatch.waitForExistence(timeout: 3) {
                break
            }
        }
        field.typeText(text)
    }

    /// The channel read back through and measured, with history enough for it.
    private var readBackChannel: String {
        ProcessInfo.processInfo.environment["ASPEN_TEST_CHANNEL"] ?? "general"
    }

    /// The channel the tests write in, so what they write never lengthens the one read back.
    private var scratchChannel: String {
        ProcessInfo.processInfo.environment["ASPEN_TEST_SCRATCH_CHANNEL"] ?? "scratch"
    }

    private var composer: XCUIElement { web.textViews["Message"].firstMatch }

    /// The channel list's row for `name`, whose label says when it is unread ("general, unread").
    private func channelRow(_ name: String) -> XCUIElement {
        web.staticTexts.matching(
            NSPredicate(format: "label == %@ OR label BEGINSWITH %@", name, name + ",")
        ).firstMatch
    }

    /// Opens the text channel `name` from the channel list, once its message box is up.
    private func openChannel(_ name: String) {
        let row = channelRow(name)
        XCTAssertTrue(row.waitForExistence(timeout: 20), "no channel \(name)")
        row.tap()
        XCTAssertTrue(composer.waitForExistence(timeout: 20), "no message box in \(name)")
    }

    /// Text in the web view that holds `text`.
    private func shown(_ text: String) -> XCUIElement {
        web.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
    }

    /// The control labelled `label`, or whose label begins with it (`prefix`), within `root` or
    /// else the web view, of one of `kinds`. WebKit gives a toggle button (`aria-pressed`) as a
    /// switch, and a menu's trigger (`aria-haspopup`) as no control at all, so finding one takes
    /// `.other` too, which other elements labelled alike would also match.
    private func control(
        _ label: String, prefix: Bool = false, in root: XCUIElement? = nil,
        kinds: [XCUIElement.ElementType] = [.button, .switch, .toggle, .menuItem]
    ) -> XCUIElement {
        let predicate = NSPredicate(
            format: prefix ? "label BEGINSWITH %@ AND elementType IN %@" : "label == %@ AND elementType IN %@",
            label, kinds.map(\.rawValue))
        return (root ?? web).descendants(matching: .any).matching(predicate).firstMatch
    }

    /// Asks the helper `ui_tests.py` runs beside the tests (`ASPEN_TEST_HELPER`) to `action`
    /// (`say`, `away`, `notify`), and answers with the text it had written.
    private func helper(_ action: String) throws -> String {
        let base = try environment("ASPEN_TEST_HELPER")
        var request = URLRequest(url: try XCTUnwrap(URL(string: "\(base)/\(action)")))
        request.httpMethod = "POST"
        request.timeoutInterval = 300
        let done = DispatchSemaphore(value: 0)
        var answer: (data: Data?, error: Error?) = (nil, nil)
        URLSession.shared.dataTask(with: request) { data, _, error in
            answer = (data, error)
            done.signal()
        }.resume()
        done.wait()
        let data = try XCTUnwrap(answer.data, "the helper did not answer \(action): \(String(describing: answer.error))")
        let body = try XCTUnwrap(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        return try XCTUnwrap(body["text"] as? String, "the helper could not \(action): \(body["error"] ?? body)")
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
        openChannel(readBackChannel)
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
        openChannel(readBackChannel)
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
        let row = channelRow(channel)
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

    /// A notification of a message, tapped, opens that message in the app. Once the reader has
    /// left the app, the helper (`notify`) has someone message them, checks that the push woke
    /// the app's own subscription, and shows the notification as the service extension would:
    /// the simulator never runs the extension. It is tapped as its banner comes down.
    func testOpensATappedNotification() throws {
        try signIn()
        XCTAssertTrue(web.buttons["Settings"].firstMatch.waitForExistence(timeout: 20))
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        XCUIDevice.shared.press(.home)
        let text = try helper("notify")
        let banner = springboard.descendants(matching: .any)
            .matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
        XCTAssertTrue(banner.waitForExistence(timeout: 10), "no notification saying \(text)")
        banner.tap()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 15), "the app did not open")
        XCTAssertTrue(shown(text).waitForExistence(timeout: 30), "the message is not shown: \(app.debugDescription)")
    }

    /// Writing with the keyboard on the screen: the message box stays above the keyboard while
    /// it is up, and what is sent shows in the channel and leaves the box empty.
    func testSendsAMessageFromTheKeyboard() throws {
        try signIn()
        openChannel(scratchChannel)
        let text = "Typed on the keyboard \(UUID().uuidString.prefix(6))"
        type(text, into: composer)
        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.exists, "no keyboard came up")
        XCTAssertLessThanOrEqual(
            composer.frame.maxY, keyboard.frame.minY + 1,
            "the message box \(composer.frame) is under the keyboard \(keyboard.frame)")
        web.buttons["Send"].firstMatch.tap()
        XCTAssertTrue(shown(text).waitForExistence(timeout: 20), "the message did not show")
        XCTAssertFalse((composer.value as? String ?? "").contains(text), "the box kept what was sent")
    }

    /// Back from the background, the app shows what was written while it was away without being
    /// reloaded. The helper (`away`) freezes it as iOS suspends a phone's, which iOS does not do
    /// to an app a test drives, until the deployment has dropped its silent connection.
    func testCatchesUpAfterTheBackground() throws {
        try signIn()
        openChannel(scratchChannel)
        XCUIDevice.shared.press(.home)
        let text = try helper("away")
        app.activate()
        XCTAssertTrue(shown(text).waitForExistence(timeout: 30), "what was said while away did not show")
    }

    /// An invite link (`ASPEN_TEST_INVITE`, `aspen://app/invite/…`) opened from outside the app
    /// shows the invite to its community (`ASPEN_TEST_INVITE_COMMUNITY`) in the app, ready to be
    /// joined; nothing is joined.
    func testOpensAnInviteLink() throws {
        let link = try environment("ASPEN_TEST_INVITE")
        let community = try environment("ASPEN_TEST_INVITE_COMMUNITY")
        try signIn()
        XCTAssertTrue(web.buttons["Settings"].firstMatch.waitForExistence(timeout: 20))
        guard #available(iOS 16.4, *) else {
            throw XCTSkip("opening a link in the app needs iOS 16.4")
        }
        app.open(try XCTUnwrap(URL(string: link)))
        XCTAssertTrue(
            shown("You have been invited to \(community)").waitForExistence(timeout: 20),
            "no invite: \(app.debugDescription)")
        XCTAssertTrue(web.buttons["Join"].firstMatch.exists, "the invite offers no Join")
    }

    /// A finger held on a message opens its actions in a sheet; a quick reaction there adds that
    /// reaction and closes the sheet, and the reaction, tapped, is taken back.
    func testReactsFromALongPress() throws {
        try signIn()
        openChannel(scratchChannel)
        let text = try helper("say")
        let message = shown(text)
        XCTAssertTrue(message.waitForExistence(timeout: 20), "the message to react to did not show")
        message.press(forDuration: 1.2)
        let sheet = web.otherElements.matching(NSPredicate(format: "label BEGINSWITH %@", "Message actions"))
            .firstMatch
        XCTAssertTrue(sheet.waitForExistence(timeout: 10), "no actions: \(app.debugDescription)")
        let quick = control("React with ", prefix: true, in: sheet)
        XCTAssertTrue(quick.waitForExistence(timeout: 5), "no quick reactions: \(sheet.debugDescription)")
        let emoji = String(quick.label.dropFirst("React with ".count))
        quick.tap()
        XCTAssertTrue(sheet.waitForNonExistence(timeout: 10), "the sheet stayed")
        let mine = control("Remove your \(emoji)")
        XCTAssertTrue(mine.waitForExistence(timeout: 10), "no \(emoji) of the reader's on the message")
        mine.tap()
        XCTAssertTrue(mine.waitForNonExistence(timeout: 10), "the reaction stayed")
    }

    /// A picture chosen in the system's photo picker, which the message box's file input opens,
    /// is described, sent, and shown in the channel by its description. `ui_tests.py` puts a
    /// picture in the simulator's library.
    func testSendsAPictureFromThePhotoLibrary() throws {
        try signIn()
        openChannel(scratchChannel)
        control("Add a file or a poll", kinds: [.button, .other]).tap()
        let attach = control("Attach a file")
        XCTAssertTrue(attach.waitForExistence(timeout: 10), "no Attach a file")
        attach.tap()
        // iOS asks where the file comes from, then offers the library in a picker of its own.
        let library = app.buttons["Photo Library"].firstMatch
        XCTAssertTrue(library.waitForExistence(timeout: 10), "no choice of source: \(app.debugDescription)")
        library.tap()
        let photo = app.images.firstMatch
        XCTAssertTrue(photo.waitForExistence(timeout: 20), "no photos offered: \(app.debugDescription)")
        photo.tap()
        let add = app.buttons["Add"].firstMatch
        if add.waitForExistence(timeout: 3) {
            add.tap()
        }
        let describe = control("Describe ", prefix: true)
        XCTAssertTrue(describe.waitForExistence(timeout: 20), "the picture was not attached")
        // The field's label may carry its hint after its name.
        let field = web.textViews.matching(NSPredicate(format: "label BEGINSWITH %@", "Description")).firstMatch
        // A tap while the system's picker is still going away is lost, so it is tapped until the
        // description's dialog opens.
        for _ in 0..<3 {
            describe.tap()
            if field.waitForExistence(timeout: 3) {
                break
            }
        }
        let description = "A gradient \(UUID().uuidString.prefix(6))"
        type(description, into: field)
        // The keyboard covers the dialog's Save, as it would a person's finger, until put away.
        app.toolbars.buttons["Done"].firstMatch.tap()
        web.buttons["Save"].firstMatch.tap()
        // The message box is hidden from assistive technology until the dialog has closed.
        let send = web.buttons["Send"].firstMatch
        XCTAssertTrue(send.waitForExistence(timeout: 10), "no Send once described: \(app.debugDescription)")
        send.tap()
        // Shown as the picture, or as the button that opens it in the gallery, named either way
        // by its description.
        let picture = web.descendants(matching: .any).matching(NSPredicate(format: "label == %@", description))
            .firstMatch
        XCTAssertTrue(picture.waitForExistence(timeout: 60), "the picture was not shown")
    }

    /// Text follows the system's Larger Text (`theme/systemTextSize.ts`): the app launched at
    /// Accessibility Extra Large, whose body text is 40 points to the default's 17, draws a
    /// channel's name (`ASPEN_TEST_CHANNEL`, "general") in the channel list over twice as tall,
    /// and Settings at the foot of the list stays on the screen. The size is given at launch,
    /// for this app alone, as the system's own setting would give it.
    func testTextFollowsLargerText() throws {
        try signIn()
        let normal = try nameHeight(readBackChannel, at: "UICTContentSizeCategoryL")
        let larger = try nameHeight(readBackChannel, at: "UICTContentSizeCategoryAccessibilityXL")
        XCTAssertGreaterThan(larger / normal, 1.8, "drawn \(normal) then \(larger) tall")
        let settings = web.buttons["Settings"].firstMatch
        XCTAssertTrue(settings.isHittable, "Settings is off the screen: \(settings.frame)")
        XCTAssertLessThanOrEqual(settings.frame.maxX, app.windows.firstMatch.frame.maxX)
    }

    /// Launches the app at the text size `category` and reads how tall the channel list draws
    /// the name of `channel`. A channel's name is text of its own to WebKit, where the user's
    /// name at the foot of the list is inside a button that answers only by its own label.
    private func nameHeight(_ channel: String, at category: String) throws -> CGFloat {
        app.terminate()
        app.launchArguments = ["-UIPreferredContentSizeCategoryName", category]
        app.launch()
        let name = channelRow(channel)
        XCTAssertTrue(name.waitForExistence(timeout: 30), "no \(channel): \(app.debugDescription)")
        // The page measures the system's text size a moment after it starts.
        Thread.sleep(forTimeInterval: 2)
        return name.frame.height
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
