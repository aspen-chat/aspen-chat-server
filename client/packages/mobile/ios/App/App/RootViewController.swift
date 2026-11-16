import UIKit

/// The window's root: the call's video beneath the web view. Capacitor's view controller has
/// no container under its web view, so this one holds `CallVideo`'s view across the whole
/// window and `MainViewController` as a child over it, its web view made transparent
/// (`MainViewController.capacitorDidLoad`), so the pictures the call engine draws show through
/// the holes the page leaves (`CallVideo`), and the page draws everything else over them. The
/// web view keeps its own frame (it ends where the keyboard begins); the video view takes the
/// whole window. What the system asks of the root about the status bar and the home indicator is
/// the web view controller's answer.
final class RootViewController: UIViewController {
    let web = MainViewController()

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = .black
        let video = CallEngine.video.view
        video.frame = view.bounds
        video.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(video)
        addChild(web)
        web.view.frame = view.bounds
        web.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(web.view)
        web.didMove(toParent: self)
        CallEngine.video.webView = web.webView
    }

    override var childForStatusBarStyle: UIViewController? { web }
    override var childForStatusBarHidden: UIViewController? { web }
    override var childForHomeIndicatorAutoHidden: UIViewController? { web }
    override var childForScreenEdgesDeferringSystemGestures: UIViewController? { web }
    override var supportedInterfaceOrientations: UIInterfaceOrientationMask { web.supportedInterfaceOrientations }
}
