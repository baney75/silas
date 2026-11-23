import XCTest
@testable import SilasCore

/// dictionary.json: a file that is there but can't be read is never treated as an empty dictionary.
final class DictionaryFileTests: XCTestCase {
    var dir: URL!
    var file: URL { dir.appendingPathComponent("dictionary.json") }

    override func setUpWithError() throws {
        dir = FileManager.default.temporaryDirectory.appendingPathComponent("silas-dict-\(UUID())")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: dir.path)
        try? FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: file.path)
        try? FileManager.default.removeItem(at: dir)
    }

    func testMissingThenSavedThenLoaded() throws {
        XCTAssertEqual(DictionaryFile.load(from: file), .missing)
        var d = PersonalDictionary()
        d.addTerm("Parish1", source: "user")
        try DictionaryFile.save(d, to: file)
        XCTAssertEqual(DictionaryFile.load(from: file), .ok(d))
    }

    /// Protected data unavailable reads like a file the process may not open: unreadable, and
    /// the file is left exactly as it was.
    func testUnreadableIsNotEmpty() throws {
        var d = PersonalDictionary()
        d.addTerm("Parish1", source: "user")
        try DictionaryFile.save(d, to: file)
        let bytes = try Data(contentsOf: file)
        try FileManager.default.setAttributes([.posixPermissions: 0], ofItemAtPath: file.path)
        XCTAssertEqual(DictionaryFile.load(from: file), .unreadable)
        try FileManager.default.setAttributes([.posixPermissions: 0o644], ofItemAtPath: file.path)
        XCTAssertEqual(try Data(contentsOf: file), bytes)
        XCTAssertEqual(DictionaryFile.load(from: file), .ok(d), "loads once it can be read again")
    }

    func testCorruptIsMovedAside() throws {
        try Data("not json".utf8).write(to: file)
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        guard case .corrupt(let aside) = DictionaryFile.load(from: file, now: now) else {
            return XCTFail("a file that doesn't decode is corrupt")
        }
        XCTAssertEqual(aside.lastPathComponent, "dictionary.corrupt-1800000000.json")
        XCTAssertEqual(try Data(contentsOf: aside), Data("not json".utf8))
        XCTAssertFalse(FileManager.default.fileExists(atPath: file.path))
    }

    /// When the corrupt file can't be moved aside it stays in place and nothing may write over it.
    func testCorruptThatCannotMoveIsUnreadable() throws {
        try Data("not json".utf8).write(to: file)
        try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: dir.path)
        XCTAssertEqual(DictionaryFile.load(from: file), .unreadable)
        XCTAssertEqual(try Data(contentsOf: file), Data("not json".utf8))
    }
}
