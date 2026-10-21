import Foundation

/// The time of the wake-up field, read from what is typed and written in
/// the Mac's 12- or 24-hour clock, with Latin digits.
extension ClockTime {
    /// `7`, `7:30`, `07.30`, `730` and `0730`; with am or pm, also the
    /// locale's own words for them, before or after: `7:30 pm`. `nil` for
    /// anything else, `24:00`, `7:60` and `13 pm` included.
    init?(typed text: String, locale: Locale = .autoupdatingCurrent) {
        let compact = text.lowercased().filter { !$0.isWhitespace }
        let (digits, half) = Self.halfDay(of: compact, locale: locale)
        guard let (hour, minute) = Self.hourAndMinute(digits) else { return nil }
        switch half {
        case nil:
            guard (0...23).contains(hour) else { return nil }
            self.init(hour: hour, minute: minute)
        case .am:
            guard (1...12).contains(hour) else { return nil }
            self.init(hour: hour == 12 ? 0 : hour, minute: minute)
        case .pm:
            guard (1...12).contains(hour) else { return nil }
            self.init(hour: hour == 12 ? 12 : hour + 12, minute: minute)
        }
    }

    /// The hour and minute `date` shows on the Mac's clock.
    init(of date: Date, calendar: Calendar = .autoupdatingCurrent) {
        self.init(hour: calendar.component(.hour, from: date), minute: calendar.component(.minute, from: date))
    }

    /// `07:30`, or `7:30 AM` where the Mac shows a 12-hour clock: always
    /// Latin digits and AM or PM, as the panel is in English, so that
    /// [`init?(typed:)`] reads it back in every locale. The locale's own
    /// way (`٧:٣٠ ص`, `清晨7:30`) it may not.
    func text(locale: Locale = .autoupdatingCurrent) -> String {
        // A day in a zone without daylight saving time, so every time exists.
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = .gmt
        guard let date = calendar.date(from: DateComponents(year: 2000, month: 1, day: 1, hour: hour, minute: minute))
        else {
            assertionFailure("invariant: every hour and minute exists on 1 January 2000 in GMT")
            return String(format: "%02d:%02d", hour, minute)
        }
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.timeZone = .gmt
        formatter.dateFormat = Self.usesTwelveHours(locale) ? "h:mm a" : "HH:mm"
        return formatter.string(from: date)
    }

    /// Whether `locale`, with the user's choice in System Settings, shows
    /// hours 1 to 12 with a day period.
    private static func usesTwelveHours(_ locale: Locale) -> Bool {
        // No pattern only for a locale Foundation does not know: the
        // 24-hour clock most of the world uses.
        let pattern = DateFormatter.dateFormat(fromTemplate: "j", options: 0, locale: locale) ?? "HH"
        return pattern.contains("a") || pattern.contains("b") || pattern.contains("B")
    }

    private enum HalfDay {
        case am, pm
    }

    /// `text` without its am or pm, and which of them it had.
    private static func halfDay(of text: String, locale: Locale) -> (String, HalfDay?) {
        let formatter = DateFormatter()
        formatter.locale = locale
        let words: [(String, HalfDay)] = [
            ("a.m.", .am), ("am", .am), (formatter.amSymbol, .am),
            ("p.m.", .pm), ("pm", .pm), (formatter.pmSymbol, .pm),
        ]
        .map { (word, half) in (word.lowercased().filter { !$0.isWhitespace }, half) }
        .filter { (word, _) in !word.isEmpty }
        // The longest first: "a.m." is not "am" with dots left over.
        for (word, half) in words.sorted(by: { $0.0.count > $1.0.count }) {
            if text.hasSuffix(word) { return (String(text.dropLast(word.count)), half) }
            if text.hasPrefix(word) { return (String(text.dropFirst(word.count)), half) }
        }
        return (text, nil)
    }

    /// `7`, `7:30`, `7.30`, `730`: one or two digits of hours, then two of
    /// minutes, if any, with or without a separator.
    private static func hourAndMinute(_ text: String) -> (Int, Int)? {
        let isDigit: (Character) -> Bool = { $0.isASCII && $0.isNumber }
        let parts = text.split(separator: ":", omittingEmptySubsequences: false)
            .flatMap { $0.split(separator: ".", omittingEmptySubsequences: false) }
        let (hours, minutes): (Substring, Substring)
        switch parts.count {
        case 1:
            let all = parts[0]
            guard (1...4).contains(all.count) else { return nil }
            (hours, minutes) = all.count <= 2 ? (all, "00") : (all.dropLast(2), all.suffix(2))
        case 2:
            (hours, minutes) = (parts[0], parts[1])
        default:
            return nil
        }
        guard (1...2).contains(hours.count), minutes.count == 2,
              hours.allSatisfy(isDigit), minutes.allSatisfy(isDigit),
              let hour = Int(hours), let minute = Int(minutes), (0...59).contains(minute)
        else { return nil }
        return (hour, minute)
    }
}
