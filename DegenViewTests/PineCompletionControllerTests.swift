import AppKit
import XCTest

@testable import DegenView

/// The completion popup wired into a real editor: typing, keys, mouse, focus and undo.
@MainActor
final class PineCompletionControllerTests: XCTestCase {
    private var window: NSWindow!
    private var editor: LineNumberedTextEditorView.EditorContainerView!
    private var textView: PineTextView { editor.textView as! PineTextView }
    private var completion: PineCompletionController { textView.completion }

    override func setUp() {
        super.setUp()
        window = NSWindow(
            contentRect: NSRect(x: 100, y: 100, width: 600, height: 400), styleMask: [.titled],
            backing: .buffered, defer: false)
        editor = LineNumberedTextEditorView.EditorContainerView(frame: window.contentView!.bounds)
        window.contentView!.addSubview(editor)
        window.makeKeyAndOrderFront(nil)
        textView.undoManager?.groupsByEvent = false
        // Each test starts from its own analysis cache and no libraries on disk.
        textView.completion.analysisCache = PineEditorAnalysisCache()
        textView.completion.libraries = PineNoLibraryExports()
    }

    override func tearDown() {
        completion.dismissAll()
        window.close()
        super.tearDown()
    }

    private func load(_ marked: String) {
        let fixture = PineEditorFixture(marked)
        editor.setText(fixture.text)
        window.makeFirstResponder(textView)
        textView.setSelectedRange(fixture.selection)
    }

    private var rendered: String {
        PineEditorFixture.render(textView.string, textView.selectedRange())
    }

    /// Runs one user action as its own undo group.
    private func act(_ body: () -> Void) {
        textView.undoManager?.beginUndoGrouping()
        body()
        textView.undoManager?.endUndoGrouping()
    }

    /// Types one character as its own undo group, the way the keyboard delivers it.
    private func type(_ text: String) {
        for character in text {
            textView.undoManager?.beginUndoGrouping()
            textView.insertText(String(character), replacementRange: NSRange(location: NSNotFound, length: 0))
            textView.undoManager?.endUndoGrouping()
        }
    }

    private var labels: [String] { completion.items.map(\.label) }

