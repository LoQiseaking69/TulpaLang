package dev.hermitech.tulpafield

import android.app.Application
import android.net.Uri
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import dev.hermitech.tulpafield.core.Analysis
import dev.hermitech.tulpafield.core.Completion
import dev.hermitech.tulpafield.core.Diagnostic
import dev.hermitech.tulpafield.core.Example
import dev.hermitech.tulpafield.core.MachineGraph
import dev.hermitech.tulpafield.core.Project
import dev.hermitech.tulpafield.core.Symbol
import dev.hermitech.tulpafield.core.Template
import dev.hermitech.tulpafield.core.TulpaNative
import dev.hermitech.tulpafield.core.TulpaThreads
import dev.hermitech.tulpafield.core.VerifyResult
import dev.hermitech.tulpafield.core.RunListener
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import org.json.JSONObject
import java.io.File
import kotlin.coroutines.resume

enum class Screen { Home, Projects, Code, Run, Sim, Repl, Learn, Sensors, Machines, Verify, Bytecode, Settings }

data class EditorState(
    val project: File? = null,
    val file: File? = null,
    val files: List<File> = emptyList(),
    val text: TextFieldValue = TextFieldValue(""),
    val dirty: Boolean = false,
    val readOnly: Boolean = false,
    val diagnostics: List<Diagnostic> = emptyList(),
    val completions: List<Completion> = emptyList(),
    val prefix: String = "",
    val revealLine: Int? = null,      // one-shot: scroll here
)

data class TestsResult(val passed: Int, val failed: List<Pair<String, String>>, val output: String, val error: String?)

data class VerifyState(
    val running: Boolean = false,
    val results: List<VerifyResult>? = null,
    val error: String? = null,
    val tests: TestsResult? = null,
    val testsRunning: Boolean = false,
)

sealed interface UiEvent {
    data class Toast(val text: String) : UiEvent
    data class Share(val file: File, val mime: String) : UiEvent
    data class ShareText(val text: String, val title: String) : UiEvent
    data object RequestLocation : UiEvent
    data object RequestNotifications : UiEvent
}

class AppViewModel(app: Application) : AndroidViewModel(app) {
    private val a = app as TulpaFieldApp
    val workspace = a.workspace
    val runner = a.runner
    val repl = a.repl
    val sensors = a.sensors
    val settings = a.settings
    val sim = a.sim

    val screen = MutableStateFlow(Screen.Home)
    private val backStack = ArrayDeque<Screen>()

    private val _projects = MutableStateFlow<List<Project>>(emptyList())
    val projects: StateFlow<List<Project>> = _projects
    val examples: List<Example> by lazy { workspace.examples() }
    val guide by lazy { workspace.guide() }
    val modules: Map<String, List<String>> by lazy { Analysis.modules() }

    private val _editor = MutableStateFlow(EditorState())
    val editor: StateFlow<EditorState> = _editor
    private val undo = ArrayDeque<TextFieldValue>()
    private val redo = ArrayDeque<TextFieldValue>()
    private var lastUndoPush = 0L

    val symbols = MutableStateFlow<List<Symbol>>(emptyList())
    val machines = MutableStateFlow<List<MachineGraph>>(emptyList())
    val bytecode = MutableStateFlow("")
    val verify = MutableStateFlow(VerifyState())
    val guideDoc = MutableStateFlow<String?>(null)     // id of open guide chapter
    val guideTab = MutableStateFlow<Int?>(null)        // one-shot: 0 library, 1 guide
    val openExampleId = MutableStateFlow<String?>(null) // one-shot: show this example's page
    val newProjectRequest = MutableStateFlow(false)
    /** Set by the activity: opens the system file picker for an MJCF model. */
    var importModelLauncher: () -> Unit = {}
    val runArgs = MutableStateFlow("")
    val virtualTime = MutableStateFlow(false)

    private val _events = MutableSharedFlow<UiEvent>(extraBufferCapacity = 8)
    val events: SharedFlow<UiEvent> = _events

    private var analysisJob: Job? = null
    private var completionJob: Job? = null
    private var verifyCancel = 0L

    init {
        refreshProjects()
        settings.lastProject?.let { p ->
            val dir = File(p)
            if (dir.isDirectory) openProject(dir, settings.lastFile?.let(::File)?.takeIf { it.exists() }, navigate = false)
        }
    }

    // ------------------------------------------------------------------ navigation

