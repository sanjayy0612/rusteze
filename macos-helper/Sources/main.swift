import AVFoundation
import CoreGraphics
import CoreMedia
import Foundation
import ScreenCaptureKit

enum MicrophonePermission: String {
    case granted, denied, restricted
    case notDetermined = "not_determined"
}

enum CaptureMode: String {
    case systemOnly = "system"
    case microphoneOnly = "microphone"
    case both

    var requiresMicrophone: Bool {
        self == .microphoneOnly || self == .both
    }

    var requiresScreenRecording: Bool {
        self == .systemOnly || self == .both
    }
}

func microphonePermission() -> MicrophonePermission {
    switch AVCaptureDevice.authorizationStatus(for: .audio) {
    case .authorized: return .granted
    case .denied: return .denied
    case .restricted: return .restricted
    case .notDetermined: return .notDetermined
    @unknown default: return .denied
    }
}

func printPermissionStatus() {
    let screenRecording = CGPreflightScreenCaptureAccess() ? "granted" : "missing"
    print("RESULT permission-status")
    print("MICROPHONE \(microphonePermission().rawValue)")
    print("SCREEN_RECORDING \(screenRecording)")
}

func requestPermissions(for mode: CaptureMode) async {
    if mode.requiresMicrophone, microphonePermission() == .notDetermined {
        _ = await withCheckedContinuation { continuation in
            AVCaptureDevice.requestAccess(for: .audio) { continuation.resume(returning: $0) }
        }
    }
    if mode.requiresScreenRecording, !CGPreflightScreenCaptureAccess() {
        _ = CGRequestScreenCaptureAccess()
    }
    printPermissionStatus()
}

func missingPermissions(for mode: CaptureMode) -> [String] {
    var missing = [String]()
    if mode.requiresMicrophone, microphonePermission() != .granted {
        missing.append("Microphone")
    }
    if mode.requiresScreenRecording, !CGPreflightScreenCaptureAccess() {
        missing.append("Screen Recording")
    }
    return missing
}

enum CaptureError: LocalizedError {
    case unavailableMicrophone
    case noDisplay
    case writer(String)

    var errorDescription: String? {
        switch self {
        case .unavailableMicrophone: return "No microphone input is available."
        case .noDisplay: return "No display is available for system-audio capture."
        case .writer(let reason): return reason
        }
    }
}

final class MicrophoneRecorder {
    private let engine = AVAudioEngine()
    private var inputNode: AVAudioInputNode?
    private var audioFile: AVAudioFile?

    func start(outputURL: URL) throws {
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            throw CaptureError.unavailableMicrophone
        }

        audioFile = try AVAudioFile(forWriting: outputURL, settings: format.settings)
        input.installTap(onBus: 0, bufferSize: 4_096, format: format) { [weak self] buffer, _ in
            do {
                try self?.audioFile?.write(from: buffer)
            } catch {
                fputs("Microphone write failed: \(error.localizedDescription)\n", stderr)
            }
        }
        inputNode = input
        try engine.start()
    }

    func stop() {
        inputNode?.removeTap(onBus: 0)
        engine.stop()
        audioFile = nil
    }
}

final class SystemAudioRecorder: NSObject, SCStreamOutput, SCStreamDelegate {
    private let outputURL: URL
    private let queue = DispatchQueue(label: "dev.rusteze.system-audio")
    private var stream: SCStream?
    private var writer: AVAssetWriter?
    private var writerInput: AVAssetWriterInput?

    init(outputURL: URL) {
        self.outputURL = outputURL
    }

    func start() async throws {
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        guard let display = content.displays.first else { throw CaptureError.noDisplay }

        let configuration = SCStreamConfiguration()
        configuration.capturesAudio = true
        configuration.excludesCurrentProcessAudio = true
        configuration.sampleRate = 48_000
        configuration.channelCount = 2
        configuration.queueDepth = 5

        let filter = SCContentFilter(display: display, excludingApplications: [], exceptingWindows: [])
        let stream = SCStream(filter: filter, configuration: configuration, delegate: self)
        try stream.addStreamOutput(self, type: .audio, sampleHandlerQueue: queue)
        self.stream = stream
        try await stream.startCapture()
    }

    func stop() async {
        try? await stream?.stopCapture()
        writerInput?.markAsFinished()
        if let writer {
            await withCheckedContinuation { continuation in
                writer.finishWriting { continuation.resume() }
            }
        }
        stream = nil
        writer = nil
        writerInput = nil
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .audio, CMSampleBufferDataIsReady(sampleBuffer) else { return }
        do {
            try write(sampleBuffer)
        } catch {
            fputs("System-audio write failed: \(error.localizedDescription)\n", stderr)
        }
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        fputs("System-audio capture stopped: \(error.localizedDescription)\n", stderr)
    }

