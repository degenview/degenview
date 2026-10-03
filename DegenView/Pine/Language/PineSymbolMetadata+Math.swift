import Foundation

extension PineSymbolMetadata {
    /// `math.*`.
    static let mathEntries = """
        math.abs(number: series float) -> series float :: Absolute value.
        math.acos(angle: series float) -> series float :: Arccosine, in radians.
        math.asin(angle: series float) -> series float :: Arcsine, in radians.
        math.atan(angle: series float) -> series float :: Arctangent, in radians.
        math.avg(number0: series float, number1: series float) -> series float :: Average of the arguments.
        math.ceil(number: series float) -> series int :: Smallest integer not below number.
        math.cos(angle: series float) -> series float :: Cosine of an angle in radians.
        math.exp(number: series float) -> series float :: e raised to number.
        math.floor(number: series float) -> series int :: Largest integer not above number.
        math.log(number: series float) -> series float :: Natural logarithm.
        math.log10(number: series float) -> series float :: Base-10 logarithm.
        math.max(number0: series float, number1: series float) -> series float :: Largest of the arguments.
        math.min(number0: series float, number1: series float) -> series float :: Smallest of the arguments.
        math.pow(base: series float, exponent: series float) -> series float :: base raised to exponent.
        math.round(number: series float) -> series int :: Rounds to the nearest integer.
        math.round(number: series float, precision: series int) -> series float :: Rounds to a number of decimal places.
        math.round_to_mintick(number: series float) -> series float :: Rounds to the symbol's minimum tick.
        math.sign(number: series float) -> series float :: -1, 0 or 1 by the sign of number.
        math.sin(angle: series float) -> series float :: Sine of an angle in radians.
        math.sqrt(number: series float) -> series float :: Square root.
        math.sum(source: series float, length: series int) -> series float :: Sum over the last length bars.
        math.tan(angle: series float) -> series float :: Tangent of an angle in radians.
        math.todegrees(radians: series float) -> series float :: Radians to degrees.
        math.toradians(degrees: series float) -> series float :: Degrees to radians.
        """
}
