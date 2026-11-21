# Source logos

The marks in `assets/source_logos/` identify the provider a market's data comes from, in the symbol
picker, on chart cards and in watchlists. Each belongs to its owner and stands for that owner only:
Binance, Coinbase, OKX, Kraken, Bybit, Gate.io, MEXC, Hyperliquid, Aster, Lighter, Deribit, HTX, CoinGecko, CoinMarketCap, DEXScreener, Alpaca, Polymarket and Kalshi. They are
not licensed under the application's GPL-3.0-only terms.

A new provider adds its mark here (one file named after the provider, `.svg` where the owner publishes one),
an arm in `ui::kit::source_logo::source`, and a line in this list. The OKX, Kraken, Bybit, Gate.io,
MEXC, Hyperliquid, Aster, Lighter, Deribit and HTX files redraw the owners' marks as vectors; Hyperliquid's is traced from the
favicon its site serves (`apple-touch-icon.png`, the mint shape on `#0f1a1e`), Aster's from its
`apple-touch-icon.png` (the four cream pieces on black, traced from the pixels with a gradient
between the two cream tones it uses). Lighter's is its `apple-touch-icon.png` (two white parallelograms
on black; the corners measured from the pixels). Deribit's is its `apple-touch-icon.png` (the blue
`#0052ff` D between two pairs of bars, on white: the body, the arcs' centre and radii and the bars
measured from the pixels, the two arcs sharing one centre). HTX's is the `icon-pwa-180.png` its site
serves (the navy flame with the blue one inside it, on white): the two outlines are traced from the
pixels, each edge placed where the anti-aliasing puts it and the curves cut into straight segments
less than a third of a pixel off.
