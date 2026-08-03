import AVFoundation
import CoreGraphics
import CoreMedia
import Foundation
import ScreenCaptureKit

enum MicrophonePermission: String {
    case granted, denied, restricted
    case notDetermined = "not_determined"
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

func requestPermissions() async {
    if microphonePermission() == .notDetermined {
        _ = await withCheckedContinuation { continuation in
            AVCaptureDevice.requestAccess(for: .audio) { continuation.resume(returning: $0) }
        }
    }
    if !CGPreflightScreenCaptureAccess() {
        _ = CGRequestScreenCaptureAccess()
    }
    printPermissionStatus()
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
        try await stream.startCapture()
        self.stream = stream
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
    private let microphone = MicrophoneRecorder()
    private var systemAudio: SystemAudioRecorder?

    func start(in folder: URL) async throws {
        try microphone.start(outputURL: folder.appendingPathComponent("mic.caf"))
        let systemAudio = SystemAudioRecorder(outputURL: folder.appendingPathComponent("system.caf"))
        do {
            try await systemAudio.start()
            self.systemAudio = systemAudio
        } catch {
            microphone.stop()
            throw error
        }
    }

    func stop() async {
        await systemAudio?.stop()
        microphone.stop()
    }
}

func record(folderPath: String) async -> Int32 {
    guard CGPreflightScreenCaptureAccess(), microphonePermission() == .granted else {
        printPermissionStatus()
        return 77
    }

    let controller = CaptureController()
    do {
        try await controller.start(in: URL(fileURLWithPath: folderPath))
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
        case "check-permissions": printPermissionStatus()
        case "request-permissions": await requestPermissions()
        case "record" where arguments.count == 2: exit(await record(folderPath: arguments[1]))
        default:
            fputs("Usage: rusteze-capture-helper <check-permissions|request-permissions|record SESSION_FOLDER>\n", stderr)
            exit(64)
        }
    }
}
