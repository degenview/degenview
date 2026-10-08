import SwiftUI

/// A bookmark turned on its side: a flat end with softly rounded corners, and a V cut into the other end.
/// By default the notched tail is on the left, against the panel's edge, and the flat end faces the logo.
/// `tailPointsRight` puts the notch on the right instead.
struct WatchlistFlagShape: Shape {
    var tailPointsRight = false
    /// How deep the V is cut into the tail.
    var notchDepth: CGFloat = 3
    var cornerRadius: CGFloat = 1.5

    func path(in rect: CGRect) -> Path {
        let width = rect.width
        let height = rect.height
        let notch = min(notchDepth, width / 2)
        let radius = min(cornerRadius, min(width, height) / 2)

        var path = Path()
        path.move(to: CGPoint(x: radius, y: 0))
        path.addLine(to: CGPoint(x: width, y: 0))
        path.addLine(to: CGPoint(x: width - notch, y: height / 2))
        path.addLine(to: CGPoint(x: width, y: height))
        path.addLine(to: CGPoint(x: radius, y: height))
        path.addQuadCurve(to: CGPoint(x: 0, y: height - radius), control: CGPoint(x: 0, y: height))
        path.addLine(to: CGPoint(x: 0, y: radius))
        path.addQuadCurve(to: CGPoint(x: radius, y: 0), control: CGPoint(x: 0, y: 0))
        path.closeSubpath()

        let placement =
            tailPointsRight
            ? CGAffineTransform(translationX: rect.minX, y: rect.minY)
            : CGAffineTransform(translationX: rect.maxX, y: rect.minY).scaledBy(x: -1, y: 1)
        return path.applying(placement)
    }
}
