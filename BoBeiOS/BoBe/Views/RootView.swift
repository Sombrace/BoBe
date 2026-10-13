import SwiftUI

struct RootView: View {
    @Bindable var model: PhoneModel
    @Environment(\.scenePhase) private var scenePhase

    private var invitePending: Binding<Bool> {
        Binding(
            get: { self.model.pendingInvite != nil },
            set: { if !$0 { self.model.pendingInvite = nil } }
        )
    }

    var body: some View {
        ZStack {
            if self.model.isOnboarded {
                HomeTabs(model: self.model)
                    .transition(.opacity)
            } else {
                OnboardingView(model: self.model)
                    .transition(.opacity)
            }
        }
        .phoneThemed()
        .task { self.model.start() }
        .onChange(of: self.scenePhase) { _, phase in
            self.model.scenePhaseChanged(phase)
        }
        .alert(L10n.tr("phone.invite.switch.title"), isPresented: self.invitePending) {
            Button(L10n.tr("phone.invite.switch.confirm")) { self.model.acceptPendingInvite() }
            Button(L10n.tr("phone.common.cancel"), role: .cancel) { self.model.pendingInvite = nil }
        } message: {
            Text(L10n.tr("phone.invite.switch.message"))
        }
    }
}
