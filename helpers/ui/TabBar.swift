import SwiftUI

/// White and Color as tabs, the pages for timers and settings, then Quit.
struct TabBar: View {
    @ObservedObject var model: Model

    var body: some View {
        HStack(spacing: 4) {
            tab(.white, "White", "sun.max")
            tab(.color, "Color", "paintpalette")
            page(.timers, "alarm", "Wake-up light and sleep timer")
            page(.settings, "gearshape", "Settings")
            Divider().frame(height: 22).padding(.horizontal, 2)
            Button {
                send(.quit)
            } label: {
                Image(systemName: "power")
                    .font(.system(size: 15, weight: .regular))
                    .frame(width: 40, height: 30)
                    .foregroundStyle(.secondary)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .keyboardShortcut("q")
            .help("Switch Lampster off and quit Lampster Remote")
        }
        .well()
    }

    /// An icon that opens a page of its own.
    func page(_ page: Page, _ symbol: String, _ help: String) -> some View {
        Button {
            model.page = page
        } label: {
            Image(systemName: symbol)
                .font(.system(size: 15, weight: .regular))
                .frame(width: 36, height: 30)
                .selected(model.page == page)
        }
        .buttonStyle(.plain)
        .help(help)
    }

    func tab(_ mode: Mode, _ title: String, _ symbol: String) -> some View {
        let selected = model.page == .lamp && model.mode == mode
        return Button {
            model.setMode(mode)
        } label: {
            Label(title, systemImage: symbol)
                .lineLimit(1)
                .font(.system(size: 13, weight: selected ? .semibold : .regular))
                .frame(maxWidth: .infinity, minHeight: 30)
                .selected(selected)
        }
        .buttonStyle(.plain)
    }
}

extension View {
    /// The look of a tab or page in the tab bar, highlighted when it is shown.
    fileprivate func selected(_ selected: Bool) -> some View {
        foregroundStyle(selected ? Color.primary : .secondary)
            .background(
                RoundedRectangle(cornerRadius: 8)
                    .fill(selected ? Color.primary.opacity(0.12) : .clear))
            .contentShape(Rectangle())
    }
}
