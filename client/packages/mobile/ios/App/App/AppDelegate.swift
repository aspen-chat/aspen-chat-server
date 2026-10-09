import UIKit
import Capacitor

@UIApplicationMain
class AppDelegate: UIResponder, UIApplicationDelegate {

    var window: UIWindow?

    func application(_ application: UIApplication, didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?) -> Bool {
        excludeWebDataFromBackup()
        return true
    }

    /// Keeps the web view's data out of backups and transfers to a new phone: its local storage
    /// holds the session on each deployment, which would sign whoever restored it in as the
    /// user. The web view keeps its data under `Library/WebKit`, and a directory excluded from
    /// backup takes everything in it along. The directory is made here if the web view has not
    /// made it yet, so the mark is on it before anything is written, and marked again at every
    /// launch, in case the web view made it anew.
    private func excludeWebDataFromBackup() {
        let manager = FileManager.default
        guard let library = manager.urls(for: .libraryDirectory, in: .userDomainMask).first else {
            return
        }
        var webKit = library.appendingPathComponent("WebKit", isDirectory: true)
        do {
            try manager.createDirectory(at: webKit, withIntermediateDirectories: true)
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try webKit.setResourceValues(values)
        } catch {
            NSLog("Aspen could not keep the web view's data out of backups: %@", String(describing: error))
        }
    }

    func applicationWillResignActive(_ application: UIApplication) {
    }

    func applicationDidEnterBackground(_ application: UIApplication) {
    }

    func applicationWillEnterForeground(_ application: UIApplication) {
    }

    func applicationDidBecomeActive(_ application: UIApplication) {
    }

    func applicationWillTerminate(_ application: UIApplication) {
    }

    // APNs registration reaches the Capacitor push plugin by these notifications, which it
    // listens for; without them the app never learns its token.
    func application(_ application: UIApplication, didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data) {
        NotificationCenter.default.post(name: .capacitorDidRegisterForRemoteNotifications, object: deviceToken)
    }

    func application(_ application: UIApplication, didFailToRegisterForRemoteNotificationsWithError error: Error) {
        NotificationCenter.default.post(name: .capacitorDidFailToRegisterForRemoteNotifications, object: error)
    }

    func application(_ application: UIApplication,
                     configurationForConnecting connectingSceneSession: UISceneSession,
                     options: UIScene.ConnectionOptions) -> UISceneConfiguration {
        let config = UISceneConfiguration(name: "Default Configuration",
                                          sessionRole: connectingSceneSession.role)
        config.delegateClass = SceneDelegate.self
        return config
    }
}