    fun go(s: Screen) {
        if (screen.value == s) return
        if (screen.value == Screen.Code) autosave()
        backStack.addLast(screen.value)
        if (backStack.size > 20) backStack.removeFirst()
        screen.value = s
        when (s) {
            Screen.Machines -> refreshMachines()
            Screen.Bytecode -> refreshBytecode()
            Screen.Projects -> refreshProjects()
            Screen.Sim -> ensureSimModel()
            else -> {}
        }
    }

    /** Returns false when there is nothing to go back to. */
    fun back(): Boolean {
        if (screen.value == Screen.Learn && guideDoc.value != null) { guideDoc.value = null; return true }
        val prev = backStack.removeLastOrNull() ?: return false
        if (screen.value == Screen.Code) autosave()
        screen.value = prev
        return true
    }

    fun toast(t: String) { _events.tryEmit(UiEvent.Toast(t)) }

    // ------------------------------------------------------------------ projects

    fun refreshProjects() { _projects.value = workspace.projects() }

    fun newProject(name: String, template: Template) {
        val dir = workspace.createProject(name, template)
        refreshProjects()
        openProject(dir)
    }

    fun openProject(dir: File, file: File? = null, navigate: Boolean = true) {
        autosave()
        val files = workspace.files(dir)
        val f = file ?: files.firstOrNull { it.name == "main.rav" } ?: files.firstOrNull { it.extension == "rav" } ?: files.firstOrNull()
        settings.lastProject = dir.path
        _editor.value = EditorState(project = dir, files = files)
        repl.setBaseDir(dir)
        if (f != null) openFile(f)
        if (navigate) go(Screen.Code)
    }

    fun openFile(f: File) {
        autosave()
        val text = runCatching { f.readText() }.getOrDefault("")
        undo.clear(); redo.clear()
        settings.lastFile = f.path
        val isText = f.extension in setOf("rav", "tlp", "txt", "md", "csv", "json", "vcd", "toml", "cir", "net", "")
        _editor.update { it.copy(file = f, text = TextFieldValue(text), dirty = false, readOnly = !isText || f.length() > 400_000, diagnostics = emptyList(), completions = emptyList()) }
        scheduleAnalysis(0)
    }

    fun copyExample(e: Example) {
        val dir = workspace.copyExample(e)
        refreshProjects()
        openProject(dir)
        toast("Copied “${e.title}” to your projects")
    }

    fun renameProject(p: Project, name: String) {
        val target = File(workspace.root, workspace.uniqueName(name))
        if (p.dir.renameTo(target)) {
            if (_editor.value.project == p.dir) openProject(target, navigate = false)
        }
        refreshProjects()
    }

    fun duplicateProject(p: Project) { workspace.duplicate(p.dir); refreshProjects() }

    fun deleteProject(p: Project) {
        if (_editor.value.project == p.dir) { _editor.value = EditorState(); settings.lastProject = null }
        workspace.delete(p.dir); refreshProjects()
    }

    fun shareProject(p: Project) {
        val zip = File(getApplication<Application>().cacheDir, "share/${p.name}.zip")
        zip.parentFile?.mkdirs()
        java.util.zip.ZipOutputStream(zip.outputStream()).use { z ->
            p.dir.walkTopDown().filter { it.isFile }.forEach { f ->
                z.putNextEntry(java.util.zip.ZipEntry("${p.name}/" + f.relativeTo(p.dir).path))
                f.inputStream().use { it.copyTo(z) }
                z.closeEntry()
            }
        }
        _events.tryEmit(UiEvent.Share(zip, "application/zip"))
    }

    fun shareFile(f: File) {
        val mime = when (f.extension) { "csv" -> "text/csv"; "json" -> "application/json"; "png" -> "image/png"; else -> "text/plain" }
        _events.tryEmit(UiEvent.Share(f, mime))
    }

    fun newFile(name: String) {
        val p = _editor.value.project ?: return
        val f = workspace.newFile(p, name)
        _editor.update { it.copy(files = workspace.files(p)) }
        openFile(f)
    }

    fun renameFile(name: String) {
        val s = _editor.value
        val f = s.file ?: return
        autosave()
        val t = workspace.rename(f, name)
        _editor.update { it.copy(file = t, files = workspace.files(s.project!!)) }
        settings.lastFile = t.path
    }

