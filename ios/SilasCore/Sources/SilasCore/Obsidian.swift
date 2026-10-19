import Foundation

public enum Obsidian {
    /// `obsidian://new?vault=BaneyVault&file=0_Inbox/Dictation%20yyyy-MM-dd%20HH.mm&content=…`
    public static func newNoteURL(
        text: String,
        date: Date,
        vault: String = "BaneyVault",
        folder: String = "0_Inbox",
        timeZone: TimeZone = .current
    ) -> URL? {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.timeZone = timeZone
        f.dateFormat = "yyyy-MM-dd HH.mm"
        let file = "\(folder)/Dictation \(f.string(from: date))"
        let filePath = file.split(separator: "/", omittingEmptySubsequences: false)
            .map { encode(String($0)) }
            .joined(separator: "/")
        return URL(string: "obsidian://new?vault=\(encode(vault))&file=\(filePath)&content=\(encode(text))")
    }

    /// Percent-encode everything but RFC 3986 unreserved characters.
    static func encode(_ s: String) -> String {
        var allowed = CharacterSet()
        allowed.insert(charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
        return s.addingPercentEncoding(withAllowedCharacters: allowed) ?? s
    }
}
