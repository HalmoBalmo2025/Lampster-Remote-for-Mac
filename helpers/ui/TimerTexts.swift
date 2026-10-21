import Foundation

/// A line under a timer's row.
enum TimerCaption: Hashable {
    /// What the timer does.
    case state(String)
    /// Why the text in the field cannot be set.
    case hint(String)
}

/// What the timers page says, from the model alone, so that the Swift tests
/// reach it. `now` and `calendar` come from outside.
extension Model {
    /// What the wake-up light does, and, while the field holds no time, why
    /// that cannot be set; the first stays whatever is typed.
    func wakeCaptions(now: Date, calendar: Calendar = .autoupdatingCurrent) -> [TimerCaption] {
        let hint = "Type a time such as \(ClockTime(hour: 7, minute: 30).text()), then press Return."
        return [.state(wakeState(now: now, calendar: calendar))] + (typedWakeTime == nil ? [.hint(hint)] : [])
    }

    func sleepCaptions(now: Date, calendar: Calendar = .autoupdatingCurrent) -> [TimerCaption] {
        let hint = "Type \(Model.sleepMinutes.lowerBound) to \(Model.sleepMinutes.upperBound) minutes, then press Return."
        return [.state(sleepState(now: now, calendar: calendar))] + (typedSleepMinutes == nil ? [.hint(hint)] : [])
    }

    /// Whether the time in the wake-up field comes today or tomorrow, as
    /// `timer::next` in src/timer.rs decides it with jiff: a time that is
    /// now comes tomorrow; one that daylight saving skips today is taken as
    /// the same span after the gap (2:30 becomes 3:30); one it repeats is
    /// the pass with the offset of `now`. `nil` while the field holds no
    /// time.
    func fieldDay(now: Date, calendar: Calendar = .autoupdatingCurrent) -> String? {
        guard let time = typedWakeTime else { return nil }
        // From the day's components, not `date(bySettingHour:)`, which
        // turns 2:30 into 3:00 whatever its matching policy.
        var components = calendar.dateComponents([.era, .year, .month, .day], from: now)
        components.hour = time.hour
        components.minute = time.minute
        guard let first = calendar.date(from: components) else { return nil }
        return passWithOffset(of: now, from: first, calendar: calendar) > now ? "today" : "tomorrow"
    }

    /// `first`, or in the hour repeated in autumn its second pass, if that
    /// one has the offset of `now`: Calendar takes the first.
    private func passWithOffset(of now: Date, from first: Date, calendar: Calendar) -> Date {
        let zone = calendar.timeZone
        let nowOffset = zone.secondsFromGMT(for: now)
        let firstOffset = zone.secondsFromGMT(for: first)
        let second = first.addingTimeInterval(TimeInterval(firstOffset - nowOffset))
        let sameWallTime = calendar.dateComponents([.hour, .minute], from: second)
            == calendar.dateComponents([.hour, .minute], from: first)
        return firstOffset != nowOffset && zone.secondsFromGMT(for: second) == nowOffset && sameWallTime ? second : first
    }

    private func wakeState(now: Date, calendar: Calendar) -> String {
        switch view.timers.wake {
        case nil:
            "Lampster wakes you with the light it had when you turned it off. Your Mac can be off by then."
        case .cancelling:
            "Off as soon as Lampster is connected; until then Lampster still wakes up."
        case .inLamp(let date):
            """
            Lampster wakes up \(day(date, now: now, calendar: calendar)) at \(clock(date, calendar)), \
            in \(left(until: date, from: now)).
            You can turn your Mac off. Make sure to turn off Lampster with the light you wish to wake up to.
            """
        case .pending(let date):
            """
            Lampster wakes up \(day(date, now: now, calendar: calendar)) at \(clock(date, calendar)), \
            in \(left(until: date, from: now)).
            Set as soon as Lampster is connected; until then, keep your Mac on.
            """
        }
    }

    private func sleepState(now: Date, calendar: Calendar) -> String {
        switch view.timers.sleep {
        case nil:
            "Lampster switches itself off after the time you choose, also with your Mac off."
        case .cancelling:
            "Cancelled as soon as Lampster is connected; until then Lampster still turns off."
        case .inLamp(let date):
            """
            Lampster turns off at \(clock(date, calendar)), in \(left(until: date, from: now)).
            You can turn your Mac off.
            """
        case .pending(let date):
            """
            Lampster turns off at \(clock(date, calendar)), in \(left(until: date, from: now)).
            Set as soon as Lampster is connected; until then, keep your Mac on.
            """
        }
    }
}

private func clock(_ date: Date, _ calendar: Calendar) -> String {
    ClockTime(of: date, calendar: calendar).text()
}

private func day(_ date: Date, now: Date, calendar: Calendar) -> String {
    calendar.isDate(date, inSameDayAs: now) ? "today" : "tomorrow"
}

/// Rounded up, so a timer due in 30 s still shows "1 min".
private func left(until date: Date, from now: Date) -> String {
    let minutes = max(0, Int((date.timeIntervalSince(now) / 60).rounded(.up)))
    return minutes < 60 ? "\(minutes) min" : "\(minutes / 60) h \(minutes % 60) min"
}
