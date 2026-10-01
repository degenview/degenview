import SwiftUI

/// A script's `input.*` declarations as editable controls, sectioned by `group`.
///
/// Stateless: the caller owns the compiled schema and the override map, so the same view serves
/// a chart's settings sheet and the Script Manager preview.
struct PineInputsView: View {
    let schema: PineInputSchema
    /// Overrides by input id; an input with no entry shows its default.
    let values: [String: PineInputValue]
    let onChange: (PineInputValue, String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            ForEach(Array(Self.groups(of: schema).enumerated()), id: \.offset) { _, group in
                if let title = group.title {
                    Text(title)
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(.secondary)
                        .padding(.top, 6)
                }
                ForEach(group.inputs) { input in control(for: input) }
            }
        }
    }

    /// Inputs sectioned by their `group`, in the order each group first appears.
    static func groups(of schema: PineInputSchema) -> [(title: String?, inputs: [PineInputDefinition])] {
        var groups: [(title: String?, inputs: [PineInputDefinition])] = []
        for input in schema.inputs {
            if let index = groups.firstIndex(where: { $0.title == input.group }) {
                groups[index].inputs.append(input)
            } else {
                groups.append((input.group, [input]))
            }
        }
        return groups
    }

    private func set(_ value: PineInputValue, for input: PineInputDefinition) {
        onChange(value, input.id)
    }

    /// One script input as a card. The control sits on the right.
    @ViewBuilder private func control(for input: PineInputDefinition) -> some View {
        let title = input.title ?? input.id
        let current = values[input.id] ?? input.defaultValue
        switch (input.type, current) {
        case (.bool, .bool(let value)):
            SettingsCardRow(title: title, icon: "switch.2", hint: input.tooltip) {
                Toggle(
                    title,
                    isOn: Binding(
                        get: { value },
                        set: { set(.bool($0), for: input) })
                )
                .labelsHidden()
                .toggleStyle(.switch)
            }
        case (.time, .int(let value)):
            SettingsCardRow(title: title, icon: "calendar", hint: input.tooltip) {
                DatePicker(
                    title,
                    selection: Binding(
                        get: { Date(timeIntervalSince1970: Double(value) / 1000) },
                        set: { set(.int(Int($0.timeIntervalSince1970 * 1000)), for: input) }),
                    displayedComponents: [.date, .hourAndMinute]
                )
                .labelsHidden()
            }
        case (.int, .int(let value)):
            let lower = Int(input.minValue ?? Double(min(value, 1)))
            let upper = Int(input.maxValue ?? Double(max(value, 10_000)))
            SettingsCardRow(title: title, icon: "number", hint: input.tooltip) {
                HStack(spacing: 8) {
                    Text("\(value)")
                        .font(.body.monospacedDigit())
                        .foregroundStyle(.secondary)
                    Stepper(
                        title,
                        value: Binding(
                            get: { value },
                            set: { set(.int($0), for: input) }),
                        in: lower...upper,
                        step: Int(input.step ?? 1)
                    )
                    .labelsHidden()
                }
            }
        case (.float, .float(let value)):
            SettingsCardRow(title: title, icon: "number", hint: input.tooltip) {
                TextField(
                    title,
                    value: Binding(
                        get: { value },
                        set: { set(.float($0), for: input) }),
                    format: .number
                )
                .labelsHidden()
                .multilineTextAlignment(.trailing)
                .textFieldStyle(.roundedBorder)
                .frame(width: 100)
            }
        case (.string, .string(let value)) where input.options != nil:
            SettingsCardRow(title: title, icon: "list.bullet", hint: input.tooltip) {
                Picker(
                    title,
                    selection: Binding(
                        get: { value },
                        set: { set(.string($0), for: input) })
                ) {
                    ForEach(Array((input.options ?? []).enumerated()), id: \.offset) { index, option in
                        if case .string(let text) = option {
                            Text(input.optionTitles.flatMap { $0.indices.contains(index) ? $0[index] : nil } ?? text)
                                .tag(text)
                        }
                    }
                }
                .pickerStyle(.menu)
                .labelsHidden()
            }
        case (.string, .string(let value)):
            SettingsCardRow(title: title, icon: "textformat", hint: input.tooltip) {
                TextField(
                    title,
                    text: Binding(
                        get: { value },
                        set: { set(.string($0), for: input) })
                )
                .labelsHidden()
                .textFieldStyle(.roundedBorder)
                .frame(width: 140)
            }
        case (.color, .color(let value)):
            SettingsCardRow(title: title, icon: "paintpalette", hint: input.tooltip) {
                ColorPicker(
                    title,
                    selection: Binding(
                        get: { Color(pineRGBA: value) },
                        set: {
                            guard let rgba = $0.pineRGBA else { return }
                            set(.color(rgba), for: input)
                        }), supportsOpacity: true
                )
                .labelsHidden()
            }
        case (.string, .source(let value)):
            SettingsCardRow(title: title, icon: "chart.xyaxis.line", hint: input.tooltip) {
                Picker(
                    title,
                    selection: Binding(
                        get: { value },
                        set: { set(.source($0), for: input) })
                ) {
                    ForEach(["open", "high", "low", "close", "volume"], id: \.self) {
                        Text($0.capitalized).tag($0)
                    }
                }
                .pickerStyle(.menu)
                .labelsHidden()
            }
        default: EmptyView()
        }
    }
}
