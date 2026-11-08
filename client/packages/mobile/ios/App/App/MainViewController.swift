import Capacitor
import UIKit
import WebKit

/// The app's web view, with the app's own plugins registered beside those Capacitor finds in its
/// packages.
///
/// The web view ends where the keyboard begins, as a browser's page does on Android: the page
/// lays itself out above the keyboard (the message box sits on it, and `window.innerHeight` says
/// how much room is left), rather than WebKit drawing the keyboard over the page and scrolling
/// the page up under the status bar to show the focused field, with the room kept for the home
/// indicator left between the field and the keyboard. WebKit's own following of the keyboard is
/// turned off, so it does not scroll the page as well. The bar of ⌃ ⌄ ✓ WebKit puts above the
/// keyboard for a form's fields is hidden: the app's fields are not a form to step through.
class MainViewController: CAPBridgeViewController {
    /// How much of the window's foot the keyboard covers: 0 while it is down, or floating.
    private var keyboardOverlap: CGFloat = 0

    override open func capacitorDidLoad() {
        bridge?.registerPluginInstance(AspenFilesPlugin())
        bridge?.registerPluginInstance(AspenPushPlugin())
        bridge?.registerPluginInstance(AspenNavigationPlugin())
        bridge?.registerPluginInstance(AspenAgePlugin())
        if let webView {
            for name in [
                UIResponder.keyboardWillShowNotification, UIResponder.keyboardWillHideNotification,
                UIResponder.keyboardWillChangeFrameNotification, UIResponder.keyboardDidChangeFrameNotification,
            ] {
                NotificationCenter.default.removeObserver(webView, name: name, object: nil)
            }
        }
        NotificationCenter.default.addObserver(
            self, selector: #selector(keyboardWillChangeFrame(_:)),
            name: UIResponder.keyboardWillChangeFrameNotification, object: nil)
        Self.hideFormAccessoryBar()
    }

    /// The window lays its root view out across itself again (when turned, say); the web view
    /// keeps clear of the keyboard through it.
    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        fitAboveKeyboard()
    }

    @objc private func keyboardWillChangeFrame(_ notification: Notification) {
        guard let window = view.window,
            let end = (notification.userInfo?[UIResponder.keyboardFrameEndUserInfoKey] as? NSValue)?.cgRectValue
        else { return }
        let frame = window.convert(end, from: window.screen.coordinateSpace)
        // Docked, the keyboard reaches the window's foot; floating (on an iPad), or going away,
        // it does not, and takes nothing from the page.
        let docked = frame.maxY >= window.bounds.maxY - 1 && frame.minY < window.bounds.maxY
        keyboardOverlap = docked ? window.bounds.maxY - frame.minY : 0
        let info = notification.userInfo
        let duration = (info?[UIResponder.keyboardAnimationDurationUserInfoKey] as? Double) ?? 0.25
        let curve = (info?[UIResponder.keyboardAnimationCurveUserInfoKey] as? UInt) ?? 7
        UIView.animate(
            withDuration: duration, delay: 0,
            options: [UIView.AnimationOptions(rawValue: curve << 16), .beginFromCurrentState]
        ) {
            self.fitAboveKeyboard()
        }
    }

    private func fitAboveKeyboard() {
        guard let window = view.window else { return }
        let fitted = CGRect(
            x: window.bounds.minX, y: window.bounds.minY, width: window.bounds.width,
            height: window.bounds.height - keyboardOverlap)
        if view.frame != fitted {
            view.frame = fitted
        }
    }

    /// Gives WebKit's view of the page no accessory view, which is what it shows the ⌃ ⌄ ✓ bar
    /// as. The view's class is WebKit's own, unnamed in its API, so it is found by name, and
    /// left as it is should WebKit ever call it otherwise.
    private static func hideFormAccessoryBar() {
        guard let contentView = NSClassFromString("WKContentView"),
            let method = class_getInstanceMethod(contentView, #selector(getter: UIResponder.inputAccessoryView))
        else { return }
        let none: @convention(block) (AnyObject) -> UIView? = { _ in nil }
        method_setImplementation(method, imp_implementationWithBlock(none))
    }
}

/// What the web view hands to the system. Capacitor opens every address a page leaves the app
/// for with whichever app claims it; this lets only web and mail addresses leave
/// (`externalSchemes`), so a page (a plugin's view, a message's link) cannot open another app
/// through a scheme of its own. It has no methods; Capacitor asks it about each navigation
/// (`shouldOverrideLoad`). Kept in this file, which the Xcode project already builds.
@objc(AspenNavigationPlugin)
public class AspenNavigationPlugin: CAPPlugin, CAPBridgedPlugin {
    public let identifier = "AspenNavigationPlugin"
    public let jsName = "AspenNavigation"
    public let pluginMethods: [CAPPluginMethod] = []

    /// The schemes an address may leave the app with, to the system's handler for it.
    static let externalSchemes: Set<String> = ["http", "https", "mailto"]

    /// Cancels a navigation of the whole page (or a new window) to an address outside the app
    /// whose scheme is not one of `externalSchemes`; leaves every other to Capacitor (`nil`),
    /// which loads the app's own pages in place and opens web and mail addresses outside.
    @objc override public func shouldOverrideLoad(_ navigationAction: WKNavigationAction) -> NSNumber? {
        guard let url = navigationAction.request.url, let bridge = bridge else {
            return nil
        }
        let whole = navigationAction.targetFrame == nil || navigationAction.targetFrame?.isMainFrame == true
        let own = url.absoluteString.hasPrefix(bridge.config.serverURL.absoluteString)
            || url.absoluteString.hasPrefix(bridge.config.localURL.absoluteString)
        guard whole, !own else {
            return nil
        }
        let scheme = url.scheme?.lowercased() ?? ""
        return Self.externalSchemes.contains(scheme) ? nil : true
    }
}
