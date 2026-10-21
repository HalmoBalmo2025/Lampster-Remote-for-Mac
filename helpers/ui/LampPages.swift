import SwiftUI

/// Brightness and tone of the white light.
struct WhitePage: View {
    @ObservedObject var model: Model

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Labelled(title: "Brightness", value: "\(Int(model.brightness.rounded())) %") {
                GradientSlider(
                    value: $model.brightness, range: 1...100,
                    colors: [Color(white: 0.25), Color(white: 0.95)],
                    onEditingChanged: model.setDragging, onDisappear: model.stopDragging,
                    changed: model.sendWhite)
            }
            Labelled(title: "Tone", value: toneLabel) {
                GradientSlider(
                    value: $model.tone, range: 0...100,
                    colors: [Color(red: 1, green: 0.72, blue: 0.4), Color(white: 0.97), Color(red: 0.78, green: 0.88, blue: 1)],
                    onEditingChanged: model.setDragging, onDisappear: model.stopDragging,
                    changed: model.sendWhite)
            }
        }
    }

    var toneLabel: String {
        switch model.tone {
        case ..<20: "Warm"
        case 80...: "Cool"
        default: "Neutral"
        }
    }
}

/// Swatches, hue, saturation and brightness of the colour light.
struct ColorPage: View {
    @ObservedObject var model: Model

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            VStack(alignment: .leading, spacing: 8) {
                Text("Color").font(.subheadline.weight(.medium))
                // Spread over the width of the sliders, the first and the
                // last at their ends.
                HStack(spacing: 0) {
                    ForEach(Swatch.all) { swatch in
                        if swatch.id != Swatch.all.first?.id { Spacer(minLength: 0) }
                        Button {
                            model.hue = swatch.hue
                            model.saturation = swatch.saturation
                            model.sendColor()
                        } label: {
                            Circle()
                                .fill(Color(hue: swatch.hue / 360, saturation: swatch.saturation / 100, brightness: 1))
                                .overlay(Circle().strokeBorder(Color.primary.opacity(0.15)))
                                .frame(width: 22, height: 22)
                                // A ring with some room around the colour shown;
                                // outside the frame, so the swatches do not move.
                                .overlay(
                                    Circle()
                                        .stroke(Color.primary.opacity(0.45), lineWidth: 1.5)
                                        .padding(-4)
                                        .opacity(swatch.isShown(hue: model.hue, saturation: model.saturation) ? 1 : 0))
                        }
                        .buttonStyle(.plain)
                        .help(swatch.name)
                    }
                }
            }
            Labelled(title: "Hue", value: "\(Int(model.hue.rounded()))°") {
                GradientSlider(
                    value: $model.hue, range: 0...359,
                    colors: stride(from: 0.0, through: 1.0, by: 1.0 / 6).map {
                        Color(hue: $0, saturation: 1, brightness: 1)
                    },
                    onEditingChanged: model.setDragging, onDisappear: model.stopDragging,
                    changed: model.sendColor)
            }
            Labelled(title: "Saturation", value: "\(Int(model.saturation.rounded())) %") {
                GradientSlider(
                    value: $model.saturation, range: 0...100,
                    colors: [Color(white: 0.95), Color(hue: model.hue / 360, saturation: 1, brightness: 1)],
                    onEditingChanged: model.setDragging, onDisappear: model.stopDragging,
                    changed: model.sendColor)
            }
            Labelled(title: "Brightness", value: "\(Int(model.brightness.rounded())) %") {
                GradientSlider(
                    value: $model.brightness, range: 1...100,
                    colors: [
                        Color(hue: model.hue / 360, saturation: model.saturation / 100, brightness: 0.25),
                        Color(hue: model.hue / 360, saturation: model.saturation / 100, brightness: 1),
                    ],
                    onEditingChanged: model.setDragging, onDisappear: model.stopDragging,
                    changed: model.sendColor)
            }
        }
    }
}

/// A control with its name and its value above it.
struct Labelled<Content: View>: View {
    let title: String
    let value: String
    @ViewBuilder let content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(title).font(.subheadline.weight(.medium))
                Spacer()
                Text(value).font(.subheadline.monospacedDigit()).foregroundStyle(.secondary)
            }
            content
        }
    }
}
