import Capacitor
import UIKit

/// The app's web view (`Main.storyboard`), with the app's own plugins registered beside those
/// Capacitor finds in its packages.
class MainViewController: CAPBridgeViewController {
    override open func capacitorDidLoad() {
        bridge?.registerPluginInstance(AspenFilesPlugin())
        bridge?.registerPluginInstance(AspenPushPlugin())
    }
}
