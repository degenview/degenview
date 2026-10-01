import AppKit
import XCTest

@testable import DegenView

/// The assistance wired into a real editor: typing goes through `insertText`, commands through
/// the shortcut path, and each action must be one undo step.
@MainActor
final class PineTextViewEditingTests: XCTestCase {
    private var window: NSWindow!
    private var editor: LineNumberedTextEditorView.EditorContainerView!
    private var textView: NSTextView { editor.textView }

    override func setUp() {
        super.setUp()
        window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 500, height: 300), styleMask: [.titled],
            backing: .buffered, defer: false)
        editor = LineNumberedTextEditorView.EditorContainerView(frame: window.contentView!.bounds)
        window.contentView!.addSubview(editor)
        window.makeKeyAndOrderFront(nil)
        textView.undoManager?.groupsByEvent = false
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

    private func type(_ text: String) {
        act { textView.insertText(text, replacementRange: NSRange(location: NSNotFound, length: 0)) }
    }

    private func undo() { textView.undoManager?.undo() }

    func testTypingAnOpeningParenthesisInsertsThePair() {
        load("ta.sma|")
        type("(")
        XCTAssertEqual(rendered, "ta.sma(|)")
    }

    func testOneUndoRemovesTheParenthesisAndItsPair() {
        load("ta.sma|")
        type("(")
        undo()
        XCTAssertEqual(textView.string, "ta.sma")
    }

    func testOvertypeMovesPastTheCloser() {
        load("foo(a|)")
        type(")")
        XCTAssertEqual(rendered, "foo(a)|")
        XCTAssertEqual(textView.string, "foo(a)")
    }

    func testBackspaceDeletesTheEmptyPairAndUndoRestoresIt() {
        load("f(|)")
        act { textView.deleteBackward(nil) }
        XCTAssertEqual(rendered, "f|")
        undo()
        XCTAssertEqual(textView.string, "f()")
    }

    func testWrappingASelectionIsOneUndoStep() {
        load("⟦close⟧")
        type("(")
        XCTAssertEqual(rendered, "(⟦close⟧)")
        undo()
        XCTAssertEqual(textView.string, "close")
    }

    func testReturnAfterABlockOpenerIndentsAndUndoes() {
        load("if a|")
        act { textView.insertNewline(nil) }
        XCTAssertEqual(rendered, "if a\n    |")
        undo()
        XCTAssertEqual(textView.string, "if a")
    }

    func testTabIndentsASelectionAsOneUndoStep() {
        load("⟦x = close\ny = open⟧")
        act { textView.insertTab(nil) }
        XCTAssertEqual(rendered, "⟦    x = close\n    y = open⟧")
        undo()
        XCTAssertEqual(textView.string, "x = close\ny = open")
    }

    func testShiftTabOutdents() {
        load("⟦    x\n    y⟧")
        act { textView.insertBacktab(nil) }
        XCTAssertEqual(rendered, "⟦x\ny⟧")
    }

    func testTogglingACommentIsOneUndoStep() {
        load("plot(close)|")
        act { _ = self.pressShortcut(keyCode: 44, characters: "/", modifiers: [.command]) }
        XCTAssertEqual(rendered, "// plot(close)|")
        undo()
        XCTAssertEqual(textView.string, "plot(close)")
    }

    func testMovingLinesWithTheShortcutKeepsTheSelection() {
        load("a|\nb")
        act { _ = self.pressShortcut(keyCode: 125, characters: "\u{F701}", modifiers: [.option, .function]) }
        XCTAssertEqual(rendered, "b\na|")
        undo()
        XCTAssertEqual(textView.string, "a\nb")
    }

    func testDuplicatingALineWithTheShortcut() {
        load("a|")
        act { _ = self.pressShortcut(keyCode: 125, characters: "\u{F701}", modifiers: [.option, .shift, .function]) }
        XCTAssertEqual(rendered, "a\na|")
    }

    func testMarkedTextDisablesAssistance() {
        load("a|")
        act {
            textView.setMarkedText(
                "k", selectedRange: NSRange(location: 1, length: 0),
                replacementRange: NSRange(location: NSNotFound, length: 0))
        }
        XCTAssertTrue(textView.hasMarkedText())
        XCTAssertNil((textView as? PineTextView)?.editingContext)
        act { textView.unmarkText() }
    }

    func testEditsKeepDiagnosticUnderlinesOnTheRightText() {
        load("x = 1\ny = |")
        editor.setDiagnostics([
            PineDiagnostic(
                code: "TEST", severity: .error, category: .syntax, message: "bad",
                range: PineSourceRange(
                    start: PineSourcePosition(line: 1, column: 1, offset: 0),
                    end: PineSourcePosition(line: 1, column: 2, offset: 1)))
        ])
        type("(")
        let underline = textView.textStorage!.attribute(.underlineStyle, at: 0, effectiveRange: nil)
        XCTAssertNotNil(underline)
        XCTAssertNil(textView.textStorage!.attribute(.underlineStyle, at: 8, effectiveRange: nil))
    }

    func testBracketMatchPaintsTemporaryAttributesOnly() {
        load("f(a|)")
        editor.updateOccurrenceHighlights()
        let layout = textView.layoutManager!
        XCTAssertNotNil(layout.temporaryAttribute(.backgroundColor, atCharacterIndex: 1, effectiveRange: nil))
        XCTAssertNotNil(layout.temporaryAttribute(.backgroundColor, atCharacterIndex: 3, effectiveRange: nil))
        XCTAssertNil(textView.textStorage!.attribute(.backgroundColor, at: 1, effectiveRange: nil))
        XCTAssertEqual(textView.string, "f(a)")
    }

    func testOccurrencesIgnoreStringsCommentsAndSubstrings() {
        load("fast = 1\nplot(fast|)\nx = \"fast\" // fast\nfaster = 2")
        editor.updateOccurrenceHighlights()
        let layout = textView.layoutManager!
        let source = textView.string as NSString
        func tinted(_ offset: Int) -> Bool {
            layout.temporaryAttribute(.backgroundColor, atCharacterIndex: offset, effectiveRange: nil) != nil
        }
        XCTAssertTrue(tinted(source.range(of: "fast").location))
        XCTAssertTrue(tinted(source.range(of: "fast", options: [], range: NSRange(location: 9, length: 12)).location))
        XCTAssertFalse(tinted(source.range(of: "\"fast\"").location + 1))
        XCTAssertFalse(tinted(source.range(of: "// fast").location + 3))
        XCTAssertFalse(tinted(source.range(of: "faster").location))
    }

    func testIndentGuideAndCurrentLineBandAreDrawnBehindTheText() throws {
        load("if a\n    x\n    y|")
        editor.layoutSubtreeIfNeeded()
        let layout = textView.layoutManager!
        layout.ensureLayout(for: textView.textContainer!)
        let bounds = textView.bounds
        let bitmap = try XCTUnwrap(textView.bitmapImageRepForCachingDisplay(in: bounds))
        textView.cacheDisplay(in: bounds, to: bitmap)

        let origin = textView.textContainerOrigin
        func rect(ofLineContaining character: Int) -> NSRect {
            layout.lineFragmentRect(
                forGlyphAt: layout.glyphIndexForCharacter(at: character), effectiveRange: nil)
        }
        // The bitmap is in device pixels; a Retina backing store doubles the point coordinates.
        let scale = CGFloat(bitmap.pixelsWide) / bounds.width
        func pixel(_ x: CGFloat, _ line: NSRect) -> NSColor {
            bitmap.colorAt(x: Int(x * scale), y: Int((line.midY + origin.y) * scale))!
                .usingColorSpace(.sRGB)!
        }
        let second = rect(ofLineContaining: 5)
        let guideX = origin.x + textView.textContainer!.lineFragmentPadding
        // Line two is not the caret line, so only the guide separates these two pixels.
        XCTAssertNotEqual(pixel(guideX, second), pixel(guideX + 4, second))
        // The caret's own line is tinted across its width; an ordinary line is not.
        let first = rect(ofLineContaining: 0)
        let caretLine = rect(ofLineContaining: 11)
        XCTAssertNotEqual(pixel(bounds.width - 20, caretLine), pixel(bounds.width - 20, first))
    }

    // MARK: - Helpers

    private func pressShortcut(
        keyCode: UInt16, characters: String, modifiers: NSEvent.ModifierFlags
    ) -> Bool {
        let event = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: modifiers,
            timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
            context: nil, characters: characters, charactersIgnoringModifiers: characters,
            isARepeat: false, keyCode: keyCode)!
        return textView.performKeyEquivalent(with: event)
    }
}
