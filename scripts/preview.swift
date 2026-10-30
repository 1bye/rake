// macOS 14+ ScreenCaptureKit → VideoToolbox → bounded binary preview packets.
import Foundation
import AppKit
import ScreenCaptureKit
import CoreImage
import CoreMedia
import VideoToolbox

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}

// CVPixelBuffer is retained read-only across the capture/encoder callback queues.
final class Surface: @unchecked Sendable {
    let image: CVPixelBuffer
    init(_ image: CVPixelBuffer) { self.image = image }
}

@available(macOS 14.0, *)
// Mutable capture state is confined to queue; only encoded output uses writer/capacity.
final class Frames: NSObject, SCStreamOutput, SCStreamDelegate, @unchecked Sendable {
    let queue = DispatchQueue(label: "rake.preview.capture", qos: .userInteractive)
    private let writer = NSLock()
    private let capacity = DispatchSemaphore(value: 2)
    private let codec: String
    private let width: Int
    private let height: Int
    private var session: VTCompressionSession?
    private var last: CVPixelBuffer?
    private var sequence = 0
    private var timestamp: Int64 = 0
    private let context = CIContext(options: [.cacheIntermediates: false])
    private let color = CGColorSpace(name: CGColorSpace.sRGB)!

    init(codec: String, width: Int, height: Int) {
        self.codec = codec
        self.width = width
        self.height = height
        super.init()
        if codec == "h264" {
            var created: VTCompressionSession?
            var spec: [CFString: Any] = [
                kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder: true,
                kVTVideoEncoderSpecification_EnableLowLatencyRateControl: true,
            ]
            func create() -> OSStatus {
                VTCompressionSessionCreate(allocator: nil, width: Int32(width), height: Int32(height),
                    codecType: kCMVideoCodecType_H264, encoderSpecification: spec as CFDictionary,
                    imageBufferAttributes: nil, compressedDataAllocator: nil, outputCallback: nil,
                    refcon: nil, compressionSessionOut: &created)
            }
            if create() != noErr || created == nil {
                spec.removeValue(forKey: kVTVideoEncoderSpecification_EnableLowLatencyRateControl)
                guard create() == noErr else { fail("Hardware H.264 encoder unavailable; use JPEG fallback") }
            }
            guard let created else { fail("Hardware H.264 encoder unavailable") }
            let bitrate = min(20_000_000, max(4_000_000, width * height * 7))
            let properties: [CFString: Any] = [
                kVTCompressionPropertyKey_RealTime: true,
                kVTCompressionPropertyKey_AllowFrameReordering: false,
                kVTCompressionPropertyKey_ProfileLevel: kVTProfileLevel_H264_High_AutoLevel,
                kVTCompressionPropertyKey_ExpectedFrameRate: 60,
                kVTCompressionPropertyKey_AverageBitRate: bitrate,
                kVTCompressionPropertyKey_MaxKeyFrameInterval: 120,
            ]
            for (key, value) in properties {
                guard VTSessionSetProperty(created, key: key, value: value as CFTypeRef) == noErr else {
                    fail("H.264 configuration failed; use JPEG fallback")
                }
            }
            // Some encoders don't support this optional extra bound.
            VTSessionSetProperty(created, key: kVTCompressionPropertyKey_MaxFrameDelayCount, value: 1 as CFNumber)
            guard VTCompressionSessionPrepareToEncodeFrames(created) == noErr else { fail("H.264 encoder could not start") }
            session = created
        }
    }

    deinit { if let session { VTCompressionSessionInvalidate(session) } }

    private func packet(_ metadata: [String: Any], _ payload: Data) {
        guard payload.count <= 2 * 1024 * 1024 else { fail("Encoded preview packet exceeds 2 MiB") }
        sequence += 1
        var metadata = metadata
        metadata["sequence"] = sequence
        metadata["width"] = width
        metadata["height"] = height
        guard let header = try? JSONSerialization.data(withJSONObject: metadata) else { fail("Preview metadata encoding failed") }
        var h = UInt32(header.count).bigEndian
        var n = UInt32(payload.count).bigEndian
        var bytes = Data(bytes: &h, count: 4)
        bytes.append(Data(bytes: &n, count: 4))
        bytes.append(header)
        bytes.append(payload)
        do { try FileHandle.standardOutput.write(contentsOf: bytes) } catch { exit(0) }
    }

