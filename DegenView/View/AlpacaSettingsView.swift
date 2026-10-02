import SwiftUI

/// Settings ▸ Alpaca: the API keys that unlock US stock data.
struct AlpacaSettingsView: View {
    @State private var keyID = AlpacaCredentialsStore.credentials.keyID
    @State private var secretKey = AlpacaCredentialsStore.credentials.secretKey
    @State private var isSaved = AlpacaCredentialsStore.isConfigured
    @State private var status: String?
    @State private var statusIsError = false

    private var canSave: Bool {
        !keyID.trimmingCharacters(in: .whitespaces).isEmpty && !secretKey.trimmingCharacters(in: .whitespaces).isEmpty
    }

    var body: some View {
        SettingsPage(
            systemImage: "chart.xyaxis.line", title: "Alpaca",
            subtitle: "Connect Alpaca to chart US stocks and ETFs in DegenView."
        ) {
            HStack {
                SettingsStatusBadge(
                    text: isSaved ? "Keys saved" : "Not configured", tone: isSaved ? .good : .neutral)
                Spacer()
            }

            SettingsSection(title: "API credentials") {
                SettingsCard {
                    SettingsField(title: "API Key ID", prompt: "PK…", text: $keyID)
                    SettingsField(
                        title: "Secret Key", hint: "Stored in your Mac's Keychain, never in a file.",
                        text: $secretKey, isSecret: true)
                }
            }

            SettingsSection(title: "Getting your keys") {
                SettingsCard {
                    Step(number: 1, text: "Create a free Alpaca account.")
                    Step(number: 2, text: "Open the API Keys section of the dashboard and generate a key.")
                    Step(number: 3, text: "Copy both values here — Alpaca only shows the secret once.")
                    Link(destination: URL(string: "https://app.alpaca.markets/")!) {
                        Label("Open the Alpaca dashboard", systemImage: "arrow.up.right.square")
                    }
                }
            }
        } footer: {
            HStack(spacing: 10) {
                if let status { SettingsStatusMessage(text: status, isError: statusIsError) }
                Spacer()
                if isSaved {
                    Button("Remove Keys", role: .destructive, action: remove)
                        .controlSize(.large)
                }
                Button("Save", action: save)
                    .buttonStyle(.borderedProminent)
                    .controlSize(.large)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canSave)
            }
        }
    }

    private func save() {
        do {
            try AlpacaCredentialsStore.save(.init(keyID: keyID, secretKey: secretKey))
            isSaved = true
            report("Saved securely in Keychain.")
        } catch {
            report("Could not save: \(error.localizedDescription)", isError: true)
        }
    }

    private func remove() {
        do {
            try AlpacaCredentialsStore.save(.init(keyID: "", secretKey: ""))
            keyID = ""
            secretKey = ""
            isSaved = false
            report("Keys removed. Stock charts need keys to load.")
        } catch {
            report("Could not remove the keys: \(error.localizedDescription)", isError: true)
        }
    }

    private func report(_ message: String, isError: Bool = false) {
        status = message
        statusIsError = isError
    }
}

/// A numbered instruction line.
private struct Step: View {
    let number: Int
    let text: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text("\(number)")
                .font(.caption.weight(.bold))
                .foregroundStyle(Color.accentColor)
                .frame(width: 20, height: 20)
                .background(Color.accentColor.opacity(0.14), in: Circle())
            Text(text).font(.callout).foregroundStyle(.secondary)
        }
    }
}