    fun deleteFile() {
        val s = _editor.value
        val f = s.file ?: return
        workspace.delete(f)
        val files = workspace.files(s.project!!)
        _editor.update { it.copy(files = files, file = null, text = TextFieldValue(""), dirty = false) }
        files.firstOrNull()?.let { openFile(it) }
    }

    fun refreshFiles() {
        val p = _editor.value.project ?: return
        _editor.update { it.copy(files = workspace.files(p)) }
    }

    fun import(uri: Uri, name: String?) {
        val f = runCatching { workspace.import(uri, name) }.getOrNull()
        if (f == null) { toast("Could not read that file"); return }
        refreshProjects()
        openProject(f.parentFile!!, f)
        toast("Imported ${f.name} into Inbox")
    }

    fun importText(text: String, name: String = "shared.rav") {
        val f = workspace.importText(text, name)
        refreshProjects()
        openProject(f.parentFile!!, f)
    }

    // ------------------------------------------------------------------ editing

    fun edit(v: TextFieldValue) {
        val s = _editor.value
        if (s.readOnly) { _editor.update { it.copy(text = it.text.copy(selection = v.selection)) }; return }
        val changed = v.text != s.text.text
        if (changed) {
            val now = System.currentTimeMillis()
            if (now - lastUndoPush > 700 || undo.isEmpty()) { undo.addLast(s.text); if (undo.size > 200) undo.removeFirst(); lastUndoPush = now }
            redo.clear()
        }
        _editor.update { it.copy(text = v, dirty = it.dirty || changed) }
        if (changed) { scheduleAnalysis(350); scheduleCompletion() }
        else if (v.selection != s.text.selection) _editor.update { it.copy(completions = emptyList()) }
    }

    fun undo() {
        val prev = undo.removeLastOrNull() ?: return
        redo.addLast(_editor.value.text)
        _editor.update { it.copy(text = prev, dirty = true, completions = emptyList()) }
        scheduleAnalysis(200)
    }

    fun redo() {
        val next = redo.removeLastOrNull() ?: return
        undo.addLast(_editor.value.text)
        _editor.update { it.copy(text = next, dirty = true) }
        scheduleAnalysis(200)
    }

    val canUndo get() = undo.isNotEmpty()
    val canRedo get() = redo.isNotEmpty()

    fun save(quiet: Boolean = false) {
        val s = _editor.value
        val f = s.file ?: return
        if (s.readOnly || !s.dirty) return
        runCatching { f.writeText(s.text.text) }
            .onSuccess { _editor.update { it.copy(dirty = false) }; if (!quiet) toast("Saved ${f.name}") }
            .onFailure { toast("Save failed: ${it.message}") }
    }

    fun autosave() { if (settings.prefs.value.autosave) save(quiet = true) }

    fun reveal(line: Int, col: Int = 0) {
        val text = _editor.value.text.text
        var off = 0
        var l = 0
        while (l < line && off < text.length) { val nl = text.indexOf('\n', off); if (nl < 0) { off = text.length; break }; off = nl + 1; l++ }
        val pos = (off + col).coerceAtMost(text.length)
        _editor.update { it.copy(text = it.text.copy(selection = TextRange(pos)), revealLine = line) }
        go(Screen.Code)
    }

    fun revealHandled() { _editor.update { it.copy(revealLine = null) } }

    /** Cursor as zero-based (line, col). */
    fun cursor(): Pair<Int, Int> {
        val v = _editor.value.text
        val pos = v.selection.start.coerceIn(0, v.text.length)
        val before = v.text.substring(0, pos)
        val line = before.count { it == '\n' }
        return line to (pos - (before.lastIndexOf('\n') + 1))
    }

    private fun scheduleAnalysis(delayMs: Long) {
        analysisJob?.cancel()
        analysisJob = viewModelScope.launch {
            delay(delayMs)
            val src = _editor.value.text.text
            if (_editor.value.file?.extension !in setOf("rav", "tlp")) return@launch
            val d = onWorker { Analysis.diagnostics(src) }
            if (_editor.value.text.text == src) _editor.update { it.copy(diagnostics = d) }
        }
    }

