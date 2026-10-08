import SwiftUI

/// The EMA's one setting, applied as soon as it is picked.
struct EMAPeriodPopover: View {
    @ObservedObject var viewModel: ChartViewModel
    var onStyleChanged: () -> Void = {}

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("EMA period").font(.headline)
            Picker(
                "Period",
                selection: Binding(
                    get: { viewModel.emaPeriod },
                    set: {
                        viewModel.emaPeriod = $0
                        onStyleChanged()
                    })
            ) {
                ForEach(Indicator.emaPeriods, id: \.self) { Text("\($0)").tag($0) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
        }
        .padding(16)
        .frame(width: 260)
    }
}
