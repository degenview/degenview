import Foundation

extension Character {
    /// Letters, digits and `_`: what counts as part of a word in code.
    var isWordCharacter: Bool { isLetter || isNumber || self == "_" }
}
