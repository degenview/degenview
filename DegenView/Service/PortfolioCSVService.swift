import Foundation

struct PortfolioCSVPreview: Identifiable {
    let id = UUID()
    var transactions: [PortfolioTransaction]
    var errors: [String]
    var warnings: [String]
    var isValid: Bool { errors.isEmpty }
}

struct CoinMarketCapCSVRow: Identifiable {
    let id: String
    let line: Int
    let timestamp: Date
    let token: String
    let type: PortfolioTransactionType
    let price: Decimal
    let amount: Decimal
    let totalValue: Decimal
    let fee: Decimal
    let feeCurrency: PortfolioCurrency?
    let notes: String
}

struct CoinMarketCapCSVPreview: Identifiable {
    let id = UUID()
    var rows: [CoinMarketCapCSVRow]
    var errors: [String]
    var warnings: [String]
    var symbols: [String] { Array(Set(rows.map(\.token))).sorted() }
    var isValid: Bool { errors.isEmpty }

    func transactions(
        portfolioID: UUID, mappings: [String: PortfolioAsset],
        feeFXRates: [String: Decimal] = [:], skippedRowIDs: Set<String> = []
    ) -> PortfolioCSVPreview {
        var conversionErrors = errors
        var conversionWarnings = warnings
        let skippedRows = rows.filter { mappings[$0.token] == nil || skippedRowIDs.contains($0.id) }
        if !skippedRows.isEmpty {
            let symbols = Array(Set(skippedRows.map(\.token))).sorted().joined(separator: ", ")
            conversionWarnings.append(
                "Skipped \(skippedRows.count) transactions for unmapped tickers: \(symbols). Only mapped tickers will be imported."
            )
        }
        let values = rows.compactMap { row -> PortfolioTransaction? in
            guard !skippedRowIDs.contains(row.id) else { return nil }
            guard let asset = mappings[row.token] else { return nil }
            let feeCurrency = row.feeCurrency ?? .USD
            var accountingFee = row.fee
            var notes = row.notes
            if row.fee != 0 && feeCurrency != .USD {
                guard let rate = feeFXRates[row.id], rate > 0 else {
                    conversionErrors.append(
                        "Line \(row.line): supply the historical \(feeCurrency.rawValue) → USD rate or skip this transaction."
                    )
                    return nil
                }
                accountingFee = row.fee * rate
                let disclosure = "CMC fee: \(row.fee) \(feeCurrency.rawValue) × \(rate) = \(accountingFee) USD"
                notes = notes.isEmpty ? disclosure : "\(notes) · \(disclosure)"
            }
            return PortfolioTransaction(
                portfolioID: portfolioID, asset: asset, type: row.type, quantity: row.amount,
                price: row.price, priceCurrency: .USD, fee: accountingFee,
                feeCurrency: .USD, timestamp: row.timestamp, notes: notes,
                source: .coinMarketCap, externalTransactionID: row.id
            )
        }
        return PortfolioCSVPreview(transactions: values, errors: conversionErrors, warnings: conversionWarnings)
    }
}

enum PortfolioCSVService {
    static let header =
        "portfolio,asset_id,symbol,name,source,type,quantity,price,currency,fee,fee_currency,date,notes,external_id"

    static func exportTransactions(_ transactions: [PortfolioTransaction], portfolios: [Portfolio]) -> String {
        let names = Dictionary(portfolios.map { ($0.id, $0.name) }, uniquingKeysWith: { existing, _ in existing })
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return
            ([header]
            + transactions.sorted { $0.timestamp < $1.timestamp }.map { tx in
                [
                    names[tx.portfolioID] ?? "", tx.asset.key, tx.asset.symbol, tx.asset.name,
                    tx.asset.source.rawValue, tx.type.rawValue, tx.quantity.description, tx.price?.description ?? "",
                    tx.priceCurrency.rawValue, tx.fee.description, tx.feeCurrency.rawValue,
                    formatter.string(from: tx.timestamp), tx.notes, tx.externalTransactionID ?? "",
                ].map(escape).joined(separator: ",")
            }).joined(separator: "\n")
    }

    static func exportHoldings(_ holdings: [PortfolioHolding], portfolioName: String, timestamp: Date = Date())
        -> String
    {
        let iso = ISO8601DateFormatter().string(from: timestamp)
        let rows = holdings.map { holding in
            [
                portfolioName, holding.asset.key, holding.asset.symbol, holding.quantity.description,
                holding.averageCost.description, holding.currentPrice?.description ?? "",
                holding.currentValue?.description ?? "", holding.realizedPnL.description,
                holding.unrealizedPnL?.description ?? "", iso,
            ].map(escape).joined(separator: ",")
        }
        return
            ([
                "portfolio,asset_id,symbol,quantity,average_cost,current_price,current_value,realized_pnl,unrealized_pnl,as_of"
            ] + rows).joined(separator: "\n")
    }

