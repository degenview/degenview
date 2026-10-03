import Foundation

extension PineSymbolMetadata {
    /// `color.*` functions. The named colors are constants in the catalog.
    static let colorEntries = """
        color.new(color: series color, transp: series float = 0) -> series color :: Applies transparency to a color.
        color.rgb(red: series float, green: series float, blue: series float, transp: series float = 0) -> series color :: Builds a color from components.
        color.from_gradient(value: series float, bottom_value: series float, top_value: series float, bottom_color: series color, top_color: series color) -> series color :: Interpolates between two colors.
        color.r(color: series color) -> series float :: Red component, 0 to 255.
        color.g(color: series color) -> series float :: Green component, 0 to 255.
        color.b(color: series color) -> series float :: Blue component, 0 to 255.
        color.t(color: series color) -> series float :: Transparency, 0 to 100.
        """
}
