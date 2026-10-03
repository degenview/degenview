import AppKit

/// Owns the completion session of one `PineTextView`: when the list opens, what it holds, which
/// row is selected, and the signature help beside it. It decides nothing about the language
/// (that is `PineCompletionEngine` and `PineSignatureResolver`) and draws nothing itself (that is
/// the two panels); it connects the text view's events to them.
///
/// Computation is synchronous and cheap, but every result still carries the generation it was
/// asked for, so a result that arrives after a newer request is dropped rather than shown over
/// text it no longer describes.
@MainActor
final class PineCompletionController {
    private weak var textView: PineTextView?

    var analysisCache: PineEditorAnalysisCache = .shared
    var libraries: PineLibraryExportProviding = PineLibraryExportDirectory.shared

    private(set) var items: [PineCompletionItem] = []
    private(set) var selectedIndex = 0
    private(set) var signatureHelp: PineSignatureHelp?
    /// Increments with every request; results for an older one are ignored.
    private(set) var generation = 0

    private var presentedText = ""
    /// Where the word being completed starts; moving the caret to another word ends the session.
    private var sessionWordStart = 0
    private var selectedID: String?
    private var isApplying = false
    /// Signature help follows the caret only once something asked for it (`(`, `,`, a call
    /// completion, an explicit request), and stays off after it is dismissed with Escape.
    private var signatureEnabled = false

    private var completionPanel: PineCompletionPanel?
    private var signaturePanel: PineSignaturePanel?
    private var observers: [NSObjectProtocol] = []

    init(textView: PineTextView) {
        self.textView = textView
    }

    isolated deinit {
        removeObservers()
    }

    /// The panels' content and windows, for tests and for the text view's own bookkeeping.
    var completionList: PineCompletionListView? { completionPanel?.list }
    var completionWindow: NSWindow? { completionPanel }
    var signatureWindow: NSWindow? { signaturePanel }

    var isCompletionVisible: Bool { completionPanel?.isVisible == true && !items.isEmpty }
    var isSignatureVisible: Bool { signaturePanel?.isVisible == true && signatureHelp != nil }
    /// Whether Escape has something to close.
    var hasTransientUI: Bool { isCompletionVisible || isSignatureVisible }

    // MARK: - Events from the text view

    /// The text changed. `cause` is what the user did, or nil for a change nobody typed (paste, undo,
    /// a programmatic edit), which refreshes an open list but never opens one.
    func textDidChange(_ cause: PineCompletionTrigger.Cause?) {
        guard !isApplying else { return }
        guard let (analysis, context) = analyse(explicit: false) else {
            dismissAll()
            return
        }
        var forcesSignature = false
        switch cause {
        case .typed(let text):
            forcesSignature = text == "(" || text == ","
            // An open list keeps filtering while the word grows; anything else must pass the trigger.
            if PineCompletionTrigger.opens(for: .typed(text), in: context) || (isCompletionVisible && isWord(text)) {
                present(context, analysis, cause: .typed(text))
            } else {
                hideCompletion()
            }
        case .deleted, nil:
            if isCompletionVisible { refreshSameWord(context, analysis) }
        case .explicit:
            present(context, analysis, cause: .explicit)
        }
        updateSignature(analysis, context, force: forcesSignature)
    }

    /// The caret moved without the text changing.
    func selectionDidChange() {
        guard !isApplying, hasTransientUI, let (analysis, context) = analyse(explicit: false) else {
            if !isApplying, hasTransientUI { dismissAll() }
            return
        }
        if isCompletionVisible { refreshSameWord(context, analysis) }
        updateSignature(analysis, context, force: false)
    }

    /// Control-Space, Option-Escape or F5: complete here, even with nothing typed.
    func explicitInvoke() {
        guard let (analysis, context) = analyse(explicit: true) else { return }
        present(context, analysis, cause: .explicit)
        signatureEnabled = true
        updateSignature(analysis, context, force: true)
    }

