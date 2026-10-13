import Foundation

enum Validations {
    static let soulContentMinLength = 10
    static let userProfileContentMinLength = 10

    static func validateSoulContent(_ content: String) -> String? {
        let trimmed = content.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.count < self.soulContentMinLength {
            return String(
                format: L10n.tr("settings.shared.validation.content_min_length_format"),
                self.soulContentMinLength
            )
        }
        return nil
    }

    static func validateUserProfileContent(_ content: String) -> String? {
        let trimmed = content.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.count < self.userProfileContentMinLength {
            return String(
                format: L10n.tr("settings.shared.validation.content_min_length_format"),
                self.userProfileContentMinLength
            )
        }
        return nil
    }
}
