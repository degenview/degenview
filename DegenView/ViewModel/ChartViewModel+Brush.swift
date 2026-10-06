import CoreGraphics
import Foundation

/// Freehand brush strokes: capture, commit, hit testing, move, style and delete.
///
/// Persistence and undo happen once per gesture — on release — never per pointer event.
extension ChartViewModel {
    // MARK: - Drawing a stroke

    /// Pointer went down on empty chart: start a stroke at `point`.
    func beginBrushDraft(at point: CGPoint, in plot: ChartPlot, style: BrushStyle = .default) {
        selectedBrushID = nil
        editingBrushID = nil
        hoveredBrushID = nil
        brushDraft = BrushDraft(points: [anchor(at: point, in: plot)], style: style)
        lastBrushSample = point
    }

    /// Keep a drag sample only once the pointer has travelled `sampleDistance` from the
    /// last kept one, so a 1000 Hz mouse doesn't write a thousand points a second.
    func appendBrushPoint(at point: CGPoint, in plot: ChartPlot) {
        guard var draft = brushDraft, let last = lastBrushSample,
            draft.points.count < BrushTuning.maxDraftPoints,
            hypot(point.x - last.x, point.y - last.y) >= BrushTuning.sampleDistance
        else { return }
        draft.points.append(anchor(at: point, in: plot))
        brushDraft = draft
        lastBrushSample = point
    }

    /// Release: simplify in screen space, persist once, register one undo step.
    ///
    /// The pointer's final position is always kept, so the stroke ends where the pointer
    /// did. A stroke that never travelled becomes a one-point dot. Not selected afterwards:
    /// the tool stays armed and selecting would open the editor under the next stroke.
    @discardableResult
    func commitBrushDraft(at point: CGPoint?, in plot: ChartPlot) -> Bool {
        guard var draft = brushDraft else { return false }
        defer { cancelBrushDraft() }

        if let point, let last = lastBrushSample, point != last,
            draft.points.count < BrushTuning.maxDraftPoints
        {
            draft.points.append(anchor(at: point, in: plot))
        }

        let screen = projectedPoints(draft.points, in: plot)
        let points: [TrendAnchor]
        if BrushGeometry.length(of: screen) < BrushTuning.dotMaxTravel {
            points = [draft.points[0]]
        } else {
            let kept = BrushGeometry.simplify(screen, tolerance: BrushTuning.simplifyTolerance)
            points = kept.map { draft.points[$0] }
        }

        let stroke = BrushDrawing(points: points, style: draft.style)
        let index = brushes.count
        brushes.append(stroke)
        persistBrushes()
        drawingUndoCoordinator?.recordBrush(
            instrument: drawingInstrument, before: nil, beforeIndex: index, after: stroke,
            afterIndex: index, actionName: "Add Brush Stroke")
        return true
    }

    /// Drops the stroke without a write and without an undo step.
    func cancelBrushDraft() {
        brushDraft = nil
        lastBrushSample = nil
    }

    /// Everything transient: the draft, hover, selection and editor. Used when the
    /// instrument or the tool changes.
    func resetBrushState() {
        cancelBrushDraft()
        hoveredBrushID = nil
        selectedBrushID = nil
        editingBrushID = nil
        brushSettingsOriginal = nil
    }

    // MARK: - Hit testing

    /// The topmost stroke under `point` — newest first, matching what the renderer paints
    /// last. Geometry is screen-space: the same projection the renderer used.
    func brushHit(at point: CGPoint, in plot: ChartPlot) -> UUID? {
        guard !visibleKlines.isEmpty else { return nil }
        for stroke in brushes.reversed() {
            let screen = projectedPoints(stroke.points, in: plot)
            if BrushGeometry.hit(
                point: point, in: screen, halfWidth: CGFloat(stroke.lineWidth) / 2,
                tolerance: Drawing.hitTolerance)
            {
                return stroke.id
            }
        }
        return nil
    }

