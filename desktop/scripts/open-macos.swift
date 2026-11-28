import AppKit

// Read the existing relay credential over stdin, never in argv or a release archive.
guard let token = readLine(), (32...512).contains(token.count), !token.contains(where: \.isWhitespace) else {
    fputs("Cannot read Naro relay configuration\n", stderr)
    exit(1)
}
let configuration = NSWorkspace.OpenConfiguration()
configuration.environment = [
    "NAROLINK_RELAY_URL": "https://naro.bato.chat",
    "NAROLINK_RELAY_TOKEN": token
]
configuration.activates = true
NSWorkspace.shared.openApplication(at: URL(fileURLWithPath: "/Applications/Naro.app", isDirectory: true), configuration: configuration) { app, _ in
    guard let app else {
        fputs("Cannot launch Naro.app\n", stderr)
        exit(1)
    }
    print("Naro started, PID \(app.processIdentifier)")
    exit(0)
}
RunLoop.main.run()
