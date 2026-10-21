import SwiftUI

/// The corner radius of a well.
private let wellRadius: CGFloat = 11

extension View {
    /// The rounded, lightly filled surface that groups controls: the tab
    /// bar, the fields of the timers page.
    func well() -> some View {
        padding(5)
            .background(RoundedRectangle(cornerRadius: wellRadius).fill(Color.primary.opacity(0.05)))
            .overlay(RoundedRectangle(cornerRadius: wellRadius).stroke(Color.primary.opacity(0.06)))
    }

    /// A well that takes a click anywhere on it, not only on its controls.
    func well(onTap action: @escaping () -> Void) -> some View {
        well()
            .contentShape(RoundedRectangle(cornerRadius: wellRadius))
            .onTapGesture(perform: action)
    }
}
