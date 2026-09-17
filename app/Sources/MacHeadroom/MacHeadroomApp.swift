import MacHeadroomUI
import SwiftUI

@main
struct MacHeadroomApp: App {
    @StateObject private var store = Store()

    var body: some Scene {
        MenuBarExtra {
            MenuView().environmentObject(store)
        } label: {
            MenuLabel(store: store)
        }
        .menuBarExtraStyle(.window)

        Window("mac-headroom", id: "report") {
            ReportView().environmentObject(store)
        }
        .defaultSize(width: 980, height: 820)
    }
}
