import Foundation

/// One `switch` arm; a nil condition is the default (`=> value`) arm.
struct PineSwitchArm: Sendable {
    var condition: PineExpression?
    var body: [PineStatement]
}
