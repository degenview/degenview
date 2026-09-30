import SwiftUI

/// Asks for a script's file name before anything is created. Create stays disabled, with
/// the reason shown, until the name is a valid file name that no other script uses.
struct ScriptNameSheet: View {
    /// The reason a name cannot be used, or nil when it can.
    let problemFor: (String) -> String?
    let onCreate: (String) -> Void
    let onCancel: () -> Void

    @State private var name = ""
    @FocusState private var isFocused: Bool

    private var problem: String? { problemFor(name) }

    private var canCreate: Bool { !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && problem == nil }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("New Script").font(.headline)
            TextField("Script name", text: $name)
                .textFieldStyle(.roundedBorder)
                .focused($isFocused)
                .onSubmit(submit)
            Text(problem ?? " ")
                .font(.caption)
                .foregroundStyle(.red)
                .frame(maxWidth: .infinity, alignment: .leading)
            HStack {
                Spacer()
                Button("Cancel", role: .cancel, action: onCancel)
                    .keyboardShortcut(.cancelAction)
                Button("Create", action: submit)
                    .keyboardShortcut(.defaultAction)
                    .disabled(!canCreate)
            }
        }
        .padding(20)
        .frame(width: 340)
        .onAppear { isFocused = true }
    }

    private func submit() {
        guard canCreate else { return }
        onCreate(name)
    }
}