    func setBrushHover(_ id: UUID?) {
        if hoveredBrushID != id { hoveredBrushID = id }
    }

    // MARK: - Moving

    /// Shifts the whole stroke by a screen-space `delta`.
    ///
    /// Always computed from `original`, the stroke as it was at mouse-down, so there is
    /// no drift from accumulating per-event deltas. Each point goes through the chart's
    /// projection and back — never `price += c` — so it keeps working if the vertical
    /// scale stops being linear.
    func translateBrush(original: BrushDrawing, by delta: CGSize, in plot: ChartPlot) {
        guard !original.isLocked, let index = brushes.firstIndex(where: { $0.id == original.id }) else {
            return
        }
        let moved = projectedPoints(original.points, in: plot).map {
            anchor(at: CGPoint(x: $0.x + delta.width, y: $0.y + delta.height), in: plot)
        }
        brushes[index].points = moved
        brushes[index].updatedAt = Date()
    }

    func commitBrushDrag(original: BrushDrawing) {
        guard let index = brushes.firstIndex(where: { $0.id == original.id }) else { return }
        persistBrushes()
        drawingUndoCoordinator?.recordBrush(
            instrument: drawingInstrument, before: original, beforeIndex: index,
            after: brushes[index], afterIndex: index, actionName: "Move Brush Stroke")
    }

    // MARK: - Style

    /// Replace a stroke. Pass `recordUndo: false` while a slider is live, then bracket the
    /// gesture with `beginBrushSettingsEdit` / `endBrushSettingsEdit` for one undo step.
    func updateBrush(_ stroke: BrushDrawing, recordUndo: Bool = true) {
        guard let index = brushes.firstIndex(where: { $0.id == stroke.id }) else { return }
        let previous = brushes[index]
        var updated = stroke
        updated.updatedAt = Date()
        brushes[index] = updated
        persistBrushes()
        if recordUndo {
            drawingUndoCoordinator?.recordBrush(
                instrument: drawingInstrument, before: previous, beforeIndex: index, after: updated,
                afterIndex: index, actionName: "Edit Brush Stroke")
        }
    }

    func beginBrushSettingsEdit(id: UUID) {
        brushSettingsOriginal = brushes.first { $0.id == id }
    }

    func endBrushSettingsEdit(id: UUID) {
        defer { brushSettingsOriginal = nil }
        guard let before = brushSettingsOriginal,
            let index = brushes.firstIndex(where: { $0.id == id })
        else { return }
        drawingUndoCoordinator?.recordBrush(
            instrument: drawingInstrument, before: before, beforeIndex: index,
            after: brushes[index], afterIndex: index, actionName: "Edit Brush Stroke")
    }

    // MARK: - Delete

    @discardableResult
    func removeSelectedBrush() -> Bool {
        guard let id = selectedBrushID else { return false }
        return removeBrush(id: id)
    }

    @discardableResult
    func removeBrush(id: UUID) -> Bool {
        guard let index = brushes.firstIndex(where: { $0.id == id }) else { return false }
        let stroke = brushes.remove(at: index)
        if selectedBrushID == id { selectedBrushID = nil }
        if editingBrushID == id { editingBrushID = nil }
        if hoveredBrushID == id { hoveredBrushID = nil }
        persistBrushes()
        drawingUndoCoordinator?.recordBrush(
            instrument: drawingInstrument, before: stroke, beforeIndex: index, after: nil,
            afterIndex: index, actionName: "Delete Brush Stroke")
        return true
    }

    func persistBrushes() {
        drawingStore.save(brushes, ticker: ticker, source: source)
    }

    // MARK: - Projection

    /// The anchors under the chart's current geometry.
    func projectedPoints(_ anchors: [TrendAnchor], in plot: ChartPlot) -> [CGPoint] {
        let points = visibleKlines
        guard !points.isEmpty else { return [] }
        let slot = plot.slotWidth(forCount: points.count)
        return anchors.map { plot.position(of: $0, points: points, slotWidth: slot) }
    }
}
