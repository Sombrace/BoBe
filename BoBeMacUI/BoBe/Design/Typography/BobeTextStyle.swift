import SwiftUI

enum BobeTextStyle {
    case sectionLabel
    case rowTitle
    case rowMeta
    case helper
    case body
    case badge

    case chatPending
    case chatBody

    case setupTitle
    case setupBody
    case setupHeading

    case inputField
    case heading

    case settingsBody

    case display
    case title
    case caption

    var font: Font {
        switch self {
        case .setupTitle:
            .system(size: 27, weight: .semibold, design: .rounded)
        case .display:
            .system(size: 22, weight: .semibold, design: .rounded)
        case .title:
            .system(size: 17, weight: .semibold, design: .rounded)
        case .heading:
            .system(size: 15, weight: .semibold)
        case .rowTitle, .setupHeading:
            .system(size: 13.5, weight: .semibold)
        case .inputField:
            .system(size: 14)
        case .settingsBody, .setupBody:
            .system(size: 13.5)
        case .body:
            .system(size: 13)
        case .chatBody:
            .system(size: 13.5)
        case .rowMeta, .helper:
            .system(size: 12)
        case .caption:
            .system(size: 11.5, weight: .medium)
        case .sectionLabel:
            .system(size: 11.5, weight: .semibold)
        case .badge:
            .system(size: 10, weight: .semibold)
        case .chatPending:
            .system(size: 10.5)
        }
    }
}

extension View {
    func bobeTextStyle(_ style: BobeTextStyle) -> some View {
        font(style.font)
    }
}