    private fun scheduleCompletion() {
        completionJob?.cancel()
        completionJob = viewModelScope.launch {
            delay(120)
            val v = _editor.value.text
            val pos = v.selection.start
            if (v.selection.length != 0 || pos == 0) { _editor.update { it.copy(completions = emptyList()) }; return@launch }
            val text = v.text
            var st = pos
            while (st > 0 && (text[st - 1].isLetterOrDigit() || text[st - 1] == '_')) st--
            val prefix = text.substring(st, pos)
            val afterColons = st >= 2 && text.substring(st - 2, st) == "::"
            if (prefix.isEmpty() && !afterColons) { _editor.update { it.copy(completions = emptyList(), prefix = "") }; return@launch }
            val (line, col) = cursor()
            var items = onWorker { Analysis.completion(text, line, col) }
            if (afterColons) {
                var ms = st - 2; while (ms > 0 && (text[ms - 1].isLetterOrDigit() || text[ms - 1] == '_')) ms--
                val module = text.substring(ms, st - 2)
                val members = modules[module]
                if (members != null && items.none { it.label in members }) items = members.map { Completion(it, 3, "$module::") }
            }
            val filtered = items.filter { it.label.startsWith(prefix) && it.label != prefix }
                .distinctBy { it.label }.sortedWith(compareBy({ !it.label.startsWith(prefix, ignoreCase = false) }, { it.label.length })).take(30)
            if (_editor.value.text.text == text) _editor.update { it.copy(completions = filtered, prefix = prefix) }
        }
    }

    fun applyCompletion(c: Completion) {
        val v = _editor.value.text
        val pos = v.selection.start
        val prefix = _editor.value.prefix
        val st = pos - prefix.length
        val insert = c.label
        val text = v.text.substring(0, st) + insert + v.text.substring(pos)
        edit(TextFieldValue(text, TextRange(st + insert.length)))
        _editor.update { it.copy(completions = emptyList()) }
    }

    fun insert(s: String, caretBack: Int = 0) {
        val v = _editor.value.text
        val a = v.selection.min; val b = v.selection.max
        val text = v.text.substring(0, a) + s + v.text.substring(b)
        edit(TextFieldValue(text, TextRange(a + s.length - caretBack)))
    }

    fun moveCaret(delta: Int) {
        val v = _editor.value.text
        val p = (v.selection.start + delta).coerceIn(0, v.text.length)
        _editor.update { it.copy(text = v.copy(selection = TextRange(p))) }
    }

    fun caretToLineEnd() {
        val v = _editor.value.text
        val nl = v.text.indexOf('\n', v.selection.start).let { if (it < 0) v.text.length else it }
        _editor.update { it.copy(text = v.copy(selection = TextRange(nl))) }
    }

    suspend fun hoverAtCursor(): String? {
        val (l, c) = cursor()
        val src = _editor.value.text.text
        return onWorker { Analysis.hover(src, l, c) ?: Analysis.hover(src, l, (c - 1).coerceAtLeast(0)) }
    }

    fun gotoDefinition() {
        val (l, c) = cursor()
        val src = _editor.value.text.text
        viewModelScope.launch {
            val d = onWorker { Analysis.definition(src, l, c) ?: Analysis.definition(src, l, (c - 1).coerceAtLeast(0)) }
            if (d == null) toast("No definition found here") else reveal(d.first, d.second)
        }
    }

    fun refreshSymbols() {
        val src = _editor.value.text.text
        viewModelScope.launch { symbols.value = onWorker { Analysis.symbols(src) } }
    }

    fun refreshMachines() {
        val src = _editor.value.text.text
        viewModelScope.launch { machines.value = onWorker { Analysis.machines(src) } }
    }

    fun refreshBytecode() {
        val src = _editor.value.text.text
        viewModelScope.launch {
            bytecode.value = "compiling…"
            bytecode.value = onWorker { runCatching { TulpaNative.bytecode(src) }.getOrElse { it.message ?: "error" } }
        }
    }

    // ------------------------------------------------------------------ welcome / navigation helpers

    fun openGuide(id: String?) { guideTab.value = 1; guideDoc.value = id; go(Screen.Learn) }
    fun openExample(id: String) { guideTab.value = 0; guideDoc.value = null; openExampleId.value = id; go(Screen.Learn) }
    fun requestNewProject() { newProjectRequest.value = true }

    // ------------------------------------------------------------------ simulation

    /** Loads a model for the Sim tab if none is open: the project's, else the cart-pole. */
    fun ensureSimModel() {
        if (sim.state.value.model != null || sim.state.value.loading) return
        val dir = _editor.value.project
        val f = dir?.let { sim.modelFor(it) } ?: java.io.File(workspace.models, "cartpole.xml")
        loadSimModel(f)
    }

