package dev.hermitech.tulpafield.core

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import org.json.JSONObject
import java.io.File
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit

enum class LineKind { Out, Err, Info, Event, Input }

data class ConsoleLine(val id: Long, val kind: LineKind, val text: String, val line: Int? = null)

data class PlotSeries(val name: String, val unit: String?, val t: FloatArray, val v: FloatArray, val count: Int) {
    val last: Float? get() = if (count > 0) v[(count - 1) % v.size] else null
}

data class Artifact(val path: String, val kind: String?)

data class RunState(
    val running: Boolean = false,
    val title: String = "",
    val console: List<ConsoleLine> = emptyList(),
    val plots: Map<String, PlotSeries> = emptyMap(),
    val events: List<Pair<Float, String>> = emptyList(),
    val status: String? = null,
    val artifacts: List<Artifact> = emptyList(),
    val prompt: String? = null,
    val startedAt: Long = 0,
    val finishedAt: Long = 0,
    val exit: String? = null,           // null while running / on success; error text otherwise
    val ok: Boolean? = null,
)

/**
 * Runs one Tulpa program at a time on a 64 MiB thread; turns its output into console lines,
 * `@@tlm` telemetry into plots/events/status/artifacts, and bridges `input()`.
 */
class RunManager(private val sensors: SensorBridge) {
    private val _state = MutableStateFlow(RunState())
    val state: StateFlow<RunState> = _state

    private var cancel = 0L
    private var ids = 0L
    private val partial = StringBuilder()
    private var inputQueue: LinkedBlockingQueue<Any>? = null
    private val pendingLines = ArrayList<ConsoleLine>()
    private var plotT0: Double? = null
    @Volatile private var generation = 0

    companion object {
        const val MAX_LINES = 4000
        const val PLOT_POINTS = 600
        private val NULL_ANSWER = Any()
        private val LINE_RE = Regex("""(?:line |:)(\d+)(?::\d+)?""")
    }

    fun clear() { if (!_state.value.running) _state.value = RunState() }

    fun start(
        source: String, file: File, baseDir: File, args: List<String>, virtualTime: Boolean,
        title: String = file.name, useSensors: Boolean = source.contains("phone::"), note: String? = null,
    ) {
        if (_state.value.running) return
        val gen = ++generation
        cancel = TulpaNative.newCancel()
        val handle = cancel
        partial.clear(); pendingLines.clear(); plotT0 = null
        val q = LinkedBlockingQueue<Any>()
        inputQueue = q
        TulpaHost.resetForRun()
        _state.value = RunState(running = true, title = title, startedAt = System.currentTimeMillis(),
            console = listOf(ConsoleLine(ids++, LineKind.Info, "━━ Run: $title" + (if (args.isNotEmpty()) " " + args.joinToString(" ") else "") +
                (if (virtualTime) "  (virtual time)" else ""))) + listOfNotNull(note?.let { ConsoleLine(ids++, LineKind.Info, it) }))
        TulpaHost.onEvent = { emit(LineKind.Event, it) }
        if (useSensors) sensors.acquire()
        TulpaThreads.start("tulpa-run") {
            val listener = object : RunListener {
                override fun onOutput(text: String) { if (gen == generation) ingest(text) }
                override fun onInput(prompt: String): String? {
                    flushPartial()
                    _state.update { it.copy(prompt = prompt) }
                    while (true) {
                        val a = q.poll(200, TimeUnit.MILLISECONDS)
                        if (a != null) {
                            _state.update { it.copy(prompt = null) }
                            return if (a === NULL_ANSWER) null else a as String
                        }
                        if (gen != generation) return null
                    }
                }
            }
            val err = try {
                TulpaNative.run(source, file.path, baseDir.path, args.toTypedArray(), virtualTime, listener, handle)
            } catch (t: Throwable) { "internal error: ${t.message}" }
            flushPartial()
            if (useSensors) sensors.release()
            TulpaHost.releaseAll()
            TulpaNative.freeCancel(handle)
            val now = System.currentTimeMillis()
            val secs = (now - _state.value.startedAt) / 1000.0
            when {
                err == null -> emit(LineKind.Info, "━━ finished in %.2f s".format(secs))
                err == "interrupted" || err.endsWith(": interrupted") -> emit(LineKind.Info, "━━ stopped after %.2f s".format(secs))
                else -> emit(LineKind.Err, err, lineOf(err))
            }
            flushNow()
            _state.update { it.copy(running = false, prompt = null, finishedAt = now, exit = err, ok = err == null) }
            if (cancel == handle) cancel = 0
        }
    }

