import SwiftUI

/// The coin's artwork with its data source's logo badged on the corner; a monogram until the artwork resolves.
struct AlertAssetIcon: View {
    let asset: PortfolioAsset
    @ObservedObject var info: PortfolioAssetInfoViewModel
    var size: CGFloat = 34

    var body: some View {
        TickerIconView(
            symbol: asset.displayTicker, url: info.iconURL(for: asset), size: size, source: asset.source
        )
        .padding(.trailing, 3)
        .padding(.bottom, 3)
        .accessibilityHidden(true)
    }
}
