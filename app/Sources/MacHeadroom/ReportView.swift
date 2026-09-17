import AppKit
import SwiftUI
import WebKit

/// The same HTML report the CLI writes, shown in a window. The first step for
/// any screen that has not been rebuilt natively yet.
struct ReportView: View {
    @EnvironmentObject var store: Store

    var body: some View {
        Group {
            if let url = store.reportURL {
                WebView(url: url)
            } else {
                ProgressView("Preparing the report…").frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .task { await store.makeReport() }
    }
}

struct WebView: NSViewRepresentable {
    let url: URL

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> WKWebView {
        let view = WKWebView()
        view.uiDelegate = context.coordinator
        view.loadFileURL(url, allowingReadAccessTo: url.deletingLastPathComponent())
        return view
    }

    func updateNSView(_ view: WKWebView, context: Context) {
        if view.url != url {
            view.loadFileURL(url, allowingReadAccessTo: url.deletingLastPathComponent())
        }
    }

    /// Links that open a new window, such as suggesting a cleaner on GitHub,
    /// go to the person's browser instead of nowhere.
    final class Coordinator: NSObject, WKUIDelegate {
        func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration,
                     for navigationAction: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
            if let url = navigationAction.request.url { NSWorkspace.shared.open(url) }
            return nil
        }
    }
}
