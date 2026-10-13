import SwiftUI

struct BobeAvatarPalette: Sendable, Equatable {
    let ballLight: Color
    let ballDark: Color
    let orbit: Color
    let outline: Color
    let eye: Color
    let mouth: Color
    let line: Color
    let moon: Color
    let blush: Color

    init(
        ballLight: UInt32, ballDark: UInt32, orbit: UInt32, outline: UInt32, eye: UInt32,
        mouth: UInt32, line: UInt32, moon: UInt32, blush: UInt32
    ) {
        self.ballLight = Color(rgb: ballLight)
        self.ballDark = Color(rgb: ballDark)
        self.orbit = Color(rgb: orbit)
        self.outline = Color(rgb: outline)
        self.eye = Color(rgb: eye)
        self.mouth = Color(rgb: mouth)
        self.line = Color(rgb: line)
        self.moon = Color(rgb: moon)
        self.blush = Color(rgb: blush)
    }

    static let terracotta = BobeAvatarPalette(
        ballLight: 0xFFF3DC, ballDark: 0xF1D3A4, orbit: 0xC67B5C, outline: 0x6B5142, eye: 0x4A3526,
        mouth: 0xB9603F, line: 0x5A4334, moon: 0xA69080, blush: 0xF6A396
    )
    static let softClay = BobeAvatarPalette(
        ballLight: 0xF6EFE4, ballDark: 0xD2C2AF, orbit: 0x865046, outline: 0x6B5C52, eye: 0x4A3F3A,
        mouth: 0x865046, line: 0x4A4A4A, moon: 0xB8A99A, blush: 0xE6A99B
    )
    static let bubblegum = BobeAvatarPalette(
        ballLight: 0xFFE3E9, ballDark: 0xFFBAC8, orbit: 0xA33E5A, outline: 0x7A2E44, eye: 0x5A2338,
        mouth: 0xA33E5A, line: 0x5A2A3A, moon: 0xE8879C, blush: 0xF77F9B
    )
    static let cottonCandy = BobeAvatarPalette(
        ballLight: 0xE8DAF2, ballDark: 0xC4B0D8, orbit: 0x765988, outline: 0x62527B, eye: 0x43345E,
        mouth: 0x765988, line: 0x4D3F66, moon: 0xB8A4D4, blush: 0xE79CC6
    )
    static let midnightClay = BobeAvatarPalette(
        ballLight: 0xBFA683, ballDark: 0x8A7560, orbit: 0xD4926F, outline: 0x1E1E1E, eye: 0x1C1716,
        mouth: 0x3A3A3A, line: 0x2A2A2A, moon: 0xB8A090, blush: 0xD08A66
    )
    static let twilightRose = BobeAvatarPalette(
        ballLight: 0xDDA7AF, ballDark: 0xA87080, orbit: 0xF2A0B0, outline: 0x1A1A1E, eye: 0x231A20,
        mouth: 0x8A4050, line: 0x2A2228, moon: 0xC4B4E0, blush: 0xEE7F99
    )
}

extension Color {
    init(rgb: UInt32) {
        self.init(
            red: Double((rgb >> 16) & 0xFF) / 255,
            green: Double((rgb >> 8) & 0xFF) / 255,
            blue: Double(rgb & 0xFF) / 255
        )
    }
}
