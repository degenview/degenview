// Owns the CKSyncEngine for one database. Rust owns every decision about what to send, how to
// merge and what to apply; this file only moves records between CloudKit and Rust. It keeps no
// copy of the user's data beyond what is staged for the next send.

import AppKit
import CloudKit
import Foundation
import Network

/// One change Rust has handed over, waiting for the engine to send it.
private struct Staged {
    let seq: Int
    let op: String
    let recordName: String
    let entityType: String
    let schemaVersion: Int
    let revision: Int
    let originDevice: String
    let modifiedMs: Int
    let payload: Data
    let systemFields: Data?
}

final class Coordinator: NSObject, CKSyncEngineDelegate, @unchecked Sendable {
    private let containerIdentifier: String
    private let emitter: Emitter
    private let recordType = "DegenViewSyncRecord"

    private let lock = NSLock()
    private var staged: [CKRecord.ID: Staged] = [:]
    private var container: CKContainer?
    private var engine: CKSyncEngine?
    private var zoneID = CKRecordZone.ID(zoneName: "DegenViewZone", ownerName: CKCurrentUserDefaultName)

    private let commands: AsyncStream<Data>
    private let commandSink: AsyncStream<Data>.Continuation
    private var worker: Task<Void, Never>?
    private let monitor = NWPathMonitor()
    private var online: Bool?

    init(containerIdentifier: String, emitter: Emitter) {
        self.containerIdentifier = containerIdentifier
        self.emitter = emitter
        var continuation: AsyncStream<Data>.Continuation!
        self.commands = AsyncStream { continuation = $0 }
        self.commandSink = continuation
        super.init()
        worker = Task { [weak self] in
            guard let commands = self?.commands else { return }
            for await command in commands {
                guard let self else { return }
                await self.run(command)
            }
        }
        monitor.pathUpdateHandler = { [weak self] path in
            guard let self else { return }
            let available = path.status == .satisfied
            if self.online != available {
                self.online = available
                self.emitter.emit(["event": "network", "available": available])
            }
        }
        monitor.start(queue: DispatchQueue(label: "es.nico.degenview.sync.network"))
    }

    func submit(_ command: Data) {
        commandSink.yield(command)
    }

    func shutdown() {
        monitor.cancel()
        commandSink.finish()
        worker?.cancel()
        lock.withLock {
            engine = nil
            container = nil
            staged.removeAll()
        }
        emitter.close()
    }

    // MARK: Commands

    private func run(_ data: Data) async {
        guard let command = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
            let name = command["cmd"] as? String
        else {
            return fail("unknown", "unreadable command")
        }
        switch name {
        case "start": await start(command)
        case "account_check": await accountCheck()
        case "ensure_zone": ensureZone()
        case "stage": await stage(command)
        case "send": await send()
        case "fetch": await fetch()
        case "delete_zone": await deleteZone()
        case "stop": stop()
        default: fail("unknown", "unknown command \(name)")
        }
    }

    private func fail(_ kind: String, _ message: String) {
        emitter.emit(["event": "error", "kind": kind, "message": message])
    }

    private func start(_ command: [String: Any]) async {
        let zoneName = command["zone"] as? String ?? "DegenViewZone"
        let identifier = command["container"] as? String ?? containerIdentifier
        let saved = (command["engine_state"] as? String).flatMap { Data(base64Encoded: $0) }
        let state = saved.flatMap { try? JSONDecoder().decode(CKSyncEngine.State.Serialization.self, from: $0) }

        let container = CKContainer(identifier: identifier)
        var configuration = CKSyncEngine.Configuration(
            database: container.privateCloudDatabase,
            stateSerialization: state,
            delegate: self
        )
        // The core decides when to send (the outbox) and when to fetch (pushes, Sync Now and a short
        // poll). With automatic syncing the engine hands a background app's fetch to the system's
        // scheduler, which may run it minutes later.
        configuration.automaticallySync = false
        lock.withLock {
            self.container = container
            zoneID = CKRecordZone.ID(zoneName: zoneName, ownerName: CKCurrentUserDefaultName)
            staged.removeAll()
            engine = CKSyncEngine(configuration)
        }
        registerForPushes()
        emitter.emit(["event": "ready"])
    }

    /// CloudKit wakes the engine with a silent push when another Mac changes something, but only
    /// for an app that has registered with Apple's push service. Without this the engine still
    /// works, and changes arrive only on the periodic poll and on Sync Now.
    private func registerForPushes() {
        DispatchQueue.main.async {
            NSApplication.shared.registerForRemoteNotifications()
            DispatchQueue.main.asyncAfter(deadline: .now() + 5) {
                NSLog(
                    "DegenView sync: registered for remote notifications = %@",
                    NSApplication.shared.isRegisteredForRemoteNotifications ? "yes" : "no")
            }
        }
    }

    private func currentAccount() async -> (status: CKAccountStatus, hash: String?) {
        guard let container = lockedContainer() else { return (.couldNotDetermine, nil) }
        do {
            let status = try await container.accountStatus()
            guard status == .available else { return (status, nil) }
            let user = try await container.userRecordID()
            return (status, Codec.hashUser(container: containerIdentifier, recordName: user.recordName))
        } catch {
            fail(Codec.kind(of: error), error.localizedDescription)
            return (.couldNotDetermine, nil)
        }
    }