    private func press(keyCode: UInt16, characters: String, modifiers: NSEvent.ModifierFlags = []) -> Bool {
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: modifiers,
            timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
            context: nil, characters: characters, charactersIgnoringModifiers: characters,
            isARepeat: false, keyCode: keyCode)!
        return textView.performKeyEquivalent(with: event)
    }

    // MARK: Opening

    func testTypingAWordOpensTheListAfterTwoLetters() {
        load("plot(|)")
        type("c")
        XCTAssertFalse(completion.isCompletionVisible)
        type("l")
        XCTAssertTrue(completion.isCompletionVisible)
        XCTAssertTrue(labels.contains("close"))
    }

    func testADotOpensMembersAtOnce() {
        load("|")
        type("ta")
        type(".")
        XCTAssertTrue(completion.isCompletionVisible)
        XCTAssertEqual(Set(labels), Set(PineSymbolCatalog.members(of: "ta").map(\.label)))
    }

    func testTypingKeepsFilteringAndAPunctuationCharacterCloses() {
        load("|")
        type("ta.r")
        let wide = labels.count
        type("s")
        XCTAssertEqual(labels, ["rsi"].filter { labels.contains($0) })
        XCTAssertLessThan(labels.count, wide)
        type(" ")
        XCTAssertFalse(completion.isCompletionVisible)
    }

    func testNothingOpensInCommentsOrStrings() {
        load("// |")
        type("ta.r")
        XCTAssertFalse(completion.isCompletionVisible)
        load("x = \"|\"")
        type("ta.r")
        XCTAssertFalse(completion.isCompletionVisible)
    }

    func testDeletingNeverOpensTheList() {
        load("cl|")
        act { textView.deleteBackward(nil) }
        type("l")
        XCTAssertTrue(completion.isCompletionVisible)
        completion.cancel()
        act { textView.deleteBackward(nil) }
        XCTAssertFalse(completion.isCompletionVisible)
    }

    func testMarkedTextNeverOpensTheList() {
        load("|")
        act {
            textView.setMarkedText(
                "cl", selectedRange: NSRange(location: 2, length: 0),
                replacementRange: NSRange(location: NSNotFound, length: 0))
        }
        XCTAssertFalse(completion.isCompletionVisible)
        textView.unmarkText()
    }

    func testExplicitInvocationWorksWithNothingTyped() {
        load("x = |")
        textView.complete(nil)
        XCTAssertTrue(completion.isCompletionVisible)
        XCTAssertTrue(labels.contains("close"))
        XCTAssertTrue(labels.contains("ta"))
    }

    func testControlSpaceInvokesCompletionOnlyWhileTheEditorHasFocus() {
        load("x = |")
        XCTAssertTrue(press(keyCode: 49, characters: " ", modifiers: [.control]))
        XCTAssertTrue(completion.isCompletionVisible)
        completion.dismissAll()
        window.makeFirstResponder(nil)
        XCTAssertFalse(press(keyCode: 49, characters: " ", modifiers: [.control]))
        XCTAssertFalse(completion.isCompletionVisible)
    }

    // MARK: Keys

    func testDownAndUpMoveTheSelectionAndWrap() {
        load("|")
        type("ta.r")
        XCTAssertEqual(completion.selectedIndex, 0)
        textView.moveDown(nil)
        XCTAssertEqual(completion.selectedIndex, 1)
        textView.moveUp(nil)
        textView.moveUp(nil)
        XCTAssertEqual(completion.selectedIndex, completion.items.count - 1)
        XCTAssertEqual(completion.completionList?.selectedIndex, completion.selectedIndex)
    }

    func testReturnAcceptsTheSelectedRowInsteadOfInsertingANewline() {
        load("|")
        type("ta.rs")
        act { textView.insertNewline(nil) }
        XCTAssertEqual(rendered, "ta.rsi(|)")
        XCTAssertFalse(completion.isCompletionVisible)
    }

    func testTabAcceptsToo() {
        load("|")
        type("plot(clo")
        act { textView.insertTab(nil) }
        XCTAssertEqual(textView.string, "plot(close)")
    }

    func testReturnAndTabKeepTheirEditorMeaningWhenTheListIsClosed() {
        load("foo(a) =>\n    x = 1|")
        act { textView.insertNewline(nil) }
        XCTAssertEqual(rendered, "foo(a) =>\n    x = 1\n    |")
        load("|")
        act { textView.insertTab(nil) }
        XCTAssertEqual(textView.string, "    ")
    }

    func testArrowKeysMoveTheCaretWhenTheListIsClosed() {
        load("a\nb|")
        textView.moveUp(nil)
        XCTAssertEqual(textView.selectedRange().location, 1)
    }

    func testEscapeClosesTheListFirstAndSignatureHelpSecond() {
        load("|")
        type("ta.sma(")
        XCTAssertTrue(completion.isSignatureVisible)
        type("cl")
        XCTAssertTrue(completion.isCompletionVisible)
        textView.cancelOperation(nil)
        XCTAssertFalse(completion.isCompletionVisible)
        XCTAssertTrue(completion.isSignatureVisible)
        textView.cancelOperation(nil)
        XCTAssertFalse(completion.isSignatureVisible)
        XCTAssertEqual(textView.string, "ta.sma(cl)")
    }

    func testEscapeReachesTheEditorMonitorBeforeTheFindBar() {
        load("|")
        type("ta.r")
        XCTAssertTrue(completion.isCompletionVisible)
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [],
            timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
            context: nil, characters: "\u{1B}", charactersIgnoringModifiers: "\u{1B}", isARepeat: false,
            keyCode: 53)!
        window.sendEvent(event)
        XCTAssertFalse(completion.isCompletionVisible, "the first Escape closes the list")
    }

    // MARK: Editing

    func testAcceptingIsOneUndoStep() {
        load("|")
        type("ta.rs")
        let before = textView.string
        textView.undoManager?.beginUndoGrouping()
        act { textView.insertNewline(nil) }
        textView.undoManager?.endUndoGrouping()
        XCTAssertEqual(textView.string, "ta.rsi()")
        textView.undoManager?.undo()
        XCTAssertEqual(textView.string, before)
    }

    func testAcceptingACallCooperatesWithPairing() {
        load("|")
        type("plot(ta.rs")
        act { textView.insertNewline(nil) }
        XCTAssertEqual(rendered, "plot(ta.rsi(|))")
        type(")")
        XCTAssertEqual(rendered, "plot(ta.rsi()|)", "typing the closer steps over the inserted one")
        XCTAssertEqual(textView.string.filter { $0 == "(" }.count, textView.string.filter { $0 == ")" }.count)
    }

    func testDelimiterMatchingFollowsAnAcceptedCall() throws {
        load("|")
        type("ta.rs")
        act { textView.insertNewline(nil) }
        let snapshot = PineLexicalSnapshot.shared(for: textView.string)
        let match = try XCTUnwrap(PineDelimiterMatcher.match(at: textView.selectedRange().location, in: snapshot))
        XCTAssertNotNil(match.partner)
    }

    func testAcceptingANamespaceOpensItsMembers() {
        load("x = |")
        type("co")
        act { textView.insertNewline(nil) }
        XCTAssertEqual(textView.string, "x = color.")
        XCTAssertTrue(completion.isCompletionVisible)
        XCTAssertEqual(Set(labels), Set(PineSymbolCatalog.members(of: "color").map(\.label)))
    }

    func testAcceptingACallShowsSignatureHelp() {
        load("|")
        type("ta.rs")
        act { textView.insertNewline(nil) }
        XCTAssertFalse(completion.isCompletionVisible)
        XCTAssertTrue(completion.isSignatureVisible)
        XCTAssertEqual(completion.signatureHelp?.callee, "ta.rsi")
        XCTAssertEqual(completion.signatureHelp?.activeParameter, 0)
    }

    // MARK: Signature help

    func testSignatureHelpTracksTheArgumentUnderTheCaret() {
        load("|")
        type("ta.sma(")
        XCTAssertEqual(completion.signatureHelp?.activeParameter, 0)
        type("close,")
        XCTAssertEqual(completion.signatureHelp?.activeParameter, 1)
        type(" 20")
        XCTAssertEqual(completion.signatureHelp?.activeParameter, 1)
        textView.setSelectedRange(NSRange(location: 8, length: 0))
        XCTAssertEqual(completion.signatureHelp?.activeParameter, 0)
    }

    func testSignatureHelpClosesWhenTheCaretLeavesTheCall() {
        load("|")
        type("ta.sma(close, 20)")
        XCTAssertTrue(completion.isSignatureVisible)
        type(" ")
        XCTAssertFalse(completion.isSignatureVisible)
    }

    // MARK: Mouse

    func testClickingARowSelectsItAndDoubleClickAcceptsWithoutMovingTheCaret() throws {
        load("|")
        type("ta.r")
        let list = try XCTUnwrap(completion.completionList)
        let caret = textView.selectedRange()
        list.onClick?(2, 1)
        XCTAssertEqual(completion.selectedIndex, 2)
        XCTAssertEqual(textView.selectedRange(), caret)
        XCTAssertTrue(window.firstResponder === textView)
        let picked = completion.items[2].label
        act { list.onClick?(2, 2) }
        XCTAssertTrue(textView.string.hasPrefix("ta.\(picked)"))
        XCTAssertFalse(completion.isCompletionVisible)
    }

    // MARK: Anchoring and dismissal

    func testThePopupSitsInsideTheScreenAndAsAChildOfTheEditorWindow() throws {
        load("|")
        type("ta.r")
        let panel = try XCTUnwrap(completion.completionWindow)
        XCTAssertTrue(window.childWindows?.contains(panel) == true)
        XCTAssertFalse(panel.canBecomeKey)
        if let visible = window.screen?.visibleFrame { XCTAssertTrue(visible.contains(panel.frame)) }
    }

    func testLosingFocusDismissesEverythingAndLeavesNoOrphanPanel() {
        load("|")
        type("ta.sma(cl")
        XCTAssertTrue(completion.hasTransientUI)
        window.makeFirstResponder(nil)
        XCTAssertFalse(completion.hasTransientUI)
        XCTAssertTrue(window.childWindows?.filter { $0.isVisible }.isEmpty ?? true)
    }

    func testRemovingTheEditorFromItsWindowDismissesEverything() {
        load("|")
        type("ta.r")
        XCTAssertTrue(completion.isCompletionVisible)
        editor.removeFromSuperview()
        XCTAssertFalse(completion.hasTransientUI)
        XCTAssertTrue(window.childWindows?.filter { $0.isVisible }.isEmpty ?? true)
    }

    func testClickingIntoTheTextDismisses() {
        load("|")
        type("ta.r")
        func mouse(_ type: NSEvent.EventType) -> NSEvent {
            NSEvent.mouseEvent(
                with: type, location: NSPoint(x: 20, y: 20), modifierFlags: [],
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber, context: nil,
                eventNumber: 1, clickCount: 1, pressure: 1)!
        }
        // NSTextView tracks the mouse until the button comes up, so the release is queued first.
        NSApp.postEvent(mouse(.leftMouseUp), atStart: true)
        textView.mouseDown(with: mouse(.leftMouseDown))
        XCTAssertFalse(completion.isCompletionVisible)
    }

    func testMovingTheCaretToAnotherWordDismisses() {
        load("ta.r\nclose|")
        textView.setSelectedRange(NSRange(location: 4, length: 0))
        completion.explicitInvoke()
        XCTAssertTrue(completion.isCompletionVisible)
        textView.setSelectedRange(NSRange(location: 8, length: 0))
        XCTAssertFalse(completion.isCompletionVisible)
    }

    // MARK: Stale results

    func testAResultForAnOlderRequestIsIgnored() throws {
        load("|")
        type("ta.r")
        let stale = completion.generation
        let context = PineCompletionFixture("ta.r|").context
        type("s")
        XCTAssertGreaterThan(completion.generation, stale)
        let before = completion.items
        XCTAssertFalse(
            completion.applyResult(
                [try XCTUnwrap(PineCompletionFixture("ta.|").items().first)], generation: stale, context: context,
                cause: .explicit))
        XCTAssertEqual(completion.items, before)
    }

    func testAcceptingAfterTheTextChangedUnderneathIsRefused() {
        load("|")
        type("ta.rs")
        XCTAssertTrue(completion.isCompletionVisible)
        // Change the text without going through the typing path the controller watches.
        textView.textStorage?.replaceCharacters(in: NSRange(location: 0, length: 0), with: "x")
        XCTAssertFalse(completion.acceptSelected())
        XCTAssertFalse(completion.isCompletionVisible)
        XCTAssertEqual(textView.string, "xta.rs")
    }

    func testAnEmptyResultClosesTheList() {
        load("|")
        type("ta.r")
        type("zz")
        XCTAssertFalse(completion.isCompletionVisible)
    }
}
