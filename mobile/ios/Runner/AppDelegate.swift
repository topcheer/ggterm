import Flutter
import UIKit

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    // Register share platform channel.
    if let controller = window?.rootViewController as? FlutterViewController {
      let channel = FlutterMethodChannel(
        name: "dev.ggterm/share",
        binaryMessenger: controller.binaryMessenger
      )
      channel.setMethodCallHandler { call, result in
        if call.method == "shareText" {
          let args = call.arguments as? [String: Any]
          let text = args?["text"] as? String ?? ""
          let subject = args?["subject"] as? String ?? "GGTerm output"

          let activityVC = UIActivityViewController(
            activityItems: [text],
            applicationActivities: nil
          )
          activityVC.setValue(subject, forKey: "subject")

          if let popover = activityVC.popoverPresentationController {
            popover.sourceView = controller.view
            popover.sourceRect = CGRect(
              x: controller.view.bounds.midX,
              y: controller.view.bounds.midY,
              width: 0,
              height: 0
            )
            popover.permittedArrowDirections = []
          }

          controller.present(activityVC, animated: true)
          result(nil)
        } else if call.method == "shareUrl" {
          let args = call.arguments as? [String: Any]
          let urlString = args?["url"] as? String ?? ""
          if let url = URL(string: urlString) {
            UIApplication.shared.open(url) { success in
              result(success)
            }
          } else {
            result(false)
          }
        } else {
          result(FlutterMethodNotImplemented)
        }
      }
    }

    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)
  }
}
