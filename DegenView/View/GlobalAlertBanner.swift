import SwiftUI

struct GlobalAlertBanner: View {
    @StateObject private var store = AlertStore.shared
    @StateObject private var pineStore = PineAlertStore.shared
    var body: some View {
        VStack(spacing: 6) {
            priceBanner
            pineBanner
        }
    }

    @ViewBuilder private var priceBanner: some View {
        if let event = store.bannerEvent {
            HStack(spacing: 10) {
                Image(systemName: "bell.fill").foregroundStyle(.orange)
                Text("\(event.asset.symbol) reached \(event.currency.formatAlertPrice(event.target))")
                Button {
                    store.bannerEvent = nil
                } label: {
                    Image(systemName: "xmark")
                }.buttonStyle(.plain)
            }
            .padding(.horizontal, 14).padding(.vertical, 9)
            .background(.regularMaterial, in: Capsule()).shadow(radius: 5).padding(.top, 8)
            .task(id: event.id) {
                try? await Task.sleep(for: .seconds(6))
                if store.bannerEvent?.id == event.id { store.bannerEvent = nil }
            }
        }
    }

    @ViewBuilder private var pineBanner: some View {
        if let alert = pineStore.banner {
            HStack(spacing: 10) {
                Image(systemName: "bell.badge.fill").foregroundStyle(.orange)
                VStack(alignment: .leading, spacing: 1) {
                    Text(alert.title).font(.caption).foregroundStyle(.secondary)
                    Text(alert.message.isEmpty ? "Alert" : alert.message)
                }
                Button {
                    pineStore.banner = nil
                } label: {
                    Image(systemName: "xmark")
                }.buttonStyle(.plain)
            }
            .padding(.horizontal, 14).padding(.vertical, 9)
            .background(.regularMaterial, in: Capsule()).shadow(radius: 5).padding(.top, 8)
            .task(id: alert.id) {
                try? await Task.sleep(for: .seconds(6))
                if pineStore.banner?.id == alert.id { pineStore.banner = nil }
            }
        }
    }
}