    private func accountCheck() async {
        let (status, hash) = await currentAccount()
        switch status {
        case .available:
            if let hash { emitter.emit(["event": "account", "status": "available", "user_hash": hash]) }
        case .noAccount: emitter.emit(["event": "account", "status": "no_account"])
        case .restricted: emitter.emit(["event": "account", "status": "restricted"])
        case .temporarilyUnavailable: emitter.emit(["event": "account", "status": "temporarily_unavailable"])
        default: emitter.emit(["event": "account", "status": "could_not_determine"])
        }
    }

    private func ensureZone() {
        guard let engine = lockedEngine() else { return }
        engine.state.add(pendingDatabaseChanges: [.saveZone(CKRecordZone(zoneID: zoneID))])
    }

    /// Hands changes to the engine, but only if the account signed in right now is the one this
    /// data is bound to, so a switch of Apple Account can never carry data across.
    private func stage(_ command: [String: Any]) async {
        guard let engine = lockedEngine(),
            let expected = command["account_hash"] as? String,
            let changes = command["changes"] as? [[String: Any]]
        else { return }
        let (_, hash) = await currentAccount()
        guard hash == expected else {
            return fail("account_unavailable", "a different iCloud account is signed in")
        }
        var saves: [CKSyncEngine.PendingRecordZoneChange] = []
        var removals: [CKSyncEngine.PendingRecordZoneChange] = []
        for change in changes {
            guard let name = change["record_name"] as? String, let seq = change["seq"] as? Int,
                let op = change["op"] as? String
            else { continue }
            let id = CKRecord.ID(recordName: name, zoneID: zoneID)
            removals.append(.saveRecord(id))
            removals.append(.deleteRecord(id))
            let entry = Staged(
                seq: seq,
                op: op,
                recordName: name,
                entityType: change["entity_type"] as? String ?? "",
                schemaVersion: change["schema_version"] as? Int ?? 1,
                revision: change["revision"] as? Int ?? 1,
                originDevice: change["origin_device"] as? String ?? "",
                modifiedMs: change["modified_ms"] as? Int ?? 0,
                payload: (change["payload"] as? String).flatMap { Data(base64Encoded: $0) } ?? Data(),
                systemFields: (change["system_fields"] as? String).flatMap { Data(base64Encoded: $0) }
            )
            lock.withLock { staged[id] = entry }
            saves.append(op == "delete" ? .deleteRecord(id) : .saveRecord(id))
        }
        // Staging again replaces what was staged before for the same record.
        engine.state.remove(pendingRecordZoneChanges: removals)
        engine.state.add(pendingRecordZoneChanges: saves)
    }

    private func send() async {
        guard let engine = lockedEngine() else { return }
        do { try await engine.sendChanges() } catch { fail(Codec.kind(of: error), error.localizedDescription) }
    }

    private func fetch() async {
        guard let engine = lockedEngine() else { return }
        // A user-initiated group: the engine's own scheduling treats a background app's fetches as
        // discretionary and can skip the network for minutes.
        let group = CKOperationGroup()
        group.name = "DegenView fetch"
        group.defaultConfiguration.qualityOfService = .userInitiated
        do {
            try await engine.fetchChanges(CKSyncEngine.FetchChangesOptions(scope: .all, operationGroup: group))
        } catch { fail(Codec.kind(of: error), error.localizedDescription) }
    }

    private func deleteZone() async {
        guard let engine = lockedEngine() else { return }
        engine.state.add(pendingDatabaseChanges: [.deleteZone(zoneID)])
        do {
            try await engine.sendChanges()
        } catch {
            // Withdraw the request so the engine cannot carry it out later, unannounced.
            engine.state.remove(pendingDatabaseChanges: [.deleteZone(zoneID)])
            fail(Codec.kind(of: error), error.localizedDescription)
        }
    }

    private func stop() {
        lock.withLock {
            engine = nil
            staged.removeAll()
        }
    }

    private func lockedEngine() -> CKSyncEngine? {
        lock.withLock { engine }
    }

    private func lockedContainer() -> CKContainer? {
        lock.withLock { container }
    }

    // MARK: CKSyncEngineDelegate

