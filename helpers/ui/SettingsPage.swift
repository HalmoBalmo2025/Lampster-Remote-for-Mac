import SwiftUI

struct SettingsPage: View {
    @ObservedObject var model: Model

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            setting(
                "Open at login",
                "Starts Lampster Remote when you log in, so it is in the menu bar right away.\nAfter moving the app, switch this off and on again.",
                Binding(get: { model.view.options.openAtLogin == .on }, set: model.setOpenAtLogin))
            Divider()
            setting(
                "Switch Lampster off with the Mac",
                "When the Mac locks, sleeps, shuts down, restarts or logs out. Otherwise Lampster stays as it is.",
                Binding(get: { model.view.options.offWithMac == .on }, set: model.setOffWithMac))
        }
    }

    func setting(_ title: String, _ detail: String, _ isOn: Binding<Bool>) -> some View {
        HStack(alignment: .top, spacing: 12) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.subheadline.weight(.medium))
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
            Toggle("", isOn: isOn).toggleStyle(.switch).labelsHidden()
        }
    }
}
