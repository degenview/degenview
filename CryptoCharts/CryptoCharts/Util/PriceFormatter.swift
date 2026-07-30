import Foundation

/// Formatting utilities for chart price labels.
enum PriceFormatter {

    /// Format price with subscript zero-count for very small numbers (CoinMarketCap style).
    /// Examples: 0.00000278 → "0.0₅278", 45.23 → "45.23", 1,234,567 → "1,234,567"
    static func format(_ price: Double, decimalPlaces: Int? = nil) -> String {
        guard price > 0 else { return "0" }

        // Very small: CoinMarketCap subscript zero-count notation
        if price < Format.subscriptThreshold {
            var zeroCount = 0
            var scaled = price
            while scaled < Format.subscriptScaleTarget {
                scaled *= 10
                zeroCount += 1
            }
            let sigValue = Int(round(scaled * Double(Format.subscriptRoundingThreshold)))
            let sigStr: String
            if sigValue >= Format.subscriptRoundingThreshold {
                zeroCount = max(0, zeroCount - 1)
                sigStr = "100"
            } else {
                sigStr = String(format: "%03d", sigValue)
            }
            return "0.0\(zeroCount.subscriptUnicode)\(sigStr)"
        }

        // Large numbers: just grouped decimal
        if price >= Format.largeNumberThreshold {
            if let places = decimalPlaces {
                return price.formatted(.number.grouping(.automatic).precision(.fractionLength(places)))
            }
            return price.formatted(.number.grouping(.automatic).precision(.fractionLength(0)))
        }

        let digits: Int
        if let places = decimalPlaces {
            digits = places
        } else {
            digits = Self.autoDigits(for: price)
        }

        return price.formatted(
            .number
            .precision(.fractionLength(digits))
            .grouping(.automatic)
        )
    }

    private static func autoDigits(for price: Double) -> Int {
        for threshold in Format.priceDigitThresholds {
            if price >= threshold.threshold { return threshold.digits }
        }
        return Format.defaultPriceDigits
    }
}

extension Int {
    /// Unicode subscript digits: 0→₀, 1→₁, …, 9→₉
    var subscriptUnicode: String {
        String(self).map { char -> String in
            guard let digit = char.wholeNumberValue,
                  let scalar = UnicodeScalar(0x2080 + digit) else {
                return String(char)
            }
            return String(scalar)
        }.joined()
    }
}
