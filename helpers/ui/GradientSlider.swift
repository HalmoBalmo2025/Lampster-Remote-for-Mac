import SwiftUI

/// A slider whose track shows what it sets. Every move is sent;
/// Lampster Remote writes only the latest value to the lamp.
struct GradientSlider: View {
    @Binding var value: Double
    let range: ClosedRange<Double>
    let colors: [Color]
    /// `true` with every move of a drag, `false` at its end.
    let onEditingChanged: (Bool) -> Void
    /// A slider that goes away while dragged never hears the end.
    let onDisappear: () -> Void
    let changed: () -> Void

    private let height: CGFloat = 18

    var body: some View {
        GeometryReader { geometry in
            let width = geometry.size.width
            let fraction = (value - range.lowerBound) / (range.upperBound - range.lowerBound)
            let knob = height + 4
            ZStack(alignment: .leading) {
                Capsule()
                    .fill(LinearGradient(colors: colors, startPoint: .leading, endPoint: .trailing))
                    .overlay(Capsule().strokeBorder(Color.primary.opacity(0.12)))
                    .frame(height: height)
                Circle()
                    .fill(Color.white)
                    .shadow(color: .black.opacity(0.3), radius: 2, y: 1)
                    .frame(width: knob, height: knob)
                    .offset(x: (width - knob) * fraction)
            }
            .frame(height: knob)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { drag in
                        onEditingChanged(true)
                        let position = min(max((drag.location.x - knob / 2) / (width - knob), 0), 1)
                        let new = (range.lowerBound + position * (range.upperBound - range.lowerBound)).rounded()
                        if new != value {
                            value = new
                            changed()
                        }
                    }
                    .onEnded { _ in
                        onEditingChanged(false)
                        changed()
                    })
        }
        .frame(height: height + 4)
        .onDisappear(perform: onDisappear)
    }
}
