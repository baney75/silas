import CryptoKit
import XCTest
@testable import SilasCore

/// Every case in docs/sync-vectors.json, read from the repo (never a copy), so the desktop and
/// iOS are held to the same file.
final class SyncVectorTests: XCTestCase {
    struct Vectors: Decodable {
        struct Pairing: Decodable {
            let key_hex: String
            let code: String
            let parse_ok: [String]
            let parse_fail: [String]
        }
        struct Frame: Decodable {
            let nonce_hex: String
            let aad: String
            let plaintext: String
            let frame_hex: String
        }
        struct Order: Decodable {
            let a: HLC
            let b: HLC
            let cmp: Int
        }
        struct ClockCase: Decodable {
            struct MSC: Decodable { let ms: UInt64; let c: UInt32 }
            let name: String
            let clock: MSC
            let now: UInt64
            let local: MSC
        }
        struct MergeCase: Decodable {
            let name: String
            let states: [SyncState]
            let merged: SyncState
        }
        let pairing: Pairing
        let group_id: String
        let frame_key_hex: String
        let frame: Frame
        let norm: [[String]]
        let hlc_order: [Order]
        let clock: [ClockCase]
        let merge: [MergeCase]
    }

    var v: Vectors!
    var key: Data!

    override func setUpWithError() throws {
        let repo = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        let data = try Data(contentsOf: repo.appendingPathComponent("docs/sync-vectors.json"))
        v = try JSONDecoder().decode(Vectors.self, from: data)
        key = Data(hex: v.pairing.key_hex)
    }

    func testPairingCodeEncodes() {
        XCTAssertEqual(SyncKey.pairingCode(key), v.pairing.code)
        XCTAssertEqual(v.pairing.code.count, 64)  // 52 symbols, 13 groups, 12 dashes
    }

    func testPairingCodeParses() {
        for s in v.pairing.parse_ok {
            XCTAssertEqual(SyncKey.parse(s), key, s)
        }
        for s in v.pairing.parse_fail {
            XCTAssertNil(SyncKey.parse(s), s)
        }
    }

    func testPairURLRoundTrips() {
        let url = SyncKey.pairURL(key)
        XCTAssertEqual(url.absoluteString, "silas://pair?k=" + v.pairing.code.replacingOccurrences(of: "-", with: ""))
        XCTAssertEqual(SyncKey.key(fromPairURL: url), key)
        XCTAssertNil(SyncKey.key(fromPairURL: URL(string: "silas://dictate")!))
    }

    func testGroupID() {
        XCTAssertEqual(SyncKey.groupID(key), v.group_id)
    }

    func testFrameKey() {
        let fk = SyncKey.frameKey(key).withUnsafeBytes { Data($0) }
        XCTAssertEqual(fk.hex, v.frame_key_hex)
    }

    func testFrameOpensToKnownPlaintext() throws {
        XCTAssertEqual(v.frame.aad, "silas-sync v1")
        let plain = try SyncFrame.open(Data(hex: v.frame.frame_hex)!, key: key)
        XCTAssertEqual(String(decoding: plain, as: UTF8.self), v.frame.plaintext)
        let msg = try StateMessage.decode(plain)
        XCTAssertEqual(msg.entries["t:parish1"]?.v?["text"], "Parish1")
    }

    func testFrameSealsToKnownBytes() throws {
        let sealed = try SyncFrame.seal(
            Data(v.frame.plaintext.utf8), key: key, nonce: Data(hex: v.frame.nonce_hex)!)
        XCTAssertEqual(sealed.hex, v.frame.frame_hex)
    }

    func testForeignFrameIsRefused() throws {
        var other = key!
        other[0] ^= 1
        XCTAssertThrowsError(try SyncFrame.open(Data(hex: v.frame.frame_hex)!, key: other)) {
            XCTAssertEqual($0 as? SyncError, .foreignFrame)
        }
    }

    func testNorm() {
        for pair in v.norm {
            XCTAssertEqual(syncNorm(pair[0]), pair[1], pair[0])
        }
    }

    func testHLCOrder() {
        for o in v.hlc_order {
            let cmp = o.a < o.b ? -1 : (o.b < o.a ? 1 : 0)
            XCTAssertEqual(cmp, o.cmp, "\(o.a) vs \(o.b)")
        }
    }

