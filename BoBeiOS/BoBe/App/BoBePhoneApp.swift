import SwiftUI

@main
struct BoBePhoneApp: App {
    @State private var model = PhoneModel()

    var body: some Scene {
        WindowGroup {
            RootView(model: self.model)
                .onOpenURL { self.model.open($0) }
        }
    }
}