    /// Down and Up while the list is open; the selection wraps.
    @discardableResult
    func moveSelection(by delta: Int) -> Bool {
        guard isCompletionVisible else { return false }
        selectedIndex = (selectedIndex + delta + items.count) % items.count
        selectedID = items[selectedIndex].id
        completionPanel?.list.select(selectedIndex)
        return true
    }

    /// Return or Tab while a row is selected: the one edit that completes it.
    @discardableResult
    func acceptSelected() -> Bool {
        guard isCompletionVisible, items.indices.contains(selectedIndex), let textView,
            let context = textView.editingContext
        else { return false }
        let item = items[selectedIndex]
        // The list describes the text it was computed for; if that changed, it is stale.
        guard textView.string == presentedText,
            let acceptance = PineCompletionInsertion.accept(item, in: context)
        else {
            dismissAll()
            return false
        }
        hideCompletion()
        isApplying = true
        textView.perform(acceptance.edit, actionName: "Autocomplete")
        isApplying = false
        guard let (analysis, newContext) = analyse(explicit: false) else { return true }
        switch acceptance.followUp {
        case .memberCompletion:
            present(newContext, analysis, cause: .explicit)
            updateSignature(analysis, newContext, force: false)
        case .signatureHelp:
            signatureEnabled = true
            updateSignature(analysis, newContext, force: true)
        case .none:
            updateSignature(analysis, newContext, force: false)
        }
        return true
    }

    /// Escape: closes the list first, signature help second. Returns whether anything was open.
    @discardableResult
    func cancel() -> Bool {
        if isCompletionVisible {
            hideCompletion()
            return true
        }
        if isSignatureVisible {
            signatureEnabled = false
            hideSignature()
            return true
        }
        return false
    }

    func dismissAll() {
        signatureEnabled = false
        hideCompletion()
        hideSignature()
    }

    /// Shows `found` if it still answers the latest request. The generation guards the case where
    /// the text moved on while a result was on its way.
    @discardableResult
    func applyResult(
        _ found: [PineCompletionItem], generation requested: Int, context: PineCompletionContext,
        cause: PineCompletionTrigger.Cause
    ) -> Bool {
        guard requested == generation, let textView else { return false }
        guard PineCompletionTrigger.shouldPresent(found, for: cause, in: context) else {
            hideCompletion()
            return true
        }
        let selection = found.firstIndex { $0.id == selectedID } ?? 0
        items = found
        selectedIndex = selection
        selectedID = found[selection].id
        presentedText = textView.string
        sessionWordStart = context.replacementRange.location
        layoutPanels(context: context)
        return true
    }

    // MARK: - Computing

    private func analyse(explicit: Bool) -> (PineEditorAnalysisSnapshot, PineCompletionContext)? {
        guard let textView, textView.editingContext != nil else { return nil }
        let analysis = analysisCache.analysis(for: textView.string)
        let context = PineCompletionContext(
            analysis: analysis, selection: textView.selectedRange(), explicit: explicit)
        return (analysis, context)
    }

    private func present(
        _ context: PineCompletionContext, _ analysis: PineEditorAnalysisSnapshot,
        cause: PineCompletionTrigger.Cause
    ) {
        generation += 1
        let found = PineCompletionEngine.complete(context, analysis: analysis, libraries: libraries)
        applyResult(found, generation: generation, context: context, cause: cause)
    }

    /// Re-filters an open list when the caret stays in the same word, and closes it otherwise.
    private func refreshSameWord(_ context: PineCompletionContext, _ analysis: PineEditorAnalysisSnapshot) {
        guard context.suppression == nil, context.replacementRange.location == sessionWordStart else {
            hideCompletion()
            return
        }
        present(context, analysis, cause: .deleted)
    }

