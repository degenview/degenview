import SwiftUI

/// One script's editing surface: the code editor and the live preview chart, split along the axis
/// the user chose. Rebuilt per script, so the editor starts clean; the preview's model outlives it.
struct ScriptWorkspaceView: View {
    let type: ScriptType
    @ObservedObject var preview: ScriptPreviewViewModel
    @StateObject private var editor: ScriptEditorViewModel
    private let scriptID: UUID

    @AppStorage("scriptManager.chartPosition") private var position: ChartPosition = .left
    @AppStorage("scriptManager.chartVisible") private var chartVisible = true
    // One preferred length per axis: a chart that is 480 pt wide beside the code would be absurdly
    // tall above it.
    @AppStorage("scriptManager.chartWidth") private var chartWidth = 0.0
    @AppStorage("scriptManager.chartHeight") private var chartHeight = 0.0

    init(scriptID: UUID, type: ScriptType, preview: ScriptPreviewViewModel) {
        self.scriptID = scriptID
        self.type = type
        self.preview = preview
        _editor = StateObject(wrappedValue: ScriptEditorViewModel(scriptID: scriptID))
    }

    private var chartLength: Binding<Double> {
        position.axis == .horizontal ? $chartWidth : $chartHeight
    }

    var body: some View {
        SplitContainer(
            axis: position.axis, secondaryFirst: position.chartFirst, secondaryLength: chartLength,
            isSecondaryVisible: chartVisible,
            minPrimary: position.axis == .horizontal ? 320 : 160,
            minSecondary: position.axis == .horizontal ? 320 : 220,
            defaultFraction: 0.5
        ) {
            ScriptEditorView(model: editor)
        } secondary: {
            ScriptPreviewPane(preview: preview, editor: editor)
        }
        .onAppear {
            preview.bind(scriptID: scriptID, type: type)
            preview.sourceChanged(editor.source)
            preview.setPaneVisible(chartVisible)
        }
        .onDisappear { preview.paneDisappeared(scriptID: scriptID) }
        .onChange(of: type) { _, newType in preview.bind(scriptID: scriptID, type: newType) }
        .onChange(of: chartVisible) { _, visible in preview.setPaneVisible(visible) }
        // Unsaved edits included: the chart answers what is on screen, not what is on disk.
        .onChange(of: editor.source) { _, source in preview.sourceChanged(source) }
    }
}
