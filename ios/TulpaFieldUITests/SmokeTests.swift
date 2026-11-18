import XCTest

/// Simulator smoke test (mirrors android/ci/smoke.sh): launch, the guide, a library example,
/// opening it as a project and running it from the editor, the REPL, sensors, and the MuJoCo co-sim.
/// Screenshots go to the xcresult bundle and, when TF_SHOT_DIR is set, to that directory.
final class SmokeTests: XCTestCase {
    var app: XCUIApplication!

    override func setUp() {
        continueAfterFailure = false
        app = XCUIApplication()
        app.launchArguments += ["-ui-testing"]
        addUIInterruptionMonitor(withDescription: "permission prompts") { alert in
            for b in ["Allow", "Allow While Using App", "OK", "Allow Once"] where alert.buttons[b].exists {
                alert.buttons[b].tap(); return true
            }
            return false
        }
        app.launch()
    }

    private func shot(_ name: String) {
        let s = XCUIScreen.main.screenshot()
        let a = XCTAttachment(screenshot: s)
        a.name = name; a.lifetime = .keepAlways
        add(a)
        if let dir = ProcessInfo.processInfo.environment["TF_SHOT_DIR"], !dir.isEmpty {
            try? FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
            try? s.pngRepresentation.write(to: URL(fileURLWithPath: dir).appendingPathComponent("\(name).png"))
        }
    }

    private func el(_ id: String) -> XCUIElement { app.descendants(matching: .any)[id] }

    private func text(containing s: String) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@ OR value CONTAINS %@", s, s)).firstMatch
    }

    @discardableResult
    private func wait(_ e: XCUIElement, _ t: TimeInterval = 15, _ what: String) -> XCUIElement {
        if !e.waitForExistence(timeout: t) { shot("fail-\(what)"); XCTFail("timed out waiting for \(what)") }
        return e
    }

    private func tab(_ label: String) { wait(el("tab-\(label)"), 10, "tab \(label)").tap() }

    private func openExample(_ id: String, search: String) {
        tab("Learn")
        let lib = app.buttons["Library"]
        if lib.waitForExistence(timeout: 5) { lib.tap() }
        let field = wait(el("search-examples"), 10, "example search")
        field.tap()
        field.typeText(search)
        wait(el("example-\(id)"), 10, "example \(id)").tap()
    }

    func test1_HomeAndGuide() {
        wait(text(containing: "Welcome to Tulpa Field"), 30, "welcome page")
        shot("00-home")
        wait(el("start-tour"), 5, "Start the tour").tap()
        sleep(2)
        shot("01-guide")
        let run = el("snippet-run")
        for _ in 0..<8 where !(run.exists && run.isHittable) { app.swipeUp() }
        wait(run, 10, "guide snippet Run").tap()
        sleep(3)
        shot("02-snippet-run")
        XCTAssertTrue(el("console").waitForExistence(timeout: 10) || el("run-status").exists, "snippet did not open the console")
    }

    func test2_LibraryExampleRuns() {
        openExample("hello_world", search: "Hello")
        shot("03-example")
        wait(el("example-run"), 10, "example Run").tap()
        wait(text(containing: "Hello, World!"), 30, "Hello, World! output")
        shot("04-example-output")
    }

    func test3_OpenAsProjectAndRunFromEditor() {
        openExample("hello_world", search: "Hello")
        wait(el("open-as-project"), 10, "Open as project").tap()
        wait(el("code-editor"), 15, "code editor")
        shot("05-editor")
        tab("Run")
        wait(el("run"), 10, "Run button").tap()
        wait(text(containing: "Tulpa User"), 30, "program output")
        shot("06-run")
    }

    func test4_Repl() {
        tab("REPL")
        let input = wait(el("repl-input"), 10, "REPL input")
        input.tap()
        input.typeText("3[ft] + 1[m]\n")
        wait(text(containing: "6.2808"), 15, "REPL result")
        shot("07-repl")
    }

    func test5_Sensors() {
        tab("Home")
        sleep(1)
        shot("08a-home-returning")
        let menu = wait(el("instruments"), 15, "instruments menu")
        if menu.isHittable { menu.tap() } else { menu.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.5)).tap() }
        wait(app.buttons["Sensors"], 5, "Sensors item").tap()
        wait(el("sensors-screen"), 10, "sensors screen")
        sleep(2)
        shot("08-sensors")
    }

    func test6_CosimCartPole() {
        openExample("cosim_cartpole", search: "Cart")
        wait(el("example-run"), 10, "example Run").tap()
        wait(text(containing: "co-sim: driven by"), 90, "co-sim HUD")
        sleep(4)
        shot("09-cosim")
        tab("Run")
        sleep(2)
        shot("10-cosim-console")
    }
}
