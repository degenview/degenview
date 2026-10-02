import XCTest

@testable import DegenView

final class AlertDayGroupingTests: XCTestCase {
    private var calendar: Calendar = {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        return calendar
    }()
    private let locale = Locale(identifier: "en_US")
    private lazy var now = calendar.date(from: DateComponents(year: 2026, month: 10, day: 2, hour: 12))!

    private func sections(_ dates: [Date]) -> [AlertDayGrouping.Section<Date>] {
        AlertDayGrouping.sections(dates, date: { $0 }, now: now, calendar: calendar, locale: locale)
    }

    func testNoItemsMakeNoSections() {
        XCTAssertTrue(sections([]).isEmpty)
    }

    func testTodayAndYesterdayAreNamed() {
        let today = now.addingTimeInterval(-3600)
        let earlierToday = now.addingTimeInterval(-7200)
        let yesterday = calendar.date(from: DateComponents(year: 2026, month: 10, day: 1, hour: 23))!
        let result = sections([yesterday, earlierToday, today])
        XCTAssertEqual(result.map(\.title), ["Today", "Yesterday"])
        XCTAssertEqual(result.map { $0.items.count }, [2, 1])
    }

    func testNewestComesFirstWithinAndAcrossDays() {
        let a = now.addingTimeInterval(-60)
        let b = now.addingTimeInterval(-3600)
        let old = calendar.date(from: DateComponents(year: 2026, month: 9, day: 28, hour: 9))!
        let result = sections([old, b, a])
        XCTAssertEqual(result.first?.items, [a, b])
        XCTAssertEqual(result.last?.items, [old])
    }

    func testOlderDaysCarryTheirDateAndOtherYearsTheYear() {
        let thisYear = calendar.date(from: DateComponents(year: 2026, month: 9, day: 28, hour: 9))!
        let lastYear = calendar.date(from: DateComponents(year: 2025, month: 12, day: 31, hour: 9))!
        let titles = sections([thisYear, lastYear]).map(\.title)
        XCTAssertEqual(titles.count, 2)
        XCTAssertTrue(titles[0].contains("September") && titles[0].contains("28"), titles[0])
        XCTAssertFalse(titles[0].contains("2026"), titles[0])
        XCTAssertTrue(titles[1].contains("2025"), titles[1])
    }
}
