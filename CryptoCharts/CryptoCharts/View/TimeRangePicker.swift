import SwiftUI

struct TimeRangePicker: View {
    @Binding var selectedRange: TimeRange

    var body: some View {
        Picker("Time Range", selection: $selectedRange) {
            ForEach(TimeRange.allCases) { range in
                Text(range.rawValue).tag(range)
            }
        }
        .pickerStyle(.segmented)
        .labelsHidden()
    }
}

#Preview {
    TimeRangePicker(selectedRange: .constant(.oneDay))
        .padding()
}
