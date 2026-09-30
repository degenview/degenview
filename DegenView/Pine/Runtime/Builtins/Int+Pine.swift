import Foundation

extension Int {
    /// Floats beyond this magnitude stop representing every integer exactly (2^53 ≈ 9e15).
    static let pineExactLimit = 9e15

    /// Converts a script-supplied float without trapping: nil when it is not finite or too
    /// large to be an exact integer. Truncates toward zero.
    init?(pine value: Double) {
        guard value.isFinite, abs(value) < Self.pineExactLimit else { return nil }
        self = Int(value)
    }
}