    func testClock() {
        for c in v.clock {
            var clock = SyncClock(ms: c.clock.ms, c: c.clock.c)
            let t = clock.tick(now: c.now)
            XCTAssertEqual(t.ms, c.local.ms, c.name)
            XCTAssertEqual(t.c, c.local.c, c.name)
            XCTAssertEqual(clock, SyncClock(ms: c.local.ms, c: c.local.c), c.name)
        }
    }

    func testMergeInEveryOrder() {
        for m in v.merge {
            for order in permutations(Array(m.states.indices)) {
                let merged = order.map { m.states[$0] }.reduce(SyncState()) { SyncMerge.merge($0, $1) }
                XCTAssertEqual(merged, m.merged, "\(m.name) order \(order)")
            }
            // Idempotent: merging the result again changes nothing.
            XCTAssertEqual(SyncMerge.merge(m.merged, m.merged), m.merged, m.name)
            for s in m.states {
                XCTAssertEqual(SyncMerge.merge(m.merged, s), m.merged, "\(m.name) replay")
            }
            // Associative: (a+b)+c == a+(b+c) for three-way cases.
            if m.states.count == 3 {
                let (a, b, c) = (m.states[0], m.states[1], m.states[2])
                XCTAssertEqual(
                    SyncMerge.merge(SyncMerge.merge(a, b), c), SyncMerge.merge(a, SyncMerge.merge(b, c)), m.name)
            }
        }
    }

    private func permutations(_ xs: [Int]) -> [[Int]] {
        if xs.count <= 1 { return [xs] }
        return xs.indices.flatMap { i -> [[Int]] in
            var rest = xs
            let x = rest.remove(at: i)
            return permutations(rest).map { [x] + $0 }
        }
    }
}

final class SyncLedgerTests: XCTestCase {
    func dict(_ terms: [String]) -> PersonalDictionary {
        var d = PersonalDictionary()
        for t in terms { d.addTerm(t, source: "user") }
        return d
    }

    func testLocalChangesAreStampedOnce() {
        var l = SyncLedger(deviceID: String(repeating: "a", count: 32))
        var d = dict(["Parish1"])
        XCTAssertTrue(l.recordLocal(d, now: 100))
        XCTAssertEqual(l.entries["t:parish1"]?.h, HLC(ms: 100, c: l.entries["t:parish1"]!.h.c, d: l.deviceID))
        // A projection that changes nothing stamps nothing.
        XCTAssertFalse(l.recordLocal(d, now: 200))
        d.removeTerm("Parish1")
        XCTAssertTrue(l.recordLocal(d, now: 300))
        XCTAssertNil(l.entries["t:parish1"]?.v)
        XCTAssertEqual(l.entries["t:parish1"]?.h.ms, 300)
    }

    func testTwoDevicesConverge() throws {
        let key = SyncKey.generate()
        var a = SyncLedger(deviceID: String(repeating: "a", count: 32))
        var b = SyncLedger(deviceID: String(repeating: "b", count: 32))
        var da = dict(["Parish1", "Ghostty"])
        var db = dict(["Rectory1"])
        db.addCorrection("parish one", "Parish1", source: "user")
        a.recordLocal(da, now: 10)
        b.recordLocal(db, now: 11)

        // A sends to B through a real frame.
        let frame = try a.message(name: "Mac", platform: "macos", now: 12).sealed(key: key)
        let msg = try StateMessage.opened(frame, key: key)
        b.receive(msg.entries)
        b.saw(msg)
        XCTAssertEqual(SyncProjection.apply(b.entries, to: &db), 2)
        XCTAssertEqual(Set(db.terms.map(\.text)), ["Parish1", "Ghostty", "Rectory1"])
        XCTAssertEqual(b.peers.map(\.name), ["Mac"])

        // B deletes Ghostty; A edits nothing; after both ways, both agree.
        db.removeTerm("Ghostty")
        b.recordLocal(db, now: 20)
        a.receive(b.entries)
        XCTAssertNotNil(SyncProjection.apply(a.entries, to: &da))
        XCTAssertEqual(Set(da.terms.map(\.text)), ["Parish1", "Rectory1"])
        XCTAssertEqual(da.corrections.map(\.to), ["Parish1"])
        XCTAssertEqual(a.entries, b.entries)
        // Applying again changes nothing.
        XCTAssertNil(SyncProjection.apply(a.entries, to: &da))
    }

