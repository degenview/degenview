import Foundation

// MARK: - API models

/// `/coins/markets` row, trimmed to the icon fields.
private struct MarketCoin: Decodable {
    let id: String
    let symbol: String
    let image: String
}

// MARK: - Cache

private struct IconCache: Codable {
    /// Resolved icon per `"<source>:<ticker>"`. A `nil` url is a remembered miss.
    var entries: [String: Entry] = [:]
    /// Base symbol → image URL, from the market snapshot.
    var symbolMap: [String: String] = [:]
    /// CoinGecko coin id → image URL, from the same snapshot. Kept apart from
    /// `symbolMap` because ids and symbols share a namespace ("bitcoin" is both).
    var idMap: [String: String] = [:]
    var marketUpdatedAt: Date = .distantPast
    /// Optional so caches written before stock-specific resolution still decode.
    var stockResolverVersion: Int?

    struct Entry: Codable {
        let url: String?
        let updatedAt: Date
    }
}

// MARK: - Resolver

/// Resolves a coin icon by walking a chain of sources, widest coverage last:
///
/// 1. the cached result, positive or negative;
/// 2. a snapshot of the top coins by market cap, indexed by symbol *and* coin id;
/// 3. a source-specific lookup (CoinGecko by coin id, DEXScreener by pair address);
/// 4. CoinGecko `/search`, which reaches every coin it lists;
/// 5. a static icon set, for majors the APIs couldn't place.
///
/// Returns `nil` once every source misses; `TickerIconView` draws a monogram then.
///
/// An actor because every visible card resolves concurrently: the cache, the
/// in-flight table and the pending-batch set are all shared mutable state.
actor IconResolver {
    static let shared = IconResolver()

    private let baseURL = CoinGeckoAPIService.apiBase
    private let session = AppSupport.defaultSession
    private let store = JSONStore<IconCache>(filename: "icon_cache.json")

    private var cache: IconCache

    /// One task per key, so N cards asking for the same coin share one walk.
    private var inFlight: [String: Task<URL?, Never>] = [:]

    /// The market snapshot refresh, shared by everyone waiting on it.
    private var marketRefresh: Task<Void, Never>?
    private var lastMarketAttempt: Date = .distantPast

    /// Coin ids waiting to go out in the next batched `/coins/markets` call.
    private var pendingCoinIDs: Set<String> = []
    private var coinIDBatch: Task<[String: String], Never>?

    init() {
        cache = store.load() ?? IconCache()
        if cache.stockResolverVersion != Icon.stockResolverVersion {
            cache.entries = cache.entries.filter { key, _ in
                !key.hasPrefix("\(DataSourceType.alpaca.rawValue):")
            }
            cache.stockResolverVersion = Icon.stockResolverVersion
            store.save(cache)
        }
    }

    // MARK: - Public

    /// - Parameters:
    ///   - ticker: the chart's ticker — a Binance pair, a CoinGecko coin id, or a
    ///     DEXScreener pair address, depending on `source`.
    ///   - baseSymbol: the ticker reduced to its base asset, used by the
    ///     symbol-keyed steps of the chain.
    func iconURL(ticker: String, source: DataSourceType, baseSymbol: String) async -> URL? {
        let key = "\(source.rawValue):\(ticker.lowercased())"

        if let entry = cache.entries[key], !isExpired(entry) {
            return entry.url.flatMap { URL(string: $0) }
        }

        if let existing = inFlight[key] {
            return await existing.value
        }

        let task = Task<URL?, Never> {
            await self.resolve(ticker: ticker, source: source, baseSymbol: baseSymbol)
        }
        inFlight[key] = task

        let url = await task.value
        inFlight[key] = nil
        remember(key: key, url: url)
        return url
    }

    /// Seed the cache with artwork the caller already has.
    ///
    /// A Polymarket search payload carries the market image, so adding a chart from
    /// search needs no lookup at all — without this, the card would re-fetch the same
    /// URL it just displayed in the picker.
    func remember(ticker: String, source: DataSourceType, url: URL?) {
        remember(key: "\(source.rawValue):\(ticker.lowercased())", url: url)
    }

    // MARK: - Chain

    private func resolve(ticker: String, source: DataSourceType, baseSymbol: String) async -> URL? {
        var symbol = baseSymbol

        // Prediction markets carry their own artwork and have no crypto symbol behind
        // them — none of the coin-oriented steps below can contribute, so skip them
        // rather than spend a market-snapshot refresh on the way past.
        if source == .polymarket {
            return await PolymarketService.marketInfo(tokenID: ticker)?.imageURL
        }

        // Equity tickers share symbols with tokenized stocks and unrelated crypto
        // projects. Keep them out of every coin-oriented step below: an absent stock
        // logo must become a monogram, never a plausible-but-wrong CoinGecko match.
        if source == .alpaca {
            return await stockIcon(symbol: ticker)
        }

        // 1. Market snapshot — covers Binance base symbols and top-ranked coin ids.
        await refreshMarketMapIfNeeded()
        if let url = snapshotURL(ticker: ticker, source: source, symbol: symbol) {
            return url
        }

        // 2. Source-specific lookup.
        switch source {
        case .coingecko:
            if let url = await coinGeckoIcon(id: ticker) { return url }

        case .dexscreener:
            let pair = await DEXScreenerService.pairIcon(forPair: ticker)
            if let url = pair?.imageURL { return url }

            // The pair handed back a real ticker, so the remaining symbol-keyed
            // steps have something usable instead of a contract address.
            if let resolved = pair?.baseSymbol, !resolved.isEmpty {
                symbol = resolved
                if let url = cache.symbolMap[resolved.lowercased()].flatMap({ URL(string: $0) }) {
                    return url
                }
            }

        case .binance, .alpaca, .polymarket:
            break
        }

        guard isPlausibleSymbol(symbol) else { return nil }

        // 3. CoinGecko search — every listed coin, not just the snapshot.
        if let url = await CoinGeckoAPIService.iconURL(searchingFor: symbol) { return url }

        // 4. Static icon set.
        return await staticIcon(symbol: symbol)
    }

    /// Symbol-shaped enough to be worth spending a search on. DEX tickers that never
    /// resolved to a token symbol are still contract addresses at this point.
    private func isPlausibleSymbol(_ symbol: String) -> Bool {
        !symbol.isEmpty
            && symbol.count <= Icon.maxSymbolLength
            && symbol.allSatisfy { $0.isLetter || $0.isNumber }
    }

    private func snapshotURL(ticker: String, source: DataSourceType, symbol: String) -> URL? {
        let image: String?
        switch source {
        case .coingecko:
            // The ticker is the coin id; fall back to the symbol for ids that
            // happen to match one (e.g. a coin listed under "btc").
            image = cache.idMap[ticker.lowercased()] ?? cache.symbolMap[symbol.lowercased()]
        case .binance, .dexscreener, .alpaca:
            image = cache.symbolMap[symbol.lowercased()]
        case .polymarket:
            // Market questions never key into a coin symbol map.
            image = nil
        }
        return image.flatMap { URL(string: $0) }
    }

    // MARK: - Market snapshot

    /// Top coins by market cap, indexed by symbol and by id. Highest-cap coin wins
    /// per symbol (so "btc" → bitcoin, not batcat).
    private func refreshMarketMapIfNeeded() async {
        if let existing = marketRefresh {
            await existing.value
            return
        }

        let expiry = cache.marketUpdatedAt.addingTimeInterval(Icon.cacheStaleness)
        guard Date() > expiry else { return }
        // Stamped whether or not the fetch succeeds — otherwise a network error
        // means a full snapshot refetch for every card that appears afterwards.
        guard Date().timeIntervalSince(lastMarketAttempt) > Icon.refreshRetryInterval else { return }
        lastMarketAttempt = Date()

        let task = Task { await self.refreshMarketMap() }
        marketRefresh = task
        await task.value
        marketRefresh = nil
    }

    private func refreshMarketMap() async {
        guard var components = URLComponents(string: "\(baseURL)/coins/markets") else { return }
        components.queryItems = [
            URLQueryItem(name: "vs_currency", value: "usd"),
            URLQueryItem(name: "order", value: "market_cap_desc"),
            URLQueryItem(name: "per_page", value: String(Icon.maxCoins)),
            URLQueryItem(name: "sparkline", value: "false"),
        ]
        guard let url = components.url else { return }

        guard let coins = await fetchMarketCoins(url: url, describedAs: "snapshot") else { return }

        var symbols: [String: String] = [:]
        var ids: [String: String] = [:]
        for coin in coins {
            let symbol = coin.symbol.lowercased()
            if symbols[symbol] == nil { symbols[symbol] = coin.image }
            ids[coin.id.lowercased()] = coin.image
        }

        cache.symbolMap = symbols
        cache.idMap = ids
        cache.marketUpdatedAt = Date()
        save()
    }

    // MARK: - CoinGecko by coin id

    /// Icons for coin ids outside the snapshot. Cards that miss within the same
    /// window ride along on one `ids=` request instead of one call each.
    private func coinGeckoIcon(id: String) async -> URL? {
        let coinID = id.lowercased()
        pendingCoinIDs.insert(coinID)

        let batch: Task<[String: String], Never>
        if let existing = coinIDBatch {
            batch = existing
        } else {
            batch = Task {
                try? await Task.sleep(nanoseconds: Icon.batchWindowNS)
                return await self.runCoinIDBatch()
            }
            coinIDBatch = batch
        }

        let images = await batch.value
        return images[coinID].flatMap { URL(string: $0) }
    }

    private func runCoinIDBatch() async -> [String: String] {
        let ids = Array(pendingCoinIDs.prefix(Icon.maxCoins))
        pendingCoinIDs.subtract(ids)
        // Cleared before the request so ids arriving during it open a fresh batch.
        coinIDBatch = nil

        guard !ids.isEmpty,
              var components = URLComponents(string: "\(baseURL)/coins/markets")
        else { return [:] }

        components.queryItems = [
            URLQueryItem(name: "vs_currency", value: "usd"),
            URLQueryItem(name: "ids", value: ids.joined(separator: ",")),
            URLQueryItem(name: "per_page", value: String(min(ids.count, CoinGecko.pageLimit))),
            URLQueryItem(name: "sparkline", value: "false"),
        ]
        guard let url = components.url,
              let coins = await fetchMarketCoins(url: url, describedAs: "\(ids.count) coin id(s)")
        else { return [:] }

        var images: [String: String] = [:]
        for coin in coins {
            images[coin.id.lowercased()] = coin.image
            // Worth keeping: these coins sit outside the snapshot entirely.
            cache.idMap[coin.id.lowercased()] = coin.image
        }
        save()
        return images
    }

    /// Shared `/coins/markets` request path — rate limited alongside OHLC fetches,
    /// which draw on the same public-tier budget.
    private func fetchMarketCoins(url: URL, describedAs label: String) async -> [MarketCoin]? {
        await CGRateLimiter.shared.waitForSlot()
        guard !Task.isCancelled else { return nil }

        do {
#if DEBUG
            print("[Icon] Fetching market \(label)")
#endif
            let (data, response) = try await session.data(from: url)

            guard let httpResponse = response as? HTTPURLResponse else { return nil }
            if httpResponse.statusCode == 429 {
                let waitSeconds = httpResponse.value(forHTTPHeaderField: "Retry-After").flatMap(Int.init) ?? 30
                await CGRateLimiter.shared.backoff(seconds: waitSeconds)
                return nil
            }
            guard httpResponse.statusCode == 200 else { return nil }

            await CGRateLimiter.shared.noteSuccess()
            return try JSONDecoder().decode([MarketCoin].self, from: data)
        } catch {
#if DEBUG
            print("[Icon] Market fetch failed (\(label)): \(error.localizedDescription)")
#endif
            return nil
        }
    }

    // MARK: - Static icon set

    /// Company artwork for Alpaca equities. Verify the endpoint before caching it;
    /// an unknown ticker returns a text 404 rather than an image.
    private func stockIcon(symbol: String) async -> URL? {
        let ticker = symbol.trimmingCharacters(in: .whitespacesAndNewlines).uppercased()
        guard isPlausibleSymbol(ticker),
              let encoded = ticker.addingPercentEncoding(withAllowedCharacters: .urlPathAllowed),
              let url = URL(string: "\(Icon.stockCDNBase)/\(encoded).png")
        else { return nil }

        var request = URLRequest(url: url)
        request.httpMethod = "HEAD"

        guard let (_, response) = try? await session.data(for: request),
              let httpResponse = response as? HTTPURLResponse,
              httpResponse.statusCode == 200,
              httpResponse.mimeType?.hasPrefix("image/") == true
        else { return nil }

        return url
    }

    /// A plain file in a public repo — no API, no rate limit, 404 on miss. The 404
    /// has to be checked here: caching the URL unverified would leave the card with
    /// a permanently broken image reference.
    private func staticIcon(symbol: String) async -> URL? {
        guard let url = URL(string: "\(Icon.staticCDNBase)/\(symbol.lowercased()).png") else {
            return nil
        }

        var request = URLRequest(url: url)
        request.httpMethod = "HEAD"

        guard let (_, response) = try? await session.data(for: request),
              let httpResponse = response as? HTTPURLResponse,
              httpResponse.statusCode == 200
        else { return nil }

        return url
    }

    // MARK: - Cache

    private func isExpired(_ entry: IconCache.Entry) -> Bool {
        let ttl = entry.url == nil ? Icon.negativeTTL : Icon.cacheStaleness
        return Date() > entry.updatedAt.addingTimeInterval(ttl)
    }

    private func remember(key: String, url: URL?) {
        cache.entries[key] = IconCache.Entry(url: url?.absoluteString, updatedAt: Date())
        save()
    }

    private func save() {
        store.save(cache)
    }
}