    func handleEvent(_ event: CKSyncEngine.Event, syncEngine: CKSyncEngine) async {
        switch event {
        case .stateUpdate(let update):
            if let data = try? JSONEncoder().encode(update.stateSerialization) {
                emitter.emit(["event": "state", "data": data.base64EncodedString()])
            }
        case .accountChange(let change):
            // The engine forgets what it had pending on any account change.
            lock.withLock { staged.removeAll() }
            switch change.changeType {
            case .signIn(let user):
                emitter.emit(["event": "account_change", "type": "sign_in", "user_hash": hash(user)])
            case .signOut:
                emitter.emit(["event": "account_change", "type": "sign_out"])
            case .switchAccounts(_, let user):
                emitter.emit(["event": "account_change", "type": "switch", "user_hash": hash(user)])
            @unknown default:
                // An account change this version does not understand: treat it as a sign-out so
                // nothing is sent until the account is checked again.
                emitter.emit(["event": "account_change", "type": "sign_out"])
            }
        case .fetchedDatabaseChanges(let changes):
            for deletion in changes.deletions where deletion.zoneID == zoneID {
                let kind: String
                switch deletion.reason {
                case .purged: kind = "purged"
                case .encryptedDataReset: kind = "encrypted_data_reset"
                default: kind = "deleted"
                }
                emitter.emit(["event": "zone", "kind": kind])
            }
        case .fetchedRecordZoneChanges(let changes):
            let records = changes.modifications.map { Codec.json($0.record) }
            let deletions = changes.deletions.map { $0.recordID.recordName }
            if !records.isEmpty || !deletions.isEmpty {
                emitter.emit(["event": "fetched", "records": records, "deletions": deletions])
            }
        case .didFetchChanges:
            emitter.emit(["event": "fetch_complete"])
        case .sentRecordZoneChanges(let sent):
            sentRecordZoneChanges(sent, engine: syncEngine)
        case .sentDatabaseChanges(let sent):
            for failure in sent.failedZoneSaves {
                fail(Codec.kind(of: failure.error), failure.error.localizedDescription)
            }
            // The deletion this Mac asked for (see `deleteZone`) went through.
            if sent.deletedZoneIDs.contains(zoneID) {
                emitter.emit(["event": "zone_deleted"])
            }
            for (_, error) in sent.failedZoneDeletes {
                fail(Codec.kind(of: error), error.localizedDescription)
            }
        default:
            break
        }
    }

    private func hash(_ user: CKRecord.ID) -> String {
        Codec.hashUser(container: containerIdentifier, recordName: user.recordName)
    }

    private func sentRecordZoneChanges(_ sent: CKSyncEngine.Event.SentRecordZoneChanges, engine: CKSyncEngine) {
        var saved: [[String: Any]] = []
        var deleted: [[String: Any]] = []
        var failed: [[String: Any]] = []
        var finished: [CKSyncEngine.PendingRecordZoneChange] = []

        lock.withLock {
            for record in sent.savedRecords {
                if let entry = staged.removeValue(forKey: record.recordID) {
                    saved.append([
                        "seq": entry.seq, "name": record.recordID.recordName,
                        "system_fields": Codec.systemFields(of: record).base64EncodedString(),
                    ])
                }
            }
            for id in sent.deletedRecordIDs {
                if let entry = staged.removeValue(forKey: id) {
                    deleted.append(["seq": entry.seq, "name": id.recordName])
                }
            }
            for failure in sent.failedRecordSaves {
                if let entry = staged.removeValue(forKey: failure.record.recordID) {
                    failed.append(Codec.failure(seq: entry.seq, recordID: failure.record.recordID, error: failure.error))
                    finished.append(.saveRecord(failure.record.recordID))
                }
            }
            for (id, error) in sent.failedRecordDeletes {
                if let entry = staged.removeValue(forKey: id) {
                    failed.append(Codec.failure(seq: entry.seq, recordID: id, error: error))
                    finished.append(.deleteRecord(id))
                }
            }
        }

        // Rust's outbox decides what to try again; drop what the engine would otherwise retry on
        // its own with a stale payload.
        if !finished.isEmpty { engine.state.remove(pendingRecordZoneChanges: finished) }
        if !saved.isEmpty || !deleted.isEmpty || !failed.isEmpty {
            emitter.emit(["event": "sent", "saved": saved, "deleted": deleted, "failed": failed])
        }
    }

    func nextRecordZoneChangeBatch(
        _ context: CKSyncEngine.SendChangesContext,
        syncEngine: CKSyncEngine
    ) async -> CKSyncEngine.RecordZoneChangeBatch? {
        let scope = context.options.scope
        let pending = syncEngine.state.pendingRecordZoneChanges.filter { scope.contains($0) }
        return await CKSyncEngine.RecordZoneChangeBatch(pendingChanges: pending) { [self] id in
            let entry = lock.withLock { staged[id] }
            guard let entry, entry.op == "save" else { return nil }
            return makeRecord(id: id, entry: entry)
        }
    }

    private func makeRecord(id: CKRecord.ID, entry: Staged) -> CKRecord {
        let record = entry.systemFields.flatMap(Codec.record(fromSystemFields:))
            ?? CKRecord(recordType: recordType, recordID: id)
        // What Apple can see: enough to triage and gate on schema, nothing about the content.
        record["schemaVersion"] = Int64(entry.schemaVersion)
        record["entityType"] = entry.entityType
        record["revision"] = Int64(entry.revision)
        record["originDevice"] = entry.originDevice
        record["payloadSize"] = Int64(entry.payload.count)
        // The content is stored in CloudKit's encrypted fields.
        record.encryptedValues["payload"] = entry.payload
        record.encryptedValues["modifiedAtMs"] = Int64(entry.modifiedMs)
        return record
    }
}
