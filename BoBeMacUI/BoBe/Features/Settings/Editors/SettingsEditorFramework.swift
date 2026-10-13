import Observation
import SwiftUI

@MainActor
@Observable
final class SettingsEditCoordinator {
    static let shared = SettingsEditCoordinator()

    private(set) var isDirty = false
    var showsConfirmation = false
    private var saveAction: (() async -> Bool)?
    private var discardAction: (() -> Void)?
    private var pendingTransition: (() -> Void)?

    private init() {}

    func register(isDirty: Bool, save: @escaping () async -> Bool, discard: @escaping () -> Void) {
        self.isDirty = isDirty
        self.saveAction = save
        self.discardAction = discard
    }

    func unregister() {
        guard !self.showsConfirmation else { return }
        self.isDirty = false
        self.saveAction = nil
        self.discardAction = nil
    }

    func requestTransition(_ transition: @escaping () -> Void) {
        guard self.isDirty else {
            transition()
            return
        }
        self.pendingTransition = transition
        self.showsConfirmation = true
    }

    func saveAndContinue() async {
        guard let saveAction, await saveAction() else { return }
        self.finishTransition()
    }

    func discardAndContinue() {
        self.discardAction?()
        self.finishTransition()
    }

    func cancelTransition() {
        self.pendingTransition = nil
        self.showsConfirmation = false
    }

    private func finishTransition() {
        let transition = self.pendingTransition
        self.pendingTransition = nil
        self.showsConfirmation = false
        self.isDirty = false
        transition?()
    }
}

func reconcileSavedDraft<Value: Equatable>(
    current: Value, submitted: Value, saved: Value
) -> Value {
    current == submitted ? saved : current
}

struct SettingsEditorState<SelectionID: Hashable>: Equatable {
    var selectedId: SelectionID?
    var isDirty = false
    var isLoading = false
    var isSaving = false
    var isCreating = false
    var showDeleteConfirmation = false
    var errorMessage: String?

    var hasSelection: Bool {
        self.selectedId != nil
    }

    mutating func select(_ id: SelectionID?) {
        self.selectedId = id
        self.isDirty = false
        self.showDeleteConfirmation = false
    }

    mutating func setDirty(_ value: Bool = true) {
        self.isDirty = value
    }

    mutating func setLoading(_ value: Bool) {
        self.isLoading = value
    }

    mutating func setSaving(_ value: Bool) {
        self.isSaving = value
    }

    mutating func setCreating(_ value: Bool) {
        self.isCreating = value
    }

    mutating func requestDeleteConfirmation() {
        self.showDeleteConfirmation = true
    }

    mutating func dismissDeleteConfirmation() {
        self.showDeleteConfirmation = false
    }

    mutating func setError(_ message: String?) {
        self.errorMessage = message
    }

    mutating func setError(_ error: any Error) {
        self.errorMessage = error.localizedDescription
    }

    mutating func clearError() {
        self.errorMessage = nil
    }
}

struct SettingsEditorScaffold<ListPane: View, DetailPane: View, EmptyPane: View>: View {
    private let leftWidth: CGFloat
    private let hasSelection: Bool
    private let listPane: ListPane
    private let detailPane: DetailPane
    private let emptyPane: EmptyPane

    init(
        leftWidth: CGFloat = 300,
        hasSelection: Bool,
        @ViewBuilder listPane: () -> ListPane,
        @ViewBuilder detailPane: () -> DetailPane,
        @ViewBuilder emptyPane: () -> EmptyPane
    ) {
        self.leftWidth = leftWidth
        self.hasSelection = hasSelection
        self.listPane = listPane()
        self.detailPane = detailPane()
        self.emptyPane = emptyPane()
    }

    var body: some View {
        ThemedSplitPane(leftWidth: self.leftWidth) {
            SettingsEditorListPane {
                self.listPane
            }
        } right: {
            if self.hasSelection {
                SettingsEditorDetailPane {
                    self.detailPane
                }
            } else {
                self.emptyPane
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
    }
}

struct SettingsEditorListPane<Content: View>: View {
    private let minWidth: CGFloat
    private let idealWidth: CGFloat
    private let content: Content

    init(
        minWidth: CGFloat = 220,
        idealWidth: CGFloat = 300,
        @ViewBuilder content: () -> Content
    ) {
        self.minWidth = minWidth
        self.idealWidth = idealWidth
        self.content = content()
    }

    var body: some View {
        self.content
            .frame(minWidth: self.minWidth, idealWidth: self.idealWidth)
            .frame(maxHeight: .infinity, alignment: .top)
            .padding(.horizontal, BobeMetrics.paneHorizontalPadding)
            .padding(.top, BobeMetrics.paneTopPadding)
            .padding(.bottom, BobeMetrics.paneTopPadding)
    }
}

struct SettingsEditorDetailPane<Content: View>: View {
    private let content: Content

    init(@ViewBuilder content: () -> Content) {
        self.content = content()
    }

    var body: some View {
        self.content
            .frame(maxHeight: .infinity, alignment: .top)
            .padding(.horizontal, BobeMetrics.paneHorizontalPadding)
            .padding(.top, BobeMetrics.paneTopPadding)
            .padding(.bottom, BobeMetrics.paneTopPadding)
    }
}

struct SettingsEditorActionRow<Leading: View, Trailing: View>: View {
    private let spacing: CGFloat
    private let leading: Leading
    private let trailing: Trailing

    init(
        spacing: CGFloat = 8,
        @ViewBuilder leading: () -> Leading,
        @ViewBuilder trailing: () -> Trailing
    ) {
        self.spacing = spacing
        self.leading = leading()
        self.trailing = trailing()
    }

    var body: some View {
        HStack(spacing: self.spacing) {
            self.leading
            Spacer(minLength: self.spacing)
            self.trailing
        }
    }
}

struct SettingsEditorSaveActions: View {
    let isDirty: Bool
    let isSaving: Bool
    let onDiscard: () -> Void
    let onSave: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            if self.isDirty {
                Button(L10n.tr("settings.shared.action.discard"), action: self.onDiscard)
                    .bobeButton(.secondary, size: .small)
            }
            Button(
                self.isSaving
                    ? L10n.tr("settings.shared.action.saving")
                    : L10n.tr("settings.shared.action.save"),
                action: self.onSave
            )
            .bobeButton(.primary, size: .small)
            .disabled(!self.isDirty || self.isSaving)
        }
    }
}

struct SettingsEditorErrorText: View {
    let message: String
    @Environment(\.theme) private var theme

    var body: some View {
        Text(self.message)
            .font(.caption)
            .foregroundStyle(self.theme.colors.primary)
    }
}