    static func exportHistory(_ snapshots: [PortfolioSnapshot]) -> String {
        let iso = ISO8601DateFormatter()
        return
            (["portfolio_id,timestamp,value,net_contributions,realized_pnl,unrealized_pnl,is_complete"]
            + snapshots.sorted { $0.timestamp < $1.timestamp }.map {
                [
                    $0.portfolioID.uuidString, iso.string(from: $0.timestamp), $0.value.description,
                    $0.netContributions.description, $0.realizedPnL.description, $0.unrealizedPnL.description,
                    $0.isComplete ? "true" : "false",
                ].joined(separator: ",")
            }).joined(separator: "\n")
    }

    static func preview(_ csv: String, portfolios: [Portfolio]) -> PortfolioCSVPreview {
        let rows = csv.split(whereSeparator: \.isNewline).map { parseRow(String($0)) }
        guard let fields = rows.first else {
            return .init(transactions: [], errors: ["The CSV is empty."], warnings: [])
        }
        let indexes = Dictionary(
            fields.enumerated().map { ($1.lowercased(), $0) }, uniquingKeysWith: { existing, _ in existing })
        let required = ["portfolio", "asset_id", "symbol", "source", "type", "quantity", "currency", "date"]
        let missing = required.filter { indexes[$0] == nil }
        guard missing.isEmpty else {
            return .init(
                transactions: [], errors: ["Missing columns: \(missing.joined(separator: ", "))."], warnings: [])
        }
        var values: [PortfolioTransaction] = []
        var errors: [String] = []
        var warnings: [String] = []
        let iso = ISO8601DateFormatter()
        func value(_ row: [String], _ name: String) -> String {
            indexes[name].flatMap { row.indices.contains($0) ? row[$0] : nil } ?? ""
        }
        for (offset, row) in rows.dropFirst().enumerated() {
            let line = offset + 2
            guard
                let portfolio = portfolios.first(where: {
                    $0.name.caseInsensitiveCompare(value(row, "portfolio")) == .orderedSame
                })
            else {
                errors.append("Line \(line): unknown portfolio.")
                continue
            }
            guard let source = DataSourceType(rawValue: value(row, "source")),
                let type = PortfolioTransactionType(rawValue: value(row, "type")),
                let quantity = Decimal(string: value(row, "quantity")), quantity > 0,
                let currency = PortfolioCurrency(rawValue: value(row, "currency")),
                let date = iso.date(from: value(row, "date"))
            else {
                errors.append("Line \(line): invalid source, type, quantity, currency, or ISO-8601 date.")
                continue
            }
            let feeCurrency = PortfolioCurrency(rawValue: value(row, "fee_currency")) ?? currency
            let asset = PortfolioAsset(
                key: value(row, "asset_id"), symbol: value(row, "symbol"),
                name: value(row, "name").nilIfEmpty ?? value(row, "symbol"), source: source, quoteCurrency: currency)
            let tx = PortfolioTransaction(
                portfolioID: portfolio.id, asset: asset, type: type, quantity: quantity,
                price: Decimal(string: value(row, "price")), priceCurrency: currency,
                fee: Decimal(string: value(row, "fee")) ?? 0, feeCurrency: feeCurrency, timestamp: date,
                notes: value(row, "notes"), source: .csv, externalTransactionID: value(row, "external_id").nilIfEmpty)
            values.append(tx)
            if tx.externalTransactionID == nil {
                warnings.append("Line \(line): no external_id; repeat imports cannot be deduplicated reliably.")
            }
        }
        return .init(transactions: values, errors: errors, warnings: warnings)
    }

