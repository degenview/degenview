import Foundation

/// An enumeration of Pine builtin constants that share a namespace, e.g. `size.tiny`.
/// Raw values are the name after the namespace prefix.
protocol PineNamedConstant: RawRepresentable, Sendable where RawValue == String {
    /// The namespace including its separator, e.g. `"size."` or `"line.style_"`.
    static var pinePrefix: String { get }
}

extension PineNamedConstant {
    init?(pineName: String?) {
        guard let pineName, pineName.hasPrefix(Self.pinePrefix) else { return nil }
        self.init(rawValue: String(pineName.dropFirst(Self.pinePrefix.count)))
    }

    /// `absent` when the script passed nothing, `unknown` for a name this enum does not list.
    static func parse(_ pineName: String?, absent: Self, unknown: Self? = nil) -> Self {
        guard let pineName else { return absent }
        return Self(pineName: pineName) ?? unknown ?? absent
    }
}