    func testLocalOnlyFieldsSurvive() {
        var l = SyncLedger(deviceID: String(repeating: "a", count: 32))
        var d = dict(["Parish1"])
        d.terms[0].uses = 7
        d.pending = [Pending(from: "x", to: "y", count: 1, lastSeen: "")]
        l.recordLocal(d, now: 1)
        var incoming = l.entries
        incoming["t:parish1"] = SyncEntry(
            v: .object(["text": .string("Parish1"), "added": .string("later"), "source": .string("edit")]),
            h: HLC(ms: 5, c: 0, d: String(repeating: "b", count: 32)))
        l.receive(incoming)
        XCTAssertNotNil(SyncProjection.apply(l.entries, to: &d))
        XCTAssertEqual(d.terms[0].uses, 7)
        XCTAssertEqual(d.terms[0].added, "later")
        XCTAssertEqual(d.pending.count, 1)
        XCTAssertEqual(l.clock.ms, 5)
    }

    func testUnknownKindsAreIgnored() throws {
        let json = #"{"type":"state","v":2,"device":"00","name":"Future","platform":"x","sent":1,"entries":{"t:a":{"v":{"text":"A","added":"","source":"u"},"h":{"ms":1,"c":0,"d":"00"}},"z:new":{"v":{"what":1},"h":{"ms":1,"c":0,"d":"00"}}}}"#
        let m = try StateMessage.decode(Data(json.utf8))
        XCTAssertEqual(Array(m.entries.keys), ["t:a"])
    }

    func testTombstoneEncodesAsNull() throws {
        let e = SyncEntry(v: nil, h: HLC(ms: 1, c: 0, d: "d"))
        let s = String(decoding: try JSONEncoder().encode(e), as: UTF8.self)
        XCTAssertTrue(s.contains(#""v":null"#), s)
    }
}

final class SyncTransportTests: XCTestCase {
    func testFolderRoundTrip() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("silas-sync-\(UUID())")
        defer { try? FileManager.default.removeItem(at: dir) }
        let key = SyncKey.generate()
        let folder = SyncFolder(root: dir)
        let a = SyncPeer(ledger: SyncLedger(), keys: MemoryKeyStore(key), name: "iPhone", platform: "ios")
        let b = SyncPeer(ledger: SyncLedger(), keys: MemoryKeyStore(key), name: "Mac", platform: "macos")
        var da = PersonalDictionary()
        da.addTerm("Parish1", source: "user")
        var db = PersonalDictionary()
        db.addTerm("Rectory1", source: "user")

        _ = try await a.syncFolder(folder, dict: da)
        let got = try await b.syncFolder(folder, dict: db)
        XCTAssertEqual(got.count, 1)
        XCTAssertEqual(got[0].from, "iPhone")
        db = got[0].dictionary ?? db
        XCTAssertEqual(Set(db.terms.map(\.text)), ["Parish1", "Rectory1"])
        let back = try await a.syncFolder(folder, dict: da)
        da = back.first?.dictionary ?? da
        XCTAssertEqual(Set(da.terms.map(\.text)), ["Parish1", "Rectory1"])
        let files = try FileManager.default.contentsOfDirectory(atPath: folder.directory.path)
        XCTAssertEqual(files.filter { $0.hasSuffix(".silas") }.count, 2)
    }

    func testFolderSkipsAnotherGroup() async throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("silas-sync-\(UUID())")
        defer { try? FileManager.default.removeItem(at: dir) }
        let folder = SyncFolder(root: dir)
        let a = SyncPeer(ledger: SyncLedger(), keys: MemoryKeyStore(SyncKey.generate()), name: "A", platform: "ios")
        let b = SyncPeer(ledger: SyncLedger(), keys: MemoryKeyStore(SyncKey.generate()), name: "B", platform: "ios")
        var d = PersonalDictionary()
        d.addTerm("Secret", source: "user")
        _ = try await a.syncFolder(folder, dict: d)
        let got = try await b.syncFolder(folder, dict: PersonalDictionary())
        XCTAssertTrue(got.isEmpty)
    }