    fun loadSimModel(f: File) {
        viewModelScope.launch(kotlinx.coroutines.Dispatchers.IO) { sim.load(f)?.let { toast("Cannot load ${f.name}: $it") } }
    }

    fun importModel(uri: Uri, name: String?) {
        val text = runCatching { getApplication<Application>().contentResolver.openInputStream(uri)?.use { it.readBytes().decodeToString() } }.getOrNull()
        if (text == null || !text.contains("<mujoco")) { toast("That file is not a MuJoCo MJCF model"); return }
        val dir = _editor.value.project ?: workspace.createProject("Models", Template.Blank).also { refreshProjects() }
        var n = (name ?: "model.xml").substringAfterLast('/')
        if (!n.endsWith(".xml")) n += ".xml"
        val f = File(dir, n)
        f.writeText(text)
        refreshFiles()
        loadSimModel(f)
        toast("Imported ${f.name} into ${dir.name}")
    }

    /** A new project wired to [m]: hermisim.json + a starter controller using its actuators. */
    fun newCosimProject(m: dev.hermitech.tulpafield.core.SimModel) {
        val dir = workspace.createProject("${m.name} co-sim", Template.Blank)
        val bundled = File(m.path).parentFile == workspace.models
        val rel = if (bundled) "../../models/${File(m.path).name}" else File(m.path).also { it.copyTo(File(dir, it.name), true) }.name
        File(dir, "hermisim.json").writeText(org.json.JSONObject().put("name", m.name).put("model", rel).put("entry", "main.rav").toString(2))
        File(dir, "main.rav").writeText("""
            # Co-simulation with ${m.name}: ${m.actuators} actuators, ${m.joints} joints.
            # Run this file, then open the Sim tab to watch it in 3D.

            let sim = cosim::connect()
            let info = sim.info()
            print(f"{info.model}: {info.nu} actuators, timestep {info.timestep} s")
            for a in info.actuators { print(" ", a.name, a.ctrlrange) }

            let o = sim.reset()
            let ctrl = [0.0] * info.nu
            while o.time < 5.0 {
                # your controller here: compute ctrl from the observation o
                o = sim.step(ctrl, 10)
                tlm::plot("com z", o.com[2])
            }
            print(f"done at t = {o.time:.2f} s, com = {o.com}")
        """.trimIndent() + "\n")
        refreshProjects()
        openProject(dir)
    }

    /** Exports TULPA_COSIM for programs that use cosim::; returns a console note or an error. */
    private fun prepareCosim(src: String, dir: File): Result<String?> {
        val remote = settings.prefs.value.remoteCosim.takeIf { it.isNotBlank() }
        return sim.prepareRun(src, dir, remote).map { addr ->
            addr?.let {
                if (remote != null) "co-sim: remote simulator at $it"
                else "co-sim: ${sim.state.value.model?.name} (MuJoCo, on device) — open the Sim tab to watch in 3D"
            }
        }
    }

    // ------------------------------------------------------------------ running

    private fun permissionsFor(src: String) {
        if (src.contains("phone::location") && !sensors.hasLocationPermission()) _events.tryEmit(UiEvent.RequestLocation)
        else if (src.contains("phone::location")) sensors.setLocationEnabled(true)
        if (src.contains("phone::notify")) _events.tryEmit(UiEvent.RequestNotifications)
    }

