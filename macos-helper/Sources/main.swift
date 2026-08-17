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

final class CaptureState: @unchecked Sendable {
    private let lock = NSLock()
    private var stopRequested = false
    private var failureMessage: String?

    func requestStop() {
        lock.lock()
        stopRequested = true
        lock.unlock()
    }

    func fail(_ message: String) {
        lock.lock()
        if failureMessage == nil {
            failureMessage = message
        }
        lock.unlock()
    }

    func snapshot() -> (stopRequested: Bool, failureMessage: String?) {
        lock.lock()
        defer { lock.unlock() }
        return (stopRequested, failureMessage)
    }
}

func enforcePrivatePermissions(for url: URL, directory: Bool = false) throws {
    try FileManager.default.setAttributes(
        [.posixPermissions: directory ? 0o700 : 0o600],
        ofItemAtPath: url.path
    )
}

let minimumFreeRecordingBytes: Int64 = 256 * 1024 * 1024

func ensureRecordingSpace(at url: URL) throws {
    let values = try url.resourceValues(forKeys: [
        .volumeAvailableCapacityForImportantUsageKey,
        .volumeAvailableCapacityKey,
    ])
    let importantUsageCapacity = values.volumeAvailableCapacityForImportantUsage
        .flatMap { $0 > 0 ? $0 : nil }
    let available = importantUsageCapacity ?? values.volumeAvailableCapacity.map(Int64.init)
    guard let available else {
        throw CaptureError.writer("Could not determine available disk space.")
    }
    guard available >= minimumFreeRecordingBytes else {
        throw CaptureError.writer(
            "Only \(available) bytes are free; Rusteze keeps a \(minimumFreeRecordingBytes)-byte reserve while processing audio."
        )
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
    private let state: CaptureState
    private var inputNode: AVAudioInputNode?
    private var audioFile: AVAudioFile?

    init(state: CaptureState) {
        self.state = state
    }

    func start(outputURL: URL) throws {
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0, format.channelCount > 0 else {
            throw CaptureError.unavailableMicrophone
        }

        let audioFile = try AVAudioFile(forWriting: outputURL, settings: format.settings)
        try enforcePrivatePermissions(for: outputURL)
        self.audioFile = audioFile
        input.installTap(onBus: 0, bufferSize: 4_096, format: format) { [weak self] buffer, _ in
            do {
                try self?.audioFile?.write(from: buffer)
            } catch {
                let message = "Microphone write failed: \(error.localizedDescription)"
                self?.state.fail(message)
                fputs("\(message)\n", stderr)
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
    private let state: CaptureState
    private let queue = DispatchQueue(label: "dev.rusteze.system-audio")
    private var stream: SCStream?
    private var writer: AVAssetWriter?
    private var writerInput: AVAssetWriterInput?
    private var started = false

    init(outputURL: URL, state: CaptureState) {
        self.outputURL = outputURL
        self.state = state
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
        started = true
    }

    func stop() async {
        if let stream {
            do {
                try await stream.stopCapture()
            } catch {
                state.fail("System-audio capture could not stop cleanly: \(error.localizedDescription)")
            }
        }
        writerInput?.markAsFinished()
        if let writer {
            if writer.status == .writing {
                await withCheckedContinuation { continuation in
                    writer.finishWriting { continuation.resume() }
                }
            }
            if writer.status != .completed {
                state.fail(
                    writer.error?.localizedDescription
                        ?? "System-audio file could not be finalized."
                )
            }
        } else if started {
            state.fail("System-audio capture ended before any audio samples were written.")
        }
        stream = nil
        writer = nil
        writerInput = nil
        started = false
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .audio, CMSampleBufferDataIsReady(sampleBuffer) else { return }
        do {
            try write(sampleBuffer)
        } catch {
            let message = "System-audio write failed: \(error.localizedDescription)"
            state.fail(message)
            fputs("\(message)\n", stderr)
        }
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        let message = "System-audio capture stopped: \(error.localizedDescription)"
        state.fail(message)
        fputs("\(message)\n", stderr)
    }

    private func write(_ sampleBuffer: CMSampleBuffer) throws {
        if writer == nil {
            guard let sourceFormat = CMSampleBufferGetFormatDescription(sampleBuffer) else {
                throw CaptureError.writer("System-audio stream has no format description.")
            }
            let newWriter = try AVAssetWriter(outputURL: outputURL, fileType: .caf)
            let outputSettings: [String: Any] = [
                AVFormatIDKey: kAudioFormatLinearPCM,
                AVSampleRateKey: 48_000,
                AVNumberOfChannelsKey: 2,
                AVLinearPCMBitDepthKey: 16,
                AVLinearPCMIsBigEndianKey: false,
                AVLinearPCMIsFloatKey: false,
                AVLinearPCMIsNonInterleaved: false,
            ]
            let newInput = AVAssetWriterInput(
                mediaType: .audio,
                outputSettings: outputSettings,
                sourceFormatHint: sourceFormat
            )
            guard newWriter.canAdd(newInput) else {
                throw CaptureError.writer(
                    "Cannot add a system-audio track to the output file; supported media types: "
                        + newWriter.availableMediaTypes.map(\.rawValue).joined(separator: ", ")
                )
            }
            newWriter.add(newInput)
            guard newWriter.startWriting() else {
                throw CaptureError.writer(newWriter.error?.localizedDescription ?? "Cannot start system-audio file.")
            }
            do {
                try enforcePrivatePermissions(for: outputURL)
            } catch {
                newWriter.cancelWriting()
                throw error
            }
            newWriter.startSession(atSourceTime: CMSampleBufferGetPresentationTimeStamp(sampleBuffer))
            writer = newWriter
            writerInput = newInput
        }
        guard writerInput?.isReadyForMoreMediaData == true else {
            throw CaptureError.writer("System-audio writer could not keep up with the capture stream.")
        }
        guard writerInput?.append(sampleBuffer) == true else {
            throw CaptureError.writer(
                writer?.error?.localizedDescription ?? "System-audio sample could not be written."
            )
        }
    }
}

final class CaptureController {
    private let state: CaptureState
    private var microphone: MicrophoneRecorder?
    private var systemAudio: SystemAudioRecorder?

    init(state: CaptureState) {
        self.state = state
    }

    func start(in folder: URL, mode: CaptureMode) async throws {
        switch mode {
        case .systemOnly:
            let systemAudio = SystemAudioRecorder(
                outputURL: folder.appendingPathComponent("system.caf"),
                state: state
            )
            do {
                try await systemAudio.start()
                self.systemAudio = systemAudio
            } catch {
                await systemAudio.stop()
                throw error
            }
        case .microphoneOnly:
            let microphone = MicrophoneRecorder(state: state)
            do {
                try microphone.start(outputURL: folder.appendingPathComponent("mic.caf"))
                self.microphone = microphone
            } catch {
                microphone.stop()
                throw error
            }
        case .both:
            let microphone = MicrophoneRecorder(state: state)
            do {
                try microphone.start(outputURL: folder.appendingPathComponent("mic.caf"))
            } catch {
                microphone.stop()
                throw error
            }
            self.microphone = microphone

            let systemAudio = SystemAudioRecorder(
                outputURL: folder.appendingPathComponent("system.caf"),
                state: state
            )
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

    let state = CaptureState()
    let controller = CaptureController(state: state)
    do {
        let folder = URL(fileURLWithPath: folderPath)
        try enforcePrivatePermissions(for: folder, directory: true)
        try ensureRecordingSpace(at: folder)
        try await controller.start(in: folder, mode: mode)
        print("RESULT recording-started")
        fflush(stdout)

        DispatchQueue.global(qos: .userInitiated).async {
            while let command = readLine() {
                if command == "stop" {
                    state.requestStop()
                    return
                }
            }
            state.requestStop()
        }

        var nextSpaceCheck = Date().addingTimeInterval(5)
        while true {
            if Date() >= nextSpaceCheck {
                do {
                    try ensureRecordingSpace(at: folder)
                } catch {
                    state.fail(error.localizedDescription)
                }
                nextSpaceCheck = Date().addingTimeInterval(5)
            }
            let snapshot = state.snapshot()
            if snapshot.stopRequested || snapshot.failureMessage != nil {
                break
            }
            try? await Task.sleep(nanoseconds: 100_000_000)
        }
        await controller.stop()
        if let failure = state.snapshot().failureMessage {
            fputs("Recording failed: \(failure)\n", stderr)
            return 1
        }
        return 0
    } catch {
        fputs("Recording could not start: \(error.localizedDescription)\n", stderr)
        await controller.stop()
        return 1
    }
}

func mix(folderPath: String) async -> Int32 {
    let fileManager = FileManager.default
    let folder = URL(fileURLWithPath: folderPath, isDirectory: true)
    let inputURLs = [
        folder.appendingPathComponent("system.caf"),
        folder.appendingPathComponent("mic.caf"),
    ]
    let destination = folder.appendingPathComponent("mixed.caf")
    let temporary = folder.appendingPathComponent(".mixed.\(UUID().uuidString).tmp.caf")

    do {
        try enforcePrivatePermissions(for: folder, directory: true)
        try ensureRecordingSpace(at: folder)
        for inputURL in inputURLs {
            let attributes = try fileManager.attributesOfItem(atPath: inputURL.path)
            guard attributes[.type] as? FileAttributeType == .typeRegular else {
                throw CaptureError.writer("Refusing to mix non-regular audio file \(inputURL.path).")
            }
        }

        let files = try inputURLs.map { try AVAudioFile(forReading: $0) }
        guard let outputFormat = AVAudioFormat(
            standardFormatWithSampleRate: 48_000,
            channels: 2
        ) else {
            throw CaptureError.writer("Could not create the mixed-audio PCM format.")
        }
        let engine = AVAudioEngine()
        let players = files.map { _ in AVAudioPlayerNode() }
        for (player, file) in zip(players, files) {
            engine.attach(player)
            engine.connect(player, to: engine.mainMixerNode, format: file.processingFormat)
            player.volume = 0.5
        }
        try engine.enableManualRenderingMode(
            .offline,
            format: outputFormat,
            maximumFrameCount: 4_096
        )
        guard let buffer = AVAudioPCMBuffer(
            pcmFormat: engine.manualRenderingFormat,
            frameCapacity: engine.manualRenderingMaximumFrameCount
        ) else {
            throw CaptureError.writer("Could not allocate the mixed-audio render buffer.")
        }
        var outputFile: AVAudioFile? = try AVAudioFile(
            forWriting: temporary,
            settings: outputFormat.settings
        )
        try enforcePrivatePermissions(for: temporary)
        let outputFrames: AVAudioFramePosition = files.map { file -> AVAudioFramePosition in
            let duration = Double(file.length) / file.processingFormat.sampleRate
            return AVAudioFramePosition(ceil(duration * outputFormat.sampleRate))
        }.max() ?? 0
        for (player, file) in zip(players, files) {
            player.scheduleFile(file, at: nil, completionHandler: nil)
        }
        try engine.start()
        players.forEach { $0.play() }
        var nextSpaceCheck = Date().addingTimeInterval(5)

        while engine.manualRenderingSampleTime < outputFrames {
            if Date() >= nextSpaceCheck {
                try ensureRecordingSpace(at: folder)
                nextSpaceCheck = Date().addingTimeInterval(5)
            }
            let remaining = outputFrames - engine.manualRenderingSampleTime
            let framesToRender = AVAudioFrameCount(
                min(AVAudioFramePosition(buffer.frameCapacity), remaining)
            )
            switch try engine.renderOffline(framesToRender, to: buffer) {
            case .success:
                try outputFile?.write(from: buffer)
            case .insufficientDataFromInputNode, .cannotDoInCurrentContext:
                continue
            case .error:
                throw CaptureError.writer("AVAudioEngine could not render the mixed track.")
            @unknown default:
                throw CaptureError.writer("AVAudioEngine returned an unknown rendering status.")
            }
        }
        players.forEach { $0.stop() }
        engine.stop()
        outputFile = nil

        if fileManager.fileExists(atPath: destination.path) {
            _ = try fileManager.replaceItemAt(destination, withItemAt: temporary)
        } else {
            try fileManager.moveItem(at: temporary, to: destination)
        }
        try enforcePrivatePermissions(for: destination)
        return 0
    } catch CaptureError.writer(let reason) {
        try? fileManager.removeItem(at: temporary)
        fputs("Audio mixing failed: \(reason)\n", stderr)
        return 1
    } catch {
        try? fileManager.removeItem(at: temporary)
        let details = error as NSError
        fputs(
            "Audio mixing failed: \(error.localizedDescription) "
                + "[\(details.domain) \(details.code)] \(details.userInfo)\n",
            stderr
        )
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
        case "mix":
            guard arguments.count == 2 else {
                fputs("Usage: rusteze-capture-helper mix SESSION_FOLDER\n", stderr)
                exit(64)
            }
            exit(await mix(folderPath: arguments[1]))
        default:
            fputs("Usage: rusteze-capture-helper <check-permissions|request-permissions|record SESSION_FOLDER system|microphone|both|mix SESSION_FOLDER>\n", stderr)
            exit(64)
        }
    }
}
