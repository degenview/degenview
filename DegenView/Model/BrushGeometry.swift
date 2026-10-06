import SwiftUI

/// Screen-space maths for brush strokes. Pure — points in, points or a path out — so it
/// is testable without a chart. Everything works on projected `CGPoint`s: the pixel
/// tolerances mean the same thing on DOGE as on BTC, which a price-unit tolerance would not.
enum BrushGeometry {
    // MARK: - Simplification

    /// Indices of the points Ramer–Douglas–Peucker keeps, ascending. First and last are
    /// always kept. Returning indices rather than points lets the caller keep the original
    /// financial coordinates instead of re-deriving them. Iterative, so a long stroke
    /// can't overflow the stack.
    static func simplify(_ points: [CGPoint], tolerance: CGFloat) -> [Int] {
        guard points.count > 2 else { return Array(points.indices) }
        var keep = [Bool](repeating: false, count: points.count)
        keep[0] = true
        keep[points.count - 1] = true

        var stack = [(0, points.count - 1)]
        while let (first, last) = stack.popLast() {
            guard last - first > 1 else { continue }
            var farthest = first
            var farthestDistance: CGFloat = 0
            for index in (first + 1)..<last {
                let d = distance(from: points[index], toSegmentFrom: points[first], to: points[last])
                if d > farthestDistance {
                    farthestDistance = d
                    farthest = index
                }
            }
            if farthestDistance > tolerance {
                keep[farthest] = true
                stack.append((first, farthest))
                stack.append((farthest, last))
            }
        }
        return points.indices.filter { keep[$0] }
    }

    // MARK: - Hit testing

    /// Shortest distance from a point to a segment.
    static func distance(from point: CGPoint, toSegmentFrom start: CGPoint, to end: CGPoint) -> CGFloat {
        let dx = end.x - start.x
        let dy = end.y - start.y
        let lengthSquared = dx * dx + dy * dy
        guard lengthSquared > 0 else { return hypot(point.x - start.x, point.y - start.y) }
        let projection = ((point.x - start.x) * dx + (point.y - start.y) * dy) / lengthSquared
        let t = projection.clamped(to: 0...1)
        let closestX = start.x + t * dx
        let closestY = start.y + t * dy
        return hypot(point.x - closestX, point.y - closestY)
    }

    static func bounds(of points: [CGPoint]) -> CGRect? {
        guard let first = points.first else { return nil }
        var minX = first.x
        var maxX = first.x
        var minY = first.y
        var maxY = first.y
        for point in points.dropFirst() {
            minX = min(minX, point.x)
            maxX = max(maxX, point.x)
            minY = min(minY, point.y)
            maxY = max(maxY, point.y)
        }
        return CGRect(x: minX, y: minY, width: maxX - minX, height: maxY - minY)
    }

    /// Whether `point` lies on the stroke through `points`: within half the stroke width
    /// plus `tolerance` of any segment. The expanded bounding box rejects a pointer that is
    /// nowhere near before any segment is measured.
    static func hit(point: CGPoint, in points: [CGPoint], halfWidth: CGFloat, tolerance: CGFloat) -> Bool {
        guard let box = bounds(of: points) else { return false }
        let reach = halfWidth + tolerance
        guard box.insetBy(dx: -reach, dy: -reach).contains(point) else { return false }
        if points.count == 1 {
            return hypot(point.x - points[0].x, point.y - points[0].y) <= reach
        }
        for index in 1..<points.count
        where distance(from: point, toSegmentFrom: points[index - 1], to: points[index]) <= reach {
            return true
        }
        return false
    }

    // MARK: - Smoothing

    /// A path through `points` that reads as a pen stroke rather than a polyline.
    ///
    /// Midpoint quadratic smoothing: the curve runs between segment midpoints with each
    /// vertex as the control point. A quadratic never leaves the hull of its three points,
    /// so unlike a Catmull-Rom spline it cannot overshoot. A vertex that turns sharply
    /// (more than `cornerAngle`) is kept as a real corner, so a V or a zigzag stays sharp.
    /// Render-only: the stored points are never touched.
    static func smoothedPath(_ points: [CGPoint], cornerAngle: CGFloat = BrushTuning.cornerAngle) -> Path {
        let points = deduplicated(points)
        var path = Path()
        guard let first = points.first else { return path }
        path.move(to: first)
        guard points.count > 1 else { return path }
        guard points.count > 2 else {
            path.addLine(to: points[1])
            return path
        }

        for index in 1..<(points.count - 1) {
            let vertex = points[index]
            let middle = midpoint(vertex, points[index + 1])
            if turnAngle(points[index - 1], vertex, points[index + 1]) > cornerAngle {
                path.addLine(to: vertex)
                path.addLine(to: middle)
            } else {
                path.addQuadCurve(to: middle, control: vertex)
            }
        }
        path.addLine(to: points[points.count - 1])
        return path
    }

    /// Radians the direction changes at `vertex`: 0 for a straight run, π for a U-turn.
    static func turnAngle(_ before: CGPoint, _ vertex: CGPoint, _ after: CGPoint) -> CGFloat {
        let inX = vertex.x - before.x
        let inY = vertex.y - before.y
        let outX = after.x - vertex.x
        let outY = after.y - vertex.y
        let lengths = hypot(inX, inY) * hypot(outX, outY)
        guard lengths > 0 else { return 0 }
        let cosine = ((inX * outX + inY * outY) / lengths).clamped(to: -1...1)
        return acos(cosine)
    }

    /// Total length of the polyline.
    static func length(of points: [CGPoint]) -> CGFloat {
        guard points.count > 1 else { return 0 }
        return (1..<points.count).reduce(0) { total, index in
            total + hypot(points[index].x - points[index - 1].x, points[index].y - points[index - 1].y)
        }
    }

    private static func midpoint(_ a: CGPoint, _ b: CGPoint) -> CGPoint {
        CGPoint(x: (a.x + b.x) / 2, y: (a.y + b.y) / 2)
    }

    private static func deduplicated(_ points: [CGPoint]) -> [CGPoint] {
        var result: [CGPoint] = []
        result.reserveCapacity(points.count)
        for point in points where result.last != point { result.append(point) }
        return result
    }
}
