import AppKit
import SwiftUI
import WebKit

/// The same HTML report the CLI writes, shown in a window. The first step for
/// any screen that has not been rebuilt natively yet.
public struct ReportView: View {
    @EnvironmentObject var store: Store

    public init() {}

    public var body: some View {
        Group {
            if let url = store.reportURL {
                // Show the report that is already there at once; a fresh one
                // replaces it when it is ready.
                WebView(url: url, version: store.reportVersion)
            } else {
                ProgressView("Preparing the report…").frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .task { await store.makeReport() }
    }
}

struct WebView: NSViewRepresentable {
    let url: URL
    let version: Int

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> WKWebView {
        let view = WKWebView()
        view.uiDelegate = context.coordinator
        context.coordinator.loadedVersion = version
        view.loadFileURL(url, allowingReadAccessTo: url.deletingLastPathComponent())
        return view
    }

    /// The file keeps the same path when it is rewritten, so reload on a new
    /// version rather than a new URL.
    func updateNSView(_ view: WKWebView, context: Context) {
        if context.coordinator.loadedVersion != version || view.url != url {
            context.coordinator.loadedVersion = version
            view.loadFileURL(url, allowingReadAccessTo: url.deletingLastPathComponent())
        }
    }

    /// Links that open a new window, such as suggesting a cleaner on GitHub,
    /// go to the person's browser instead of nowhere.
    final class Coordinator: NSObject, WKUIDelegate {
        var loadedVersion = -1
        func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                     for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
            if let url = navigationAction.request.url { NSWorkspace.shared.open(url) }
            return nil
        }
    }
}
