# Source logos

The marks in `assets/source_logos/` identify the provider a market's data comes from, in the symbol
picker, on chart cards and in watchlists. Each belongs to its owner and stands for that owner only:
Binance, Coinbase, OKX, Kraken, Bybit, Gate.io, MEXC, Hyperliquid, Aster, CoinGecko, CoinMarketCap, DEXScreener, Alpaca, Polymarket and Kalshi. They are
not licensed under the application's GPL-3.0-only terms.

A new provider adds its mark here (one file named after the provider, `.svg` where the owner publishes one),
an arm in `ui::kit::source_logo::source`, and a line in this list. The OKX, Kraken, Bybit, Gate.io,
MEXC, Hyperliquid and Aster files redraw the owners' marks as vectors; Hyperliquid's is traced from the
favicon its site serves (`apple-touch-icon.png`, the mint shape on `#0f1a1e`), Aster's from its
`apple-touch-icon.png` (the four cream pieces on black, traced from the pixels with a gradient
between the two cream tones it uses).
