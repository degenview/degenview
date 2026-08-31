import Foundation

/// One ordered column in the chart grid. Chart membership is keyed by the
/// stable identity persisted in `TickerConfig`, not by a ticker symbol.
struct ChartColumn: Identifiable, Codable, Equatable {
    var id: UUID
    var chartIDs: [UUID]

    init(id: UUID = UUID(), chartIDs: [UUID] = []) {
        self.id = id
        self.chartIDs = chartIDs
    }

    /// Resolve persisted columns against the charts that actually exist. Old
    /// documents have no columns, so reproduce the former two-column row-major
    /// grid. Corrupt/stale membership is repaired without dropping charts.
    static func resolved(_ persisted: [ChartColumn]?, chartIDs: [UUID]) -> [ChartColumn] {
        guard !chartIDs.isEmpty else { return [] }

        guard let persisted, !persisted.isEmpty else {
            let count = min(2, chartIDs.count)
            var columns = (0..<count).map { _ in ChartColumn() }
            for (index, chartID) in chartIDs.enumerated() {
                columns[index % count].chartIDs.append(chartID)
            }
            return columns
        }

        let valid = Set(chartIDs)
        var seen = Set<UUID>()
        var columns = persisted.compactMap { column -> ChartColumn? in
            var repaired = column
            repaired.chartIDs = column.chartIDs.filter { valid.contains($0) && seen.insert($0).inserted }
            return repaired.chartIDs.isEmpty ? nil : repaired
        }

        for chartID in chartIDs where !seen.contains(chartID) {
            if columns.count < min(2, chartIDs.count) {
                columns.append(ChartColumn(chartIDs: [chartID]))
            } else if let shortest = columns.indices.min(by: { lhs, rhs in
                let left = columns[lhs].chartIDs.count
                let right = columns[rhs].chartIDs.count
                return left == right ? lhs < rhs : left < right
            }) {
                columns[shortest].chartIDs.append(chartID)
            } else {
                columns.append(ChartColumn(chartIDs: [chartID]))
            }
        }
        return columns
    }
}
