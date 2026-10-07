import Capacitor
import UIKit
import WebKit

/// The app's web view (`Main.storyboard`), with the app's own plugins registered beside those
/// Capacitor finds in its packages.
class MainViewController: CAPBridgeViewController {
    override open func capacitorDidLoad() {
        bridge?.registerPluginInstance(AspenFilesPlugin())
        bridge?.registerPluginInstance(AspenPushPlugin())
        bridge?.registerPluginInstance(AspenNavigationPlugin())
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
