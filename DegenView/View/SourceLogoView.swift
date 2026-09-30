import AppKit
import SwiftUI

/// A data source's logo, drawn from the asset catalog in a fixed square slot.
///
/// Falls back to the source's SF Symbol if the imageset is missing, so a new
/// `DataSourceType` case never renders as an empty hole.
struct SourceLogoView: View {
    let source: DataSourceType
    var size: CGFloat = Icon.size

    var body: some View {
        Group {
            if NSImage(named: source.logoAsset) != nil {
                Image(source.logoAsset)
                    .resizable()
                    .scaledToFit()
                    .clipShape(RoundedRectangle(cornerRadius: size * 0.22, style: .continuous))
            } else {
                Image(systemName: source.icon)
                    .font(.system(size: size * 0.7))
                    .foregroundStyle(.secondary)
            }
        }
        .frame(width: size, height: size)
        .accessibilityLabel(source.displayName)
    }
}

#Preview {
    HStack(spacing: 8) {
        ForEach(DataSourceType.allCases, id: \.self) { source in
            SourceLogoView(source: source, size: 28)
        }
    }
    .padding()
}
