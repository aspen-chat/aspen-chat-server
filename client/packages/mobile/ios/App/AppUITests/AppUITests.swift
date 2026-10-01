import XCTest

/// The app driven as a person drives it, with real taps and finger drags in WebKit, against the
/// Aspen server the web build names (`VITE_ASPEN_SERVER_URL`). The account comes from the
/// environment, `ASPEN_TEST_USER` and `ASPEN_TEST_PASSWORD`, which `xcodebuild test` passes on
/// when given as `TEST_RUNNER_ASPEN_TEST_USER` and `TEST_RUNNER_ASPEN_TEST_PASSWORD`; without
/// them the tests skip. `ASPEN_TEST_CHANNEL` names the channel read back through ("general").
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

    /// Signs in when the app asks; an app signed in already goes on as it is.
    private func signIn() throws {
        let user = try environment("ASPEN_TEST_USER")
        let password = try environment("ASPEN_TEST_PASSWORD")
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
        let row = web.staticTexts[channel].firstMatch
        XCTAssertTrue(row.waitForExistence(timeout: 20), "no channel \(channel)")
        row.tap()
        XCTAssertTrue(web.textViews["Message"].firstMatch.waitForExistence(timeout: 20))
        sleep(3)

        let drag: CGFloat = 180
        let slop: CGFloat = 12
        var strays: [String] = []
        for step in 0..<12 {
            guard let anchor = middleText() else {
                strays.append("step \(step): nothing to follow in view")
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

    /// Something in view, between the top and the message box, that only one element shows:
    /// a line of text, or a picture by its description.
    private func middleText() -> (label: String, element: XCUIElement)? {
        let screen = app.windows.firstMatch.frame
        let band = screen.minY + screen.height * 0.15...screen.minY + screen.height * 0.75
        for kind in [XCUIElement.ElementType.staticText, .image] {
            let all = web.descendants(matching: kind)
            for element in all.allElementsBoundByIndex {
                let label = element.label
                guard label.count > 4, band.contains(element.frame.midY) else { continue }
                let same = all.matching(NSPredicate(format: "label == %@", label))
                if same.count == 1 {
                    return (label, same.firstMatch)
                }
            }
        }
        return nil
    }
}