    private func encoded(_ sample: CMSampleBuffer) {
        guard let block = sample.dataBuffer else { return }
        let length = CMBlockBufferGetDataLength(block)
        guard length > 0, length <= 2 * 1024 * 1024 else { fail("Invalid H.264 packet size") }
        var bytes = Data(count: length)
        let status = bytes.withUnsafeMutableBytes { raw in
            CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: length, destination: raw.baseAddress!)
        }
        guard status == noErr else { fail("Could not read H.264 output") }
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false) as? [[CFString: Any]]
        let key = attachments?.first?[kCMSampleAttachmentKey_NotSync] as? Bool != true
        writer.lock()
        defer { writer.unlock() }
        if key, let format = sample.formatDescription,
           let atoms = CMFormatDescriptionGetExtension(format, extensionKey: kCMFormatDescriptionExtension_SampleDescriptionExtensionAtoms) as? [String: Any],
           let description = atoms["avcC"] as? Data, description.count >= 4 {
            let name = "avc1." + description[1...3].map { String(format: "%02x", $0) }.joined()
            packet(["kind": "config", "codec": name, "hardware": true], description)
        }
        packet(["kind": "frame", "key": key,
                "timestamp": Int64(sample.presentationTimeStamp.seconds * 1_000_000)], bytes)
    }

    private func encode(_ image: CVPixelBuffer, force: Bool) {
        if let session {
            guard capacity.wait(timeout: .now()) == .success else { return }
            timestamp = max(timestamp + 1, Int64(DispatchTime.now().uptimeNanoseconds / 1000))
            let options: CFDictionary? = force ? [kVTEncodeFrameOptionKey_ForceKeyFrame: true] as CFDictionary : nil
            // Retain this immutable surface until VideoToolbox finishes reading it.
            let retained = Surface(image)
            let status = VTCompressionSessionEncodeFrame(session, imageBuffer: image,
                presentationTimeStamp: CMTime(value: timestamp, timescale: 1_000_000), duration: .invalid,
                frameProperties: options, infoFlagsOut: nil) { [self, retained] status, flags, sample in
                    defer { capacity.signal(); withExtendedLifetime(retained) {} }
                    guard status == noErr else { fail("H.264 encoding failed: \(status)") }
                    if !flags.contains(.frameDropped), let sample { encoded(sample) }
                }
            if status != noErr { capacity.signal(); fail("H.264 submission failed: \(status)") }
        } else {
            autoreleasepool {
                guard let jpeg = context.jpegRepresentation(of: CIImage(cvPixelBuffer: image), colorSpace: color,
                    options: [kCGImageDestinationLossyCompressionQuality as CIImageRepresentationOption: 0.8]) else { fail("JPEG encoding failed") }
                writer.lock()
                defer { writer.unlock() }
                packet(["kind": "jpeg"], jpeg)
            }
        }
    }

    func keyframe() { if let last { encode(last, force: true) } }

    func stream(_ stream: SCStream, didStopWithError error: Error) { fail("Native capture stopped: \(error.localizedDescription)") }

    func stream(_ stream: SCStream, didOutputSampleBuffer sample: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen, sample.isValid,
            let info = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
            let status = info.first?[.status] as? Int, status == SCFrameStatus.complete.rawValue,
            let image = sample.imageBuffer else { return }
        last = image
        encode(image, force: false)
    }
}

@available(macOS 14.0, *)
func run() async {
    guard CommandLine.arguments.count == 6,
        let pid = Int32(CommandLine.arguments[1]), let id = UInt32(CommandLine.arguments[2]),
        let width = Double(CommandLine.arguments[3]), let height = Double(CommandLine.arguments[4]),
        width.isFinite, height.isFinite, width > 0, height > 0,
        ["h264", "jpeg"].contains(CommandLine.arguments[5]) else { fail("Expected PID, window ID, frame dimensions and codec") }
    let codec = CommandLine.arguments[5]
    do {
        let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: false)
        guard let window = content.windows.first(where: { $0.windowID == id && $0.owningApplication?.processID == pid }) else { fail("Native window unavailable; restore it on this desktop") }
        let config = SCStreamConfiguration()
        let scale = min(2, 1920 / width, 1440 / height)
        // Even dimensions are required by H.264's chroma subsampling.
        config.width = max(2, Int(width * scale) / 2 * 2)
        config.height = max(2, Int(height * scale) / 2 * 2)
        config.scalesToFit = true
        config.showsCursor = false
        config.ignoreShadowsSingleWindow = true
        config.ignoreGlobalClipSingleWindow = true
        config.captureResolution = .best
        config.minimumFrameInterval = CMTime(value: 1, timescale: codec == "h264" ? 60 : 30)
        config.queueDepth = 3
        config.capturesAudio = false
        let frames = Frames(codec: codec, width: config.width, height: config.height)
        let stream = SCStream(filter: SCContentFilter(desktopIndependentWindow: window), configuration: config, delegate: frames)
        try stream.addStreamOutput(frames, type: .screen, sampleHandlerQueue: frames.queue)
        // New/lagging viewers can request an IDR even while the app is idle.
        DispatchQueue.global().async {
            while let input = try? FileHandle.standardInput.read(upToCount: 64), !input.isEmpty {
                frames.queue.async { frames.keyframe() }
            }
        }
        defer { withExtendedLifetime((stream, frames)) {} }
        try await stream.startCapture()
        while true { try await Task.sleep(nanoseconds: 1_000_000_000) }
    } catch { fail("Screen capture unavailable: \(error.localizedDescription). Check Screen Recording permission, then reconnect.") }
}

if #available(macOS 14.0, *) {
    _ = NSApplication.shared
    NSApplication.shared.setActivationPolicy(.accessory)
    Task { await run() }
    dispatchMain()
} else { fail("Native preview requires macOS 14 or later") }
