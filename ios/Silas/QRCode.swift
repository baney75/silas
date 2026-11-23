// Making and reading the pairing QR code: CoreImage draws it, AVFoundation reads it from the camera.

import AVFoundation
import CoreImage.CIFilterBuiltins
import SwiftUI
import UIKit

enum QRCode {
    static func image(for string: String) -> UIImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(string.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)),
            let cg = CIContext().createCGImage(output, from: output.extent)
        else { return nil }
        return UIImage(cgImage: cg)
    }
}

/// A camera sheet that reads one QR code holding `silas://pair?k=…` (or a bare code) and hands it back.
struct QRScannerSheet: View {
    let found: (String) -> Void
    @Environment(\.dismiss) private var dismiss
    @State private var denied = false

    var body: some View {
        NavigationStack {
            ZStack {
                Color.black.ignoresSafeArea()
                if denied {
                    Text("Silas needs the camera to scan the code. Turn it on in Settings › Apps › Silas, or type the code instead.")
                        .foregroundStyle(Theme.text2)
                        .multilineTextAlignment(.center)
                        .padding(24)
                } else if AVCaptureDevice.default(for: .video) == nil {
                    Text("No camera here. Type the code instead.")
                        .foregroundStyle(Theme.text2)
                } else {
                    ScannerView { value in
                        found(Self.code(from: value))
                    }
                    .ignoresSafeArea()
                    RoundedRectangle(cornerRadius: 16)
                        .strokeBorder(Theme.gold, lineWidth: 3)
                        .frame(width: 240, height: 240)
                }
            }
            .navigationTitle("Scan the pairing code")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
            }
        }
        .task {
            switch AVCaptureDevice.authorizationStatus(for: .video) {
            case .notDetermined:
                denied = !(await AVCaptureDevice.requestAccess(for: .video))
            case .denied, .restricted:
                denied = true
            default:
                break
            }
        }
    }

    /// `silas://pair?k=CODE` gives CODE; anything else is taken as typed.
    static func code(from value: String) -> String {
        if let c = URLComponents(string: value), c.scheme == "silas",
            let k = c.queryItems?.first(where: { $0.name == "k" })?.value
        {
            return k
        }
        return value
    }
}

private struct ScannerView: UIViewControllerRepresentable {
    let found: (String) -> Void

    func makeUIViewController(context: Context) -> ScannerController {
        let c = ScannerController()
        c.found = found
        return c
    }

    func updateUIViewController(_ controller: ScannerController, context: Context) {}
}

private final class ScannerController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    var found: ((String) -> Void)?
    private let session = AVCaptureSession()
    private var preview: AVCaptureVideoPreviewLayer?
    private var done = false

    override func viewDidLoad() {
        super.viewDidLoad()
        guard let device = AVCaptureDevice.default(for: .video),
            let input = try? AVCaptureDeviceInput(device: device), session.canAddInput(input)
        else { return }
        session.addInput(input)
        let output = AVCaptureMetadataOutput()
        guard session.canAddOutput(output) else { return }
        session.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: .main)
        output.metadataObjectTypes = [.qr]
        let layer = AVCaptureVideoPreviewLayer(session: session)
        layer.videoGravity = .resizeAspectFill
        view.layer.addSublayer(layer)
        preview = layer
        let s = session
        DispatchQueue.global(qos: .userInitiated).async { s.startRunning() }
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        preview?.frame = view.bounds
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        let s = session
        DispatchQueue.global(qos: .userInitiated).async { s.stopRunning() }
    }

    func metadataOutput(_ output: AVCaptureMetadataOutput, didOutput objects: [AVMetadataObject], from connection: AVCaptureConnection) {
        guard !done, let value = (objects.first as? AVMetadataMachineReadableCodeObject)?.stringValue else { return }
        done = true
        UINotificationFeedbackGenerator().notificationOccurred(.success)
        found?(value)
    }
}
