import Foundation

/// Buckets time-stamped items under "Today", "Yesterday" and dated headings, newest day first.
enum AlertDayGrouping {
    struct Section<Item>: Identifiable {
        let day: Date
        let title: String
        let items: [Item]
        var id: Date { day }
    }

    /// Items inside a day keep the newest-first order of the whole list; an empty input gives no sections.
    static func sections<Item>(
        _ items: [Item], date: (Item) -> Date, now: Date = Date(), calendar: Calendar = .current,
        locale: Locale = .current
    ) -> [Section<Item>] {
        let today = calendar.startOfDay(for: now)
        let ordered = items.sorted { date($0) > date($1) }
        let byDay = Dictionary(grouping: ordered) { calendar.startOfDay(for: date($0)) }
        return byDay.keys.sorted(by: >).map { day in
            let heading = title(for: day, today: today, calendar: calendar, locale: locale)
            return Section(day: day, title: heading, items: byDay[day] ?? [])
        }
    }

    private static func title(for day: Date, today: Date, calendar: Calendar, locale: Locale) -> String {
        if day == today { return "Today" }
        if let yesterday = calendar.date(byAdding: .day, value: -1, to: today), day == yesterday { return "Yesterday" }
        var style = Date.FormatStyle(locale: locale, calendar: calendar, timeZone: calendar.timeZone)
            .weekday(.wide).month(.wide).day()
        if calendar.component(.year, from: day) != calendar.component(.year, from: today) { style = style.year() }
        return day.formatted(style)
    }
}
