import Foundation

struct PortfolioValueChange: Equatable {
    enum Direction: Equatable {
        case up
        case down
        case unchanged
    }

    let amount: Decimal
    let percentage: Decimal?

    init(from startingValue: Decimal, to endingValue: Decimal) {
        amount = endingValue - startingValue
        percentage = startingValue == 0 ? nil : amount / startingValue
    }

    var direction: Direction {
        if amount > 0 { return .up }
        if amount < 0 { return .down }
        return .unchanged
    }
}
