import XCTest

@testable import DegenView

final class PineWordRangeTests: XCTestCase {
    func testDottedNameSplitsAtDot() {
        let source = "table.cell(t, 0, 0)" as NSString
        XCTAssertEqual(PineWordRange.range(at: 2, in: source), NSRange(location: 0, length: 5))
        XCTAssertEqual(PineWordRange.range(at: 7, in: source), NSRange(location: 6, length: 4))
    }

    func testNonWordCharacterHasNoRange() {
        let source = "a + b" as NSString
        XCTAssertNil(PineWordRange.range(at: 1, in: source))
        XCTAssertNil(PineWordRange.range(at: 5, in: source))
        XCTAssertNil(PineWordRange.range(at: -1, in: source))
    }

    func testUnderscoreAndDigitsBelongToWord() {
        let source = "x = my_var2 + 1" as NSString
        XCTAssertEqual(PineWordRange.range(at: 6, in: source), NSRange(location: 4, length: 7))
    }

    func testOccurrencesMatchWholeWordsOnly() {
        let source = "len = 1\nlength = len + ta.len\n" as NSString
        XCTAssertEqual(
            PineWordRange.occurrences(of: "len", in: source),
            [NSRange(location: 0, length: 3), NSRange(location: 17, length: 3), NSRange(location: 26, length: 3)]
        )
    }
}

@MainActor
final class PineTextViewSelectionTests: XCTestCase {
    func testDoubleClickSelectsOnlyClickedPartOfDottedName() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 500, height: 200), styleMask: [.titled],
            backing: .buffered, defer: false)
        let editor = LineNumberedTextEditorView.EditorContainerView(frame: window.contentView!.bounds)
        window.contentView!.addSubview(editor)
        editor.setText("x = table.cell(t, 0)")
        window.makeKeyAndOrderFront(nil)
        editor.layoutSubtreeIfNeeded()

        let textView = editor.textView
        let layoutManager = textView.layoutManager!
        let glyphRect = layoutManager.boundingRect(
            forGlyphRange: NSRange(location: 6, length: 1), in: textView.textContainer!)
        let point = textView.convert(
            NSPoint(
                x: glyphRect.midX + textView.textContainerOrigin.x,
                y: glyphRect.midY + textView.textContainerOrigin.y),
            to: nil)
        func event(_ type: NSEvent.EventType, clickCount: Int) -> NSEvent {
            NSEvent.mouseEvent(
                with: type, location: point, modifierFlags: [],
                timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
                context: nil, eventNumber: 0, clickCount: clickCount, pressure: 1)!
        }
        for clickCount in 1...2 {
            window.postEvent(event(.leftMouseUp, clickCount: clickCount), atStart: false)
            textView.mouseDown(with: event(.leftMouseDown, clickCount: clickCount))
        }
        XCTAssertEqual(textView.selectedRange(), NSRange(location: 4, length: 5))
    }
}
