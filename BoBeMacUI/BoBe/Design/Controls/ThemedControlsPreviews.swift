import SwiftUI

#if !SPM_BUILD
    #Preview("Button Variants") {
        VStack(spacing: 12) {
            HStack(spacing: 8) {
                Button("Primary") {}.bobeButton(.primary, size: .regular)
                Button("Secondary") {}.bobeButton(.secondary, size: .regular)
                Button("Ghost") {}.bobeButton(.ghost, size: .regular)
                Button("Destructive") {}.bobeButton(.destructive, size: .regular)
            }
            HStack(spacing: 8) {
                Button("Small") {}.bobeButton(.primary, size: .small)
                Button("Mini") {}.bobeButton(.primary, size: .mini)
            }
        }
        .environment(\.theme, allThemes[0])
        .padding()
    }

    #Preview("BobeTextField") {
        @Previewable @State var text = ""
        VStack(spacing: 12) {
            BobeTextField(placeholder: "Enter something...", text: $text)
            BobeTextField(placeholder: "Fixed width", text: $text, width: 200)
        }
        .environment(\.theme, allThemes[0])
        .padding()
        .frame(width: 400)
    }

    #Preview("BobeSecureField") {
        @Previewable @State var text = ""
        BobeSecureField(placeholder: "sk-...", text: $text, width: 300)
            .environment(\.theme, allThemes[0])
            .padding()
    }

    #Preview("BobeMenuPicker") {
        @Previewable @State var selection = "Option A"
        BobeMenuPicker(
            selection: $selection,
            options: ["Option A", "Option B", "Option C"],
            label: { $0 },
            width: 200
        )
        .environment(\.theme, allThemes[0])
        .padding()
        .frame(height: 300)
    }

    #Preview("BobeSpinner + Progress") {
        VStack(spacing: 16) {
            HStack(spacing: 16) {
                BobeSpinner(size: 14)
                BobeSpinner(size: 20)
                BobeSpinner(size: 28)
            }
            BobeLinearProgressBar(progress: 0.65)
                .frame(width: 300)
            BobeLinearProgressBar(progress: 0.2)
                .frame(width: 300)
        }
        .environment(\.theme, allThemes[0])
        .padding()
    }

    #Preview("BobeSelectableRow") {
        VStack(spacing: 6) {
            BobeSelectableRow(isSelected: true, content: {
                Text("Selected item")
            })
            BobeSelectableRow(isSelected: false, content: {
                Text("Unselected item")
            })
        }
        .environment(\.theme, allThemes[0])
        .frame(width: 300)
        .padding()
    }
#endif
