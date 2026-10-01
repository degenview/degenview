import AppKit

/// Maps each syntax category to a color. The system colors adapt to light and dark appearance.
struct PineSyntaxTheme {
    private let colors: [PineSyntaxCategory: NSColor]

    init(colors: [PineSyntaxCategory: NSColor]) {
        self.colors = colors
    }

    /// `nil` leaves the text in the editor's default color.
    func color(for category: PineSyntaxCategory) -> NSColor? { colors[category] }

    static let standard = PineSyntaxTheme(colors: [
        .keyword: .systemPurple,
        .type: .systemIndigo,
        .builtinNamespace: .systemTeal,
        .builtinFunction: .systemBlue,
        .builtinVariable: .systemGreen,
        .builtinConstant: .systemMint,
        .number: .systemOrange,
        .string: .systemRed,
        .comment: .secondaryLabelColor,
        .annotation: .systemBrown,
        .operator: .systemGray,
        .colorLiteral: .systemPink,
    ])
}
