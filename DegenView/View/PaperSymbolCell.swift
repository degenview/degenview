import SwiftUI

/// A table cell naming an instrument: its artwork and symbol.
struct PaperSymbolCell: View {
    let instrument: PaperInstrument

    var body: some View {
        HStack(spacing: 7) {
            PaperInstrumentIcon(instrument: instrument, size: 18)
            Text(instrument.symbol).fontWeight(.semibold).lineLimit(1)
        }
        .help(instrument.displayName)
    }
}
