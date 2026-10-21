/// One of the colours the colour page offers at a click.
struct Swatch: Identifiable {
    let name: String
    let hue: Double
    let saturation: Double
    var id: String { name }

    /// The panel's colour is this one, to the degree and the percent.
    func isShown(hue: Double, saturation: Double) -> Bool {
        Int(hue.rounded()) == Int(self.hue) && Int(saturation.rounded()) == Int(self.saturation)
    }

    static let all = [
        Swatch(name: "Red", hue: 0, saturation: 100),
        Swatch(name: "Orange", hue: 25, saturation: 100),
        Swatch(name: "Yellow", hue: 55, saturation: 100),
        Swatch(name: "Green", hue: 120, saturation: 100),
        Swatch(name: "Cyan", hue: 185, saturation: 100),
        Swatch(name: "Blue", hue: 235, saturation: 100),
        Swatch(name: "Purple", hue: 275, saturation: 100),
        Swatch(name: "Pink", hue: 320, saturation: 60),
    ]
}