    static func previewCoinMarketCap(_ csv: String) -> CoinMarketCapCSVPreview {
        let records = csv.split(whereSeparator: \.isNewline).map { parseRow(String($0)) }
        guard let header = records.first else {
            return .init(rows: [], errors: ["The CoinMarketCap CSV is empty."], warnings: [])
        }
        let normalized = header.map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
        func column(_ prefix: String) -> Int? { normalized.firstIndex { $0.hasPrefix(prefix) } }
        guard let dateIndex = column("date"), let tokenIndex = column("token"),
            let typeIndex = column("type"), let priceIndex = column("price"),
            let amountIndex = column("amount"), let totalIndex = column("total value"),
            let feeIndex = column("fee"), let feeCurrencyIndex = column("fee currency"),
            let notesIndex = column("notes")
        else {
            return .init(rows: [], errors: ["This is not a recognized CoinMarketCap transaction export."], warnings: [])
        }

        let timezone = timeZone(from: header[dateIndex])
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.calendar = Calendar(identifier: .gregorian)
        formatter.timeZone = timezone
        formatter.dateFormat = "yyyy-MM-dd HH:mm:ss"

        var rows: [CoinMarketCapCSVRow] = []
        var errors: [String] = []
        var warnings: [String] = []
        func field(_ record: [String], _ index: Int) -> String {
            record.indices.contains(index) ? record[index].trimmingCharacters(in: .whitespacesAndNewlines) : ""
        }
        func decimal(_ value: String) -> Decimal? {
            let cleaned = value.replacingOccurrences(of: ",", with: "")
            guard cleaned != "--", !cleaned.isEmpty else { return nil }
            return Decimal(string: cleaned, locale: Locale(identifier: "en_US_POSIX"))
        }

        for (offset, record) in records.dropFirst().enumerated() {
            let line = offset + 2
            let token = field(record, tokenIndex).uppercased()
            let rawType = field(record, typeIndex).lowercased()
            let type: PortfolioTransactionType? = {
                switch rawType {
                case "buy": return .buy
                case "sell": return .sell
                case "transfer in", "transfer_in": return .transferIn
                case "transfer out", "transfer_out": return .transferOut
                default: return PortfolioTransactionType.allCases.first { $0.rawValue.lowercased() == rawType }
                }
            }()
            guard !token.isEmpty, let type,
                let date = formatter.date(from: field(record, dateIndex)),
                let price = decimal(field(record, priceIndex)),
                let amount = decimal(field(record, amountIndex)), amount > 0,
                let total = decimal(field(record, totalIndex))
            else {
                errors.append("Line \(line): invalid date, token, type, price, amount, or total value.")
                continue
            }
            let fee = decimal(field(record, feeIndex)) ?? 0
            let feeCurrency = PortfolioCurrency(rawValue: field(record, feeCurrencyIndex).uppercased())
            if fee != 0 && feeCurrency == nil {
                errors.append("Line \(line): fee currency is missing or unsupported.")
                continue
            }
            if abs((price * amount) - total) > max(Decimal(string: "0.02")!, total * Decimal(string: "0.001")!) {
                warnings.append("Line \(line): total value differs from price × amount.")
            }
            let stableID = "cmc:\(Int(date.timeIntervalSince1970)):\(token):\(rawType):\(amount):\(price):\(total)"
            rows.append(
                .init(
                    id: stableID, line: line, timestamp: date, token: token, type: type,
                    price: price, amount: amount, totalValue: total, fee: fee,
                    feeCurrency: feeCurrency, notes: field(record, notesIndex)))
        }
        if timezone == nil { warnings.append("The date header had no usable UTC offset; UTC was used.") }
        return .init(rows: rows, errors: errors, warnings: warnings)
    }

    private static func timeZone(from header: String) -> TimeZone? {
        guard let start = header.firstIndex(of: "+") ?? header.firstIndex(of: "-") else {
            return TimeZone(secondsFromGMT: 0)
        }
        let suffix = header[start...].prefix { $0.isNumber || $0 == "+" || $0 == "-" || $0 == ":" }
        let parts = suffix.dropFirst().split(separator: ":")
        guard let hours = Int(parts.first ?? ""), let sign = suffix.first else { return TimeZone(secondsFromGMT: 0) }
        let minutes = parts.count > 1 ? Int(parts[1]) ?? 0 : 0
        let seconds = (hours * 3600 + minutes * 60) * (sign == "-" ? -1 : 1)
        return TimeZone(secondsFromGMT: seconds)
    }

    private static func escape(_ value: String) -> String {
        guard value.contains(",") || value.contains("\"") || value.contains("\n") else { return value }
        return "\"\(value.replacingOccurrences(of: "\"", with: "\"\""))\""
    }
    private static func parseRow(_ row: String) -> [String] {
        var values: [String] = []
        var field = ""
        var quoted = false
        var index = row.startIndex
        while index < row.endIndex {
            let char = row[index]
            if char == "\"" {
                let next = row.index(after: index)
                if quoted && next < row.endIndex && row[next] == "\"" {
                    field.append("\"")
                    index = next
                } else {
                    quoted.toggle()
                }
            } else if char == "," && !quoted {
                values.append(field)
                field = ""
            } else {
                field.append(char)
            }
            index = row.index(after: index)
        }
        values.append(field)
        return values
    }
}

private extension String { var nilIfEmpty: String? { isEmpty ? nil : self } }