    private func updateSignature(
        _ analysis: PineEditorAnalysisSnapshot, _ context: PineCompletionContext, force: Bool
    ) {
        guard force || signatureEnabled || isSignatureVisible else { return }
        guard let help = PineSignatureResolver.help(for: context, analysis: analysis, libraries: libraries)
        else {
            hideSignature()
            return
        }
        if force { signatureEnabled = true }
        signatureHelp = help
        layoutPanels(context: context)
    }

    private func isWord(_ text: String) -> Bool {
        guard text.count == 1, let character = text.first else { return false }
        return character.isLetter || character.isNumber || character == "_"
    }

    // MARK: - Panels

    private func layoutPanels(context: PineCompletionContext) {
        guard let textView, let window = textView.window else { return }
        var placement: PineCompletionPanel.Placement = .below
        if !items.isEmpty,
            let caret = textView.screenRect(forCharacterOffset: context.replacementRange.location)
        {
            let panel = completionPanel ?? PineCompletionPanel()
            panel.list.onClick = { [weak self] row, count in self?.handleClick(row: row, count: count) }
            completionPanel = panel
            placement = panel.present(
                items, prefix: context.prefix, selected: selectedIndex, caret: caret, in: window)
        }
        if let help = signatureHelp, signatureEnabled,
            let caret = textView.screenRect(forCharacterOffset: context.caret)
        {
            // The list and the help never stack: a list that had to open above the caret wins.
            if !items.isEmpty, placement == .above {
                signaturePanel?.dismiss()
            } else {
                let panel = signaturePanel ?? PineSignaturePanel()
                signaturePanel = panel
                panel.present(help, caret: caret, in: window)
            }
        }
        installObservers()
    }

    private func handleClick(row: Int, count: Int) {
        guard items.indices.contains(row) else { return }
        selectedIndex = row
        selectedID = items[row].id
        if count >= 2 { acceptSelected() }
    }

    private func hideCompletion() {
        items = []
        selectedIndex = 0
        selectedID = nil
        completionPanel?.dismiss()
        removeObserversIfIdle()
    }

    private func hideSignature() {
        signatureHelp = nil
        signaturePanel?.dismiss()
        removeObserversIfIdle()
    }

    // MARK: - Following the editor

    private func installObservers() {
        guard observers.isEmpty, let textView, let window = textView.window else { return }
        let center = NotificationCenter.default
        func observe(
            _ name: Notification.Name, _ object: Any?,
            _ handler: @escaping @MainActor (PineCompletionController) -> Void
        ) {
            observers.append(
                center.addObserver(forName: name, object: object, queue: .main) { [weak self] _ in
                    MainActor.assumeIsolated { if let self { handler(self) } }
                })
        }
        if let clip = textView.enclosingScrollView?.contentView {
            clip.postsBoundsChangedNotifications = true
            observe(NSView.boundsDidChangeNotification, clip) { $0.reposition() }
        }
        observe(NSWindow.didResizeNotification, window) { $0.reposition() }
        observe(NSWindow.didMoveNotification, window) { $0.reposition() }
        observe(NSWindow.didResignKeyNotification, window) { $0.dismissAll() }
    }

    private func removeObserversIfIdle() {
        if !isCompletionVisible, !isSignatureVisible { removeObservers() }
    }

    private func removeObservers() {
        for observer in observers { NotificationCenter.default.removeObserver(observer) }
        observers = []
    }

    /// The editor scrolled or its window moved: put the panels back under the caret, or close them
    /// when the caret is no longer in view.
    private func reposition() {
        guard hasTransientUI, let (_, context) = analyse(explicit: false), let textView else { return }
        guard let caret = textView.screenRect(forCharacterOffset: context.caret),
            let window = textView.window
        else {
            dismissAll()
            return
        }
        let inView = textView.convert(window.convertFromScreen(caret), from: nil)
        guard textView.visibleRect.intersects(inView) else {
            dismissAll()
            return
        }
        layoutPanels(context: context)
    }
}
