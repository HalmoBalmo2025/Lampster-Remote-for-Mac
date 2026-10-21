import AppKit
import SwiftUI

/// Ends typing in a field: its cursor goes, and the panel no longer
/// takes keys for it.
@MainActor
func endEditing() {
    NSApp.keyWindow?.makeFirstResponder(nil)
}

struct Panel: View {
    @ObservedObject var model: Model

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            header
            if let hint = model.hint { hintView(hint) }
            TabBar(model: model)
            switch model.page {
            case .settings:
                SettingsPage(model: model)
            case .timers:
                TimersPage(model: model)
            case .lamp:
                Group {
                    switch model.mode {
                    case .white: WhitePage(model: model)
                    case .color: ColorPage(model: model)
                    }
                }
                .disabled(!model.isConnected)
                .opacity(model.isConnected ? 1 : 0.45)
            }
            if let problem = model.view.problem {
                Label(problem, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(16)
        // A click where no control takes it ends typing in a field;
        // buttons, switches, sliders and fields keep their own clicks.
        .contentShape(Rectangle())
        .onTapGesture(perform: endEditing)
        // Opaque instead of the see-through glass: easier to read.
        .background(Color(nsColor: .windowBackgroundColor).ignoresSafeArea())
    }

    var header: some View {
        HStack(spacing: 12) {
            Image(nsImage: Robot.image(size: 40, look: model.robotLook))
                .frame(width: 40, height: 40)
            VStack(alignment: .leading, spacing: 2) {
                Text("Lampster Remote").font(.headline)
                HStack(spacing: 10) {
                    Text(model.status)
                    if let temperature = model.temperatureText {
                        // The thermometer close to its number, apart from the state.
                        HStack(spacing: 3) {
                            Image(systemName: "thermometer.medium")
                            Text(temperature)
                        }
                        .help("Temperature inside Lampster")
                    }
                }
                .font(.subheadline)
                .foregroundStyle(.secondary)
            }
            Spacer()
            Toggle("", isOn: Binding(get: { model.isOn }, set: model.setPower))
                .toggleStyle(.switch)
                .labelsHidden()
                .disabled(!model.isConnected)
                .help("Right-click the menu bar icon to switch on or off without opening the panel.")
        }
    }

    func hintView(_ hint: Hint) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(hint.text).font(.callout).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let button = hint.button, let url = URL(string: button.url) {
                Button(button.title) { NSWorkspace.shared.open(url) }
            }
        }
    }
}
