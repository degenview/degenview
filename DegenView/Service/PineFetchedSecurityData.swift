import Foundation

/// The candles fetched for a script's `request.security` calls, served to the session by series.
struct PineFetchedSecurityData: PineSecurityDataProvider {
    var series: [PineSecurityKey: [KlineData]] = [:]

    func candles(for key: PineSecurityKey) -> [KlineData]? { series[key] }
}