    /** `cosim` notifications from the program: `sim.plot(name, v)` and `sim.log(text)`. */
    fun simNotify(method: String, params: JSONObject) {
        when (method) {
            "log" -> emit(LineKind.Event, "▣ " + params.optString("text"))
            "plot" -> {
                val v = params.opt("v")
                val x = when (v) { is Number -> v.toDouble(); is org.json.JSONArray -> v.optDouble(0); else -> Double.NaN }
                telemetry(JSONObject().put("k", "plot").put("name", params.optString("name")).put("v", x).put("t", System.currentTimeMillis() / 1000.0))
            }
        }
    }

    fun stop() {
        val h = cancel
        if (h != 0L) TulpaNative.cancel(h)
        inputQueue?.offer(NULL_ANSWER)
    }

    fun answer(text: String?) {
        if (text != null) emit(LineKind.Input, (_state.value.prompt ?: "") + text)
        inputQueue?.offer(text ?: NULL_ANSWER)
        flushNow()
    }

    private fun lineOf(err: String): Int? = LINE_RE.find(err)?.groupValues?.get(1)?.toIntOrNull()

    // ------------------------------------------------------------------ output parsing

    @Synchronized
    private fun ingest(text: String) {
        partial.append(text)
        while (true) {
            val nl = partial.indexOf('\n')
            if (nl < 0) break
            val line = partial.substring(0, nl)
            partial.delete(0, nl + 1)
            handleLine(line)
        }
        flushNow()
    }

    @Synchronized
    private fun flushPartial() {
        if (partial.isNotEmpty()) { handleLine(partial.toString()); partial.clear() }
        flushNow()
    }

    private fun handleLine(line: String) {
        if (line.startsWith("@@tlm ")) {
            runCatching { telemetry(JSONObject(line.substring(6))) }
            return
        }
        val isErr = line.startsWith("error") || line.startsWith("Error") || line.contains("safety envelope violated")
        synchronized(pendingLines) {
            pendingLines += ConsoleLine(ids++, if (isErr) LineKind.Err else LineKind.Out, line, if (isErr) lineOf(line) else null)
        }
    }

    private fun telemetry(o: JSONObject) {
        when (o.optString("k")) {
            "plot" -> {
                val name = o.optString("name")
                val v = o.optDouble("v", Double.NaN)
                if (v.isNaN()) return
                val t = o.optDouble("t", 0.0)
                val t0 = plotT0 ?: t.also { plotT0 = it }
                val unit = if (o.isNull("unit")) null else o.optString("unit")
                _state.update { s ->
                    val old = s.plots[name] ?: PlotSeries(name, unit, FloatArray(PLOT_POINTS), FloatArray(PLOT_POINTS), 0)
                    val i = old.count % PLOT_POINTS
                    old.t[i] = (t - t0).toFloat(); old.v[i] = v.toFloat()
                    s.copy(plots = s.plots + (name to old.copy(count = old.count + 1)))
                }
            }
            "event" -> {
                val data = o.opt("data")
                val label = o.optString("name") + if (data == null || data == JSONObject.NULL) "" else " $data"
                val t = (o.optDouble("t", 0.0) - (plotT0 ?: 0.0)).toFloat()
                _state.update { it.copy(events = (it.events + (t to label)).takeLast(100)) }
                synchronized(pendingLines) { pendingLines += ConsoleLine(ids++, LineKind.Event, "◆ $label") }
            }
            "status" -> _state.update { it.copy(status = o.optString("text")) }
            "artifact" -> {
                val a = Artifact(o.optString("path"), if (o.isNull("kind")) null else o.optString("kind"))
                _state.update { s -> s.copy(artifacts = (s.artifacts.filter { it.path != a.path } + a)) }
            }
        }
    }

    private fun emit(kind: LineKind, text: String, line: Int? = null) {
        synchronized(pendingLines) { pendingLines += ConsoleLine(ids++, kind, text, line) }
        flushNow()
    }

    private fun flushNow() {
        val batch = synchronized(pendingLines) {
            if (pendingLines.isEmpty()) return
            ArrayList(pendingLines).also { pendingLines.clear() }
        }
        _state.update { s ->
            val all = s.console + batch
            s.copy(console = if (all.size > MAX_LINES) all.takeLast(MAX_LINES) else all)
        }
    }
}
