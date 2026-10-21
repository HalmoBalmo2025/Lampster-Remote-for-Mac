import SwiftUI

/// The lamp's own timers: they run without the Mac.
struct TimersPage: View {
    @ObservedObject var model: Model
    /// The field typed into; a click anywhere on its row puts it there.
    @FocusState var focused: TimerField?

    var body: some View {
        // Refreshes what is left every half minute.
        TimelineView(.periodic(from: .now, by: 30)) { context in
            VStack(alignment: .leading, spacing: 14) {
                wake(now: context.date)
                Divider()
                sleep(now: context.date)
            }
        }
        .onChange(of: focused) { left, _ in
            if let left { model.leave(left) }
        }
        // Another tab: the field's cursor goes without a change of focus.
        .onDisappear(perform: model.leaveBoth)
    }

    func wake(now: Date) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            // Title and switch aligned at the top, as on the settings page, so the
            // title starts at the height of every page's first line.
            HStack(alignment: .top) {
                Text("Wake-up light").font(.subheadline.weight(.medium))
                Spacer()
                Toggle("", isOn: Binding(get: { model.wakeIsOn }, set: model.setWake))
                    .toggleStyle(.switch)
                    .labelsHidden()
            }
            HStack(spacing: 8) {
                Image(systemName: "sunrise")
                    .foregroundStyle(.secondary)
                    .frame(width: 28)
                TextField(Model.firstWakeTime.text(), text: $model.wakeField)
                    .textFieldStyle(.plain)
                    .focused($focused, equals: .wake)
                    .font(.system(size: 15, weight: .medium).monospacedDigit())
                    .onSubmit {
                        model.submit(.wake)
                        endEditing()
                    }
                    .help("The time to wake up at; Return sets the wake-up light")
                if let day = model.fieldDay(now: now) {
                    Text(day)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .padding(.trailing, 8)
                }
            }
            .frame(minHeight: 28)
            .well { focused = .wake }
            captions(model.wakeCaptions(now: now))
        }
    }

    func sleep(now: Date) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            // Title and switch aligned at the top, as on the settings page, so the
            // title starts at the height of every page's first line.
            HStack(alignment: .top) {
                Text("Sleep timer").font(.subheadline.weight(.medium))
                Spacer()
                Toggle("", isOn: Binding(get: { model.sleepIsOn }, set: model.setSleep))
                    .toggleStyle(.switch)
                    .labelsHidden()
            }
            HStack(spacing: 8) {
                Image(systemName: "moon.zzz")
                    .foregroundStyle(.secondary)
                    .frame(width: 28)
                // As wide as what is typed, so "min" follows the number.
                ZStack(alignment: .leading) {
                    Text(model.sleepField.isEmpty ? String(Model.firstSleepMinutes) : model.sleepField).hidden()
                    TextField(String(Model.firstSleepMinutes), text: $model.sleepField)
                        .textFieldStyle(.plain)
                        .focused($focused, equals: .sleep)
                        .onSubmit {
                            model.submit(.sleep)
                            endEditing()
                        }
                        .help("The minutes until Lampster turns off; Return starts the timer")
                }
                .font(.system(size: 15, weight: .medium).monospacedDigit())
                .fixedSize()
                Text("min")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Spacer()
            }
            .frame(minHeight: 28)
            .well { focused = .sleep }
            captions(model.sleepCaptions(now: now))
        }
    }

    /// What the timer does in grey, why the field's text cannot be set in
    /// red under it.
    func captions(_ captions: [TimerCaption]) -> some View {
        ForEach(captions, id: \.self) { caption in
            switch caption {
            case .state(let text):
                Text(text)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            case .hint(let text):
                Text(text)
                    .font(.caption)
                    .foregroundStyle(.red)
            }
        }
    }
}
