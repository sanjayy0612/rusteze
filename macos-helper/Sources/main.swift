import AVFoundation
import CoreGraphics
import Foundation

enum MicrophonePermission: String {
    case granted
    case denied
    case restricted
    case notDetermined = "not_determined"
}

func microphonePermission() -> MicrophonePermission {
    switch AVCaptureDevice.authorizationStatus(for: .audio) {
    case .authorized:
        return .granted
    case .denied:
        return .denied
    case .restricted:
        return .restricted
    case .notDetermined:
        return .notDetermined
    @unknown default:
        return .denied
    }
}

func printPermissionStatus() {
    // ScreenCaptureKit relies on the same Screen Recording privacy grant that
    // CoreGraphics can preflight without beginning a capture stream.
    let screenRecordingPermission = CGPreflightScreenCaptureAccess() ? "granted" : "missing"

    // This deliberately small line protocol is the contract with Rust. It is
    // easy to inspect in a terminal and avoids tying either side to a JSON SDK.
    print("RESULT permission-status")
    print("MICROPHONE \(microphonePermission().rawValue)")
    print("SCREEN_RECORDING \(screenRecordingPermission)")
}

switch CommandLine.arguments.dropFirst().first {
case "check-permissions":
    printPermissionStatus()
case .none:
    fputs("Usage: rusteze-capture-helper check-permissions\n", stderr)
    exit(64)
default:
    fputs("Unknown helper command. Use: check-permissions\n", stderr)
    exit(64)
}
