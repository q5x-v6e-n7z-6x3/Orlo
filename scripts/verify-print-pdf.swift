// Read-only PDFKit verification of the disposable output from `--example print_qa`.
// Writes text, representative page PNGs and contact sheets beside the QA PDFs.
import AppKit
import PDFKit

let directory = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
let expectedPaper = NSPrintInfo.shared.paperSize
print("Native default paper size: \(expectedPaper.width) × \(expectedPaper.height) points")
var counts: [String: Int] = [:]
var names = ["document", "selection", "numbered-large", "empty", "no-wrap", "color", "selection-multi"]
if FileManager.default.fileExists(atPath: directory.appendingPathComponent("native-panel.pdf").path) {
    names.append("native-panel")
}
for name in names {
    guard let document = PDFDocument(url: directory.appendingPathComponent(name + ".pdf")) else {
        fatalError("Cannot read \(name).pdf")
    }
    counts[name] = document.pageCount
    let text = document.string ?? ""
    print("\(name): \(document.pageCount) pages, \(text.count) characters")
    try text.write(to: directory.appendingPathComponent(name + ".txt"), atomically: true, encoding: .utf8)
    if name == "document" || name == "numbered-large" || name == "no-wrap" || name == "color" {
        precondition(document.pageCount > 1)
        precondition(text.contains("START α café 中"))
        precondition(text.contains("END document"))
        var previous = text.startIndex
        for number in 1...240 {
            let record = String(format: "source %03d", number)
            precondition(text.components(separatedBy: record).count - 1 == 1, "missing or duplicated source record \(number)")
            let position = text.range(of: record)!.lowerBound
            precondition(position >= previous, "out-of-order source record \(number)")
            previous = position
            if name == "numbered-large" {
                let numberedRecord = "(?m)^\(number + 1) +" + record
                precondition(text.range(of: numberedRecord, options: .regularExpression) != nil,
                    "wrong source line number for \(record)")
            }
        }
        // Wrapping and page breaks may add whitespace, but must never lose source words.
        let words = text.split(whereSeparator: { $0.isWhitespace })
        for word in ["wrap", "these", "words"] {
            precondition(words.filter { $0 == word }.count == 2160, "lost wrapped source words: \(word)")
        }
        precondition(text.filter { $0 == "中" }.count == 241)
        precondition(text.contains("😀"))
    }
    if name == "numbered-large" {
        precondition(text.range(of: "(?m)^1 +START", options: .regularExpression) != nil
            && text.range(of: "(?m)^242 +END document", options: .regularExpression) != nil)
    }
    if name == "selection" {
        precondition(document.pageCount == 1 && text.contains("START α café 中"))
        precondition(!text.contains("source 001") && !text.contains("END document"))
    }
    if name == "selection-multi" {
        precondition(document.pageCount > 1)
        var previous = text.startIndex
        for number in 1...240 {
            let record = String(format: "source %03d", number)
            let expected = (20...100).contains(number) || (180...220).contains(number)
            let matches = text.components(separatedBy: record).count - 1
            precondition(matches == (expected ? 1 : 0), "wrong selection scope for \(record)")
            if expected {
                let position = text.range(of: record)!.lowerBound
                let numberedRecord = "(?m)^\(number + 1) +" + record
                precondition(text.range(of: numberedRecord, options: .regularExpression) != nil, "wrong selection source line for \(record)")
                precondition(position >= previous, "selection page order for \(record)")
                previous = position
            }
        }
    }
    if name == "empty" { precondition(document.pageCount == 1 && text.isEmpty) }
    if name == "native-panel" {
        precondition(document.pageCount > 1)
        precondition(text.components(separatedBy: "Native panel cancellation").count - 1 == 120)
    }
    let representatives = Set([0, document.pageCount / 2, document.pageCount - 1])
    let sheet = NSImage(size: NSSize(width: 1000, height: ((document.pageCount + 3) / 4) * 350))
    sheet.lockFocus()
    NSColor.white.setFill()
    NSBezierPath(rect: NSRect(origin: .zero, size: sheet.size)).fill()
    sheet.unlockFocus()
    for index in 0..<document.pageCount {
        let page = document.page(at: index)!
        let media = page.bounds(for: .mediaBox)
        let crop = page.bounds(for: .cropBox)
        precondition(media == document.page(at: 0)!.bounds(for: .mediaBox) && crop == media,
            "inconsistent or cropped page size in \(name) page \(index + 1)")
        if name != "native-panel" {
            precondition(abs(media.width - expectedPaper.width) < 0.1 && abs(media.height - expectedPaper.height) < 0.1,
                "PDF page differs from native paper size")
        }
        if representatives.contains(index) { print("\(name) page \(index + 1): MediaBox = CropBox = \(media)") }
        let image = page.thumbnail(of: NSSize(width: 850, height: 1100), for: .mediaBox)
        if representatives.contains(index) {
            let representation = NSBitmapImageRep(data: image.tiffRepresentation!)!
            try representation.representation(using: .png, properties: [:])!.write(
                to: directory.appendingPathComponent("\(name)-\(index + 1).png"))
        }
        sheet.lockFocus()
        let x = (index % 4) * 250
        let y = Int(sheet.size.height) - ((index / 4) + 1) * 350
        image.draw(in: NSRect(x: x + 10, y: y + 25, width: 230, height: 300))
        NSString(string: "Page \(index + 1)").draw(at: NSPoint(x: x + 10, y: y + 5), withAttributes: [
            .font: NSFont.systemFont(ofSize: 12), .foregroundColor: NSColor.black,
        ])
        sheet.unlockFocus()
    }
    let representation = NSBitmapImageRep(data: sheet.tiffRepresentation!)!
    try representation.representation(using: .png, properties: [:])!.write(
        to: directory.appendingPathComponent(name + "-contact.png"))
}
precondition(counts["numbered-large"]! > counts["document"]!)
precondition(counts["no-wrap"]! < counts["document"]!)
print("PDFKit page/text/options checks passed; inspect the representative pages and contact sheets")