    private func write(_ sampleBuffer: CMSampleBuffer) throws {
        if writer == nil {
            guard let sourceFormat = CMSampleBufferGetFormatDescription(sampleBuffer) else {
                throw CaptureError.writer("System-audio stream has no format description.")
            }
            let newWriter = try AVAssetWriter(outputURL: outputURL, fileType: .caf)
            let newInput = AVAssetWriterInput(mediaType: .audio, outputSettings: nil, sourceFormatHint: sourceFormat)
            guard newWriter.canAdd(newInput) else {
                throw CaptureError.writer("Cannot add a system-audio track to the output file.")
            }
            newWriter.add(newInput)
            guard newWriter.startWriting() else {
                throw CaptureError.writer(newWriter.error?.localizedDescription ?? "Cannot start system-audio file.")
            }
            newWriter.startSession(atSourceTime: CMSampleBufferGetPresentationTimeStamp(sampleBuffer))
            writer = newWriter
            writerInput = newInput
        }
        if writerInput?.isReadyForMoreMediaData == true {
            _ = writerInput?.append(sampleBuffer)
        }
    }
}

final class CaptureController {
    private var microphone: MicrophoneRecorder?
    private var systemAudio: SystemAudioRecorder?

    func start(in folder: URL, mode: CaptureMode) async throws {
        switch mode {
        case .systemOnly:
            let systemAudio = SystemAudioRecorder(outputURL: folder.appendingPathComponent("system.caf"))
            do {
                try await systemAudio.start()
                self.systemAudio = systemAudio
            } catch {
                await systemAudio.stop()
                throw error
            }
        case .microphoneOnly:
            let microphone = MicrophoneRecorder()
            do {
                try microphone.start(outputURL: folder.appendingPathComponent("mic.caf"))
                self.microphone = microphone
            } catch {
                microphone.stop()
                throw error
            }
        case .both:
            let microphone = MicrophoneRecorder()
            do {
                try microphone.start(outputURL: folder.appendingPathComponent("mic.caf"))
            } catch {
                microphone.stop()
                throw error
            }
            self.microphone = microphone

            let systemAudio = SystemAudioRecorder(outputURL: folder.appendingPathComponent("system.caf"))
            do {
                try await systemAudio.start()
                self.systemAudio = systemAudio
            } catch {
                await systemAudio.stop()
                self.microphone?.stop()
                self.microphone = nil
                throw error
            }
        }
    }

    func stop() async {
        await systemAudio?.stop()
        microphone?.stop()
        systemAudio = nil
        microphone = nil
    }
}

func record(folderPath: String, modeValue: String) async -> Int32 {
    guard let mode = CaptureMode(rawValue: modeValue) else {
        fputs("Invalid capture mode '\(modeValue)'. Expected system, microphone, or both.\n", stderr)
        return 64
    }

    let missing = missingPermissions(for: mode)
    guard missing.isEmpty else {
        printPermissionStatus()
        fputs("Missing permission for \(mode.rawValue) capture: \(missing.joined(separator: " and ")).\n", stderr)
        return 77
    }

    let controller = CaptureController()
    do {
        try await controller.start(in: URL(fileURLWithPath: folderPath), mode: mode)
        print("RESULT recording-started")
        fflush(stdout)
        while let command = readLine() {
            if command == "stop" { break }
        }
        await controller.stop()
        print("RESULT recording-stopped")
        return 0
    } catch {
        fputs("Recording could not start: \(error.localizedDescription)\n", stderr)
        await controller.stop()
        return 1
    }
}

@main
struct RustezeCaptureHelper {
    static func main() async {
        let arguments = Array(CommandLine.arguments.dropFirst())
        switch arguments.first {
        case "check-permissions":
            guard arguments.count == 1 || arguments.count == 2 else {
                fputs("Usage: rusteze-capture-helper check-permissions [system|microphone|both]\n", stderr)
                exit(64)
            }
            if arguments.count == 2, CaptureMode(rawValue: arguments[1]) == nil {
                fputs("Invalid capture mode '\(arguments[1])'. Expected system, microphone, or both.\n", stderr)
                exit(64)
            }
            printPermissionStatus()
        case "request-permissions":
            guard arguments.count == 1 || arguments.count == 2 else {
                fputs("Usage: rusteze-capture-helper request-permissions [system|microphone|both]\n", stderr)
                exit(64)
            }
            let mode = arguments.count == 2 ? CaptureMode(rawValue: arguments[1]) : .both
            guard let mode else {
                fputs("Invalid capture mode '\(arguments[1])'. Expected system, microphone, or both.\n", stderr)
                exit(64)
            }
            await requestPermissions(for: mode)
        case "record":
            guard arguments.count == 3 else {
                fputs("Usage: rusteze-capture-helper record SESSION_FOLDER system|microphone|both\n", stderr)
                exit(64)
            }
            exit(await record(folderPath: arguments[1], modeValue: arguments[2]))
        default:
            fputs("Usage: rusteze-capture-helper <check-permissions|request-permissions|record SESSION_FOLDER system|microphone|both>\n", stderr)
            exit(64)
        }
    }
}