    /// Two peers over a real TCP connection on loopback, length-prefixed frames both ways.
    func testLocalNetworkExchange() async throws {
        let key = SyncKey.generate()
        let a = SyncPeer(ledger: SyncLedger(), keys: MemoryKeyStore(key), name: "iPad", platform: "ipados")
        let b = SyncPeer(ledger: SyncLedger(), keys: MemoryKeyStore(key), name: "Mac", platform: "macos")
        let box = Box()
        await box.set(a: { var d = PersonalDictionary(); d.addTerm("Parish1", source: "user"); return d }(),
                      b: { var d = PersonalDictionary(); d.addTerm("Rectory1", source: "user"); return d }())

        let server = SyncNetwork()
        server.ownFrame = { try? await b.frame(for: await box.b) }
        server.receivedFrame = { frame, _ in
            if let o = try? await b.receive(frame, into: await box.b), let d = o.dictionary { await box.setB(d) }
        }
        try await server.start(
            identity: .init(name: "Mac", groupID: SyncKey.groupID(key), deviceID: await b.ledger.deviceID),
            port: 0, advertise: false)
        defer { server.stop() }
        let port = try XCTUnwrap(server.port)

        let client = SyncNetwork()
        client.ownFrame = { try? await a.frame(for: await box.a) }
        client.receivedFrame = { frame, _ in
            if let o = try? await a.receive(frame, into: await box.a), let d = o.dictionary { await box.setA(d) }
        }
        try await client.sync(host: "127.0.0.1", port: port)

        // The server merges after it replies; give it a moment.
        for _ in 0..<50 where await box.b.terms.count < 2 {
            try await Task.sleep(nanoseconds: 50_000_000)
        }
        let ta = Set(await box.a.terms.map(\.text)), tb = Set(await box.b.terms.map(\.text))
        XCTAssertEqual(ta, ["Parish1", "Rectory1"])
        XCTAssertEqual(tb, ["Parish1", "Rectory1"])
        let peers = await a.ledger.peers.map(\.name)
        XCTAssertEqual(peers, ["Mac"])
    }

    func testHostPort() {
        XCTAssertEqual(parseHostPort("rectory1:47823")?.host, "rectory1")
        XCTAssertEqual(parseHostPort("rectory1")?.port, 47823)
        XCTAssertEqual(parseHostPort("[::1]:9000")?.port, 9000)
        XCTAssertNil(parseHostPort(""))
    }

    func testLengthPrefix() throws {
        let framed = try SyncFrame.lengthPrefixed(Data([1, 2, 3]))
        XCTAssertEqual(Array(framed), [0, 0, 0, 3, 1, 2, 3])
        XCTAssertEqual(try SyncFrame.length(fromPrefix: framed.prefix(4)), 3)
        XCTAssertThrowsError(try SyncFrame.length(fromPrefix: Data([0x7f, 0, 0, 0])))
    }
}

actor Box {
    var a = PersonalDictionary()
    var b = PersonalDictionary()
    func set(a: PersonalDictionary, b: PersonalDictionary) {
        self.a = a
        self.b = b
    }
    func setA(_ d: PersonalDictionary) { a = d }
    func setB(_ d: PersonalDictionary) { b = d }
}

final class HandoffTests: XCTestCase {
    func testKeyboardRoundTrip() {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("silas-handoff-\(UUID())")
        defer { try? FileManager.default.removeItem(at: dir) }
        let h = Handoff(directory: dir)
        XCTAssertEqual(h.session().phase, .off)
        h.send(.start)
        XCTAssertEqual(h.request?.command, .start)
        h.setSession(.init(phase: .listening, preview: "hello"))
        XCTAssertEqual(h.session().phase, .listening)
        XCTAssertEqual(h.session(now: Date().addingTimeInterval(60)).phase, .off)
        h.publish("Hello from Silas.")
        XCTAssertEqual(h.takePending()?.text, "Hello from Silas.")
        XCTAssertNil(h.takePending(), "pending text is taken once")
        XCTAssertEqual(h.last?.text, "Hello from Silas.")
    }
}
