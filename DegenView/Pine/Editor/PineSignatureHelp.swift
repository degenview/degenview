import Foundation

/// What the signature popup shows: the callable's forms and where the caret is in the active one.
/// Deliberately not a completion item: it follows the caret through the arguments, it is not
/// something to accept.
struct PineSignatureHelp: Equatable {
    /// Every form of the callee (overloads), best match first.
    let signatures: [PineSymbolSignature]
    /// Index into `signatures` of the form the arguments so far fit.
    let activeSignature: Int
    /// Index into the active signature's parameters of the argument under the caret.
    let activeParameter: Int?
    /// The callee as written: `ta.sma`.
    let callee: String
    /// Editor offset of the call's opening parenthesis; stable while the call is typed.
    let opener: Int

    var signature: PineSymbolSignature { signatures[activeSignature] }
}