    fun runCurrent() {
        val s = _editor.value
        val f = s.file ?: run { toast("Open a file first"); return }
        save(quiet = true)
        if (runner.state.value.running) { toast("A program is already running — stop it first"); return }
        permissionsFor(s.text.text)
        val args = runArgs.value.trim().split(Regex("\\s+")).filter { it.isNotEmpty() }
        val dir = f.parentFile ?: s.project!!
        val src = s.text.text
        viewModelScope.launch {
            val note = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) { prepareCosim(src, dir) }
                .getOrElse { toast(it.message ?: "co-sim failed"); return@launch }
            runner.start(src, f, dir, args, virtualTime.value, title = f.relativeTo(s.project ?: f.parentFile!!).path, note = note)
            go(if (note != null && screen.value == Screen.Sim) Screen.Sim else Screen.Run)
        }
    }

    fun runExample(e: Example) {
        if (runner.state.value.running) { toast("A program is already running — stop it first"); return }
        val src = e.main.readText()
        permissionsFor(src)
        viewModelScope.launch {
            val note = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) { prepareCosim(src, e.dir) }
                .getOrElse { toast(it.message ?: "co-sim failed"); return@launch }
            runner.start(src, e.main, e.dir, e.args, e.virtualTime, title = e.title, note = note)
            go(if (note != null) Screen.Sim else Screen.Run)
        }
    }

    fun runSnippet(code: String, title: String = "snippet") {
        if (runner.state.value.running) { toast("A program is already running — stop it first"); return }
        val dir = File(getApplication<Application>().filesDir, "scratch").apply { mkdirs() }
        val f = File(dir, "snippet.rav").apply { writeText(code) }
        permissionsFor(code)
        viewModelScope.launch {
            val note = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) { prepareCosim(code, dir) }
                .getOrElse { toast(it.message ?: "co-sim failed"); return@launch }
            runner.start(code, f, dir, emptyList(), false, title = title, note = note)
            go(Screen.Run)
        }
    }

    fun snippetToProject(code: String, title: String) {
        val dir = workspace.createProject(title, Template.Blank)
        File(dir, "main.rav").writeText(code)
        refreshProjects()
        openProject(dir)
    }

    fun loadIntoRepl(code: String, name: String) { repl.load(code, name); go(Screen.Repl) }

    fun locationGranted() { sensors.setLocationEnabled(true) }

    // ------------------------------------------------------------------ verification

    fun runVerify() {
        val s = _editor.value
        if (s.file == null) return
        val src = s.text.text
        val base = (s.file.parentFile ?: s.project)!!.path
        val trials = settings.prefs.value.trials
        verify.update { it.copy(running = true, error = null) }
        val handle = TulpaNative.newCancel()
        verifyCancel = handle
        TulpaThreads.start("tulpa-verify") {
            val json = runCatching { TulpaNative.verify(src, base, trials, handle) }.getOrElse { "{\"error\":\"${it.message}\"}" }
            TulpaNative.freeCancel(handle)
            if (verifyCancel == handle) verifyCancel = 0
            val (r, err) = runCatching { Analysis.parseVerify(json) }.getOrElse { emptyList<VerifyResult>() to it.message }
            verify.update { it.copy(running = false, results = r, error = err) }
        }
    }

    fun stopVerify() { if (verifyCancel != 0L) TulpaNative.cancel(verifyCancel) }

    fun runTests() {
        val s = _editor.value
        val f = s.file ?: return
        val src = s.text.text
        verify.update { it.copy(testsRunning = true) }
        val out = StringBuilder()
        val handle = TulpaNative.newCancel()
        TulpaThreads.start("tulpa-tests") {
            val listener = object : RunListener {
                override fun onOutput(text: String) { synchronized(out) { text.lineSequence().filter { !it.startsWith("@@tlm") }.joinTo(out, "\n"); } }
                override fun onInput(prompt: String): String? = null
            }
            val json = runCatching { TulpaNative.tests(src, f.path, (f.parentFile ?: s.project)!!.path, listener, handle) }.getOrElse { "{\"error\":\"${it.message}\"}" }
            TulpaNative.freeCancel(handle)
            val o = JSONObject(json)
            val res = if (o.has("error")) TestsResult(0, emptyList(), out.toString(), o.optString("error"))
            else {
                val fa = o.optJSONArray("failed")
                TestsResult(o.optInt("passed"), (0 until (fa?.length() ?: 0)).map { fa!!.getJSONObject(it).let { x -> x.optString("name") to x.optString("error") } }, out.toString(), null)
            }
            verify.update { it.copy(testsRunning = false, tests = res) }
        }
    }

    fun tryCounterexample(r: VerifyResult) {
        val s = _editor.value
        repl.load(s.text.text, s.file?.name ?: "file")
        repl.submit("${r.name}(${r.counterexample.joinToString(", ")})")
        go(Screen.Repl)
    }

    fun share(text: String, title: String) { _events.tryEmit(UiEvent.ShareText(text, title)) }

    override fun onCleared() { autosave(); super.onCleared() }

    private suspend fun <T> onWorker(f: () -> T): T = suspendCancellableCoroutine { cont ->
        Analysis.worker.post {
            val r = f()
            if (cont.isActive) cont.resume(r)
        }
    }
}
