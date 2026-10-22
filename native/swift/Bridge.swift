// The C interface the Rust app calls. Nothing here throws across the boundary, nothing here
// blocks the caller, and no Swift or Objective-C exception can escape: every failure becomes an
// "error" event. See src/sync/native.rs for the other side and the wire format.

import Foundation

/// Bumped with `ABI_VERSION` in src/sync/native.rs when commands or events change incompatibly.
private let abiVersion: UInt32 = 1

public typealias DVEventCallback = @convention(c) (UnsafeMutableRawPointer?, UnsafePointer<UInt8>?, Int) -> Void

@_cdecl("dv_sync_abi_version")
public func dv_sync_abi_version() -> UInt32 {
    abiVersion
}

/// Creates a coordinator. It does not touch CloudKit until it is told to `start`, so a copy of
/// the app that is not signed for iCloud can create one safely (Rust checks the signature first).
@_cdecl("dv_sync_create")
public func dv_sync_create(
    _ config: UnsafePointer<UInt8>?,
    _ length: Int,
    _ callback: DVEventCallback?,
    _ user: UnsafeMutableRawPointer?
) -> UnsafeMutableRawPointer? {
    guard let config, let callback, length > 0 else { return nil }
    let data = Data(bytes: config, count: length)
    guard
        let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
        let container = json["container"] as? String
    else { return nil }
    let coordinator = Coordinator(containerIdentifier: container, emitter: Emitter(callback: callback, user: user))
    return Unmanaged.passRetained(coordinator).toOpaque()
}

/// Queues a command. Returns at once; the work happens on the coordinator's own task, in order.
@_cdecl("dv_sync_command")
public func dv_sync_command(_ handle: UnsafeMutableRawPointer?, _ json: UnsafePointer<UInt8>?, _ length: Int) {
    guard let handle, let json, length > 0 else { return }
    let coordinator = Unmanaged<Coordinator>.fromOpaque(handle).takeUnretainedValue()
    coordinator.submit(Data(bytes: json, count: length))
}

/// Stops the coordinator. After this returns no event callback fires, so Rust may free its side.
@_cdecl("dv_sync_destroy")
public func dv_sync_destroy(_ handle: UnsafeMutableRawPointer?) {
    guard let handle else { return }
    let coordinator = Unmanaged<Coordinator>.fromOpaque(handle).takeRetainedValue()
    coordinator.shutdown()
}
