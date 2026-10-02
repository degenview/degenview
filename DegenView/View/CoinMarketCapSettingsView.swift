import SwiftUI

/// Settings ▸ CoinMarketCap: an optional API key for higher rate limits on the market indices.
struct CoinMarketCapSettingsView: View {
    @State private var apiKey = ""
    @State private var configured = CoinMarketCapCredentialStore.isConfigured
    @State private var status: String?
    @State private var statusIsError = false
    @State private var isTesting = false

    var body: some View {
        SettingsPage(
            systemImage: "gauge.with.dots.needle.50percent", title: "CoinMarketCap",
            subtitle: "Market sentiment charts, with optional higher API rate limits."
        ) {
            HStack {
                SettingsStatusBadge(
                    text: configured ? "API key configured" : "Public API mode",
                    tone: configured ? .good : .neutral)
                Spacer()
            }

            SettingsSection(title: "API access") {
                SettingsCard {
                    SettingsField(
                        title: configured ? "Replace API Key" : "API Key",
                        hint: configured
                            ? "A key is saved in your Keychain. Enter a new one to replace it."
                            : "Stored in your Mac's Keychain, never in a file.",
                        text: $apiKey, isSecret: true)
                }
            }

            NoticeCard(
                systemImage: "info.circle.fill", tint: .blue, title: "A key is optional",
                detail: "CoinMarketCap works without an API key. Adding one gives DegenView higher API rate limits.")

            Link(destination: URL(string: "https://pro.coinmarketcap.com/signup/")!) {
                Label("Get an API key", systemImage: "arrow.up.right.square")
            }
        } footer: {
            HStack(spacing: 10) {
                if let status { SettingsStatusMessage(text: status, isError: statusIsError) }
                Spacer()
                if configured {
                    Button("Remove Key", role: .destructive, action: remove)
                        .controlSize(.large)
                }
                Button(action: testConnection) {
                    if isTesting {
                        ProgressView().controlSize(.small)
                    } else {
                        Text("Test Connection")
                    }
                }
                .controlSize(.large)
                .disabled(isTesting)
                Button("Save", action: save)
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .keyboardShortcut(.defaultAction)
                    .disabled(apiKey.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
        }
    }

    private func save() {
        do {
            try CoinMarketCapCredentialStore.save(apiKey)
            configured = true
            apiKey = ""
            report("Saved securely in Keychain.")
        } catch {
            report("Could not save: \(error.localizedDescription)", isError: true)
        }
    }

    private func remove() {
        do {
            try CoinMarketCapCredentialStore.remove()
            apiKey = ""
            configured = false
            report("Key removed. Public API mode is active.")
        } catch {
            report("Could not remove the key: \(error.localizedDescription)", isError: true)
        }
    }

    private func testConnection() {
        isTesting = true
        status = nil
        Task {
            do {
                _ = try await CoinMarketCapDataProvider.shared.fearGreedLatest(force: true)
                report("Connection successful.")
            } catch {
                report(error.localizedDescription, isError: true)
            }
            isTesting = false
        }
    }

    private func report(_ message: String, isError: Bool = false) {
        status = message
        statusIsError = isError
    }
}
