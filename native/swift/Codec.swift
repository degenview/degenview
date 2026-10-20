// Turning events into JSON for Rust, and CloudKit errors into the kinds Rust understands.

import CloudKit
import CryptoKit
import Foundation

/// Delivers events to Rust, one at a time and in the order they happened. After `close()` no
/// further callback is made.
final class Emitter: @unchecked Sendable {
    private let callback: DVEventCallback
    private let user: UnsafeMutableRawPointer?
    private let queue = DispatchQueue(label: "es.nico.degenview.sync.events")
    private var closed = false

    init(callback: @escaping DVEventCallback, user: UnsafeMutableRawPointer?) {
        self.callback = callback
        self.user = user
    }

    func emit(_ event: [String: Any]) {
        guard JSONSerialization.isValidJSONObject(event),
            let data = try? JSONSerialization.data(withJSONObject: event)
        else { return }
        queue.async { [self] in
            if closed { return }
            data.withUnsafeBytes { buffer in
                callback(user, buffer.bindMemory(to: UInt8.self).baseAddress, buffer.count)
            }
        }
    }

    /// Waits for in-flight deliveries, then stops delivering.
    func close() {
        queue.sync { closed = true }
    }
}

enum Codec {
    /// What the app keeps of an iCloud account: a hash, never the identifier. Must match
    /// `account::hash_user` in Rust.
    static func hashUser(container: String, recordName: String) -> String {
        let digest = SHA256.hash(data: Data("v1|\(container)|\(recordName)".utf8))
        return digest.map { String(format: "%02x", $0) }.joined()
    }

    static func base64(_ data: Data?) -> Any {
        data.map { $0.base64EncodedString() } ?? NSNull()
    }

    /// The part of a record CloudKit needs to save over it later (its change tag).
    static func systemFields(of record: CKRecord) -> Data {
        let coder = NSKeyedArchiver(requiringSecureCoding: true)
        record.encodeSystemFields(with: coder)
        coder.finishEncoding()
        return coder.encodedData
    }

    static func record(fromSystemFields data: Data) -> CKRecord? {
        guard let coder = try? NSKeyedUnarchiver(forReadingFrom: data) else { return nil }
        coder.requiresSecureCoding = true
        defer { coder.finishDecoding() }
        return CKRecord(coder: coder)
    }

    /// A record as Rust expects it in `fetched` events and `server_record` fields.
    static func json(_ record: CKRecord) -> [String: Any] {
        let encrypted = record.encryptedValues
        let payload = encrypted["payload"] as? Data ?? Data()
        return [
            "name": record.recordID.recordName,
            "schema_version": (record["schemaVersion"] as? Int64).map(Int.init) ?? 0,
            "revision": (record["revision"] as? Int64).map(Int.init) ?? 0,
            "origin_device": record["originDevice"] as? String ?? "",
            "modified_ms": (encrypted["modifiedAtMs"] as? Int64).map(Int.init) ?? 0,
            "payload": payload.base64EncodedString(),
            "system_fields": systemFields(of: record).base64EncodedString(),
        ]
    }

    /// The kind Rust classifies a failure by.
    static func kind(of error: Error) -> String {
        guard let error = error as? CKError else { return "unknown" }
        switch error.code {
        case .networkUnavailable, .networkFailure, .serviceUnavailable, .zoneBusy, .requestRateLimited:
            return error.code == .requestRateLimited ? "rate_limited" : "network"
        case .notAuthenticated, .accountTemporarilyUnavailable:
            return "account_unavailable"
        case .quotaExceeded: return "quota"
        case .zoneNotFound, .userDeletedZone: return "zone_not_found"
        case .serverRecordChanged: return "server_record_changed"
        case .unknownItem: return "unknown_item"
        case .changeTokenExpired: return "change_token_expired"
        case .limitExceeded, .batchRequestFailed: return "too_large"
        case .permissionFailure: return "permission"
        default: return "unknown"
        }
    }

    static func failure(seq: Int, recordID: CKRecord.ID, error: CKError) -> [String: Any] {
        var failure: [String: Any] = [
            "seq": seq,
            "name": recordID.recordName,
            "kind": kind(of: error),
            "message": error.localizedDescription,
        ]
        if let retry = error.retryAfterSeconds {
            failure["retry_after_s"] = Int(retry.rounded(.up))
        }
        if let server = error.serverRecord {
            failure["server_record"] = json(server)
        }
        return failure
    }
}
