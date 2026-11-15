package dev.hermitech.tulpafield.core

import android.content.Context
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

enum class ReplKind { Input, Output, Value, Error, Info }

data class ReplEntry(val id: Long, val kind: ReplKind, val text: String)

data class ReplState(
    val entries: List<ReplEntry> = emptyList(),
    val busy: Boolean = false,
    val continuation: Boolean = false,
    val names: List<String> = emptyList(),
    val prompt: String? = null,
)

/** One persistent Tulpa session on its own big-stack thread. */
class ReplManager(context: Context, private val sensors: SensorBridge) {
    private val worker = TulpaThreads.Worker("tulpa-repl")
    private var handle = 0L
    private var ids = 0L
    private val historyFile = File(context.filesDir, "repl_history.json")
    private val _state = MutableStateFlow(ReplState())
    val state: StateFlow<ReplState> = _state
    val history: MutableList<String> = loadHistory()
    private val inputQueue = java.util.concurrent.LinkedBlockingQueue<String>()
    private var baseDir: File = context.filesDir

    private val listener = object : RunListener {
        private val buf = StringBuilder()
        override fun onOutput(text: String) {
            buf.append(text)
            while (true) {
                val nl = buf.indexOf('\n'); if (nl < 0) break
                val l = buf.substring(0, nl); buf.delete(0, nl + 1)
                if (!l.startsWith("@@tlm ")) add(ReplKind.Output, l)
            }
        }
        override fun onInput(prompt: String): String? {
            _state.update { it.copy(prompt = prompt) }
            val a = inputQueue.take()
            _state.update { it.copy(prompt = null) }
            return if (a == "\u0000") null else a
        }
        fun flush() { if (buf.isNotEmpty()) { add(ReplKind.Output, buf.toString()); buf.clear() } }
    }

    init {
        add(ReplKind.Info, "Tulpa ${runCatching { TulpaNative.version() }.getOrDefault("")} REPL — definitions persist; phone:: works here too.")
    }

    private fun ensure() {
        if (handle == 0L) {
            handle = TulpaNative.sessionNew(listener, baseDir.path)
            sensors.acquire()
        }
    }

    fun setBaseDir(dir: File) {
        if (dir == baseDir) return
        baseDir = dir
    }

    fun submit(line: String) {
        if (line.isNotBlank() && history.lastOrNull() != line) { history += line; saveHistory() }
        add(ReplKind.Input, (if (_state.value.continuation) "… " else "› ") + line)
        _state.update { it.copy(busy = true) }
        worker.post {
            ensure()
            val r = runCatching { JSONObject(TulpaNative.sessionFeed(handle, line)) }.getOrElse { JSONObject().put("kind", "error").put("text", it.message) }
            listener.flush()
            when (r.optString("kind")) {
                "more" -> _state.update { it.copy(continuation = true) }
                "value" -> { if (!r.isNull("text")) add(ReplKind.Value, r.optString("text")); _state.update { it.copy(continuation = false) } }
                else -> { add(ReplKind.Error, r.optString("text")); _state.update { it.copy(continuation = false) } }
            }
            val names = runCatching { JSONArray(TulpaNative.sessionNames(handle)).let { a -> (0 until a.length()).map { a.getString(it) } } }.getOrDefault(emptyList())
            _state.update { it.copy(busy = false, names = names) }
        }
    }

    /** Runs a whole file into the session (definitions and top-level code). */
    fun load(source: String, name: String) {
        add(ReplKind.Info, "loaded $name")
        _state.update { it.copy(busy = true) }
        worker.post {
            ensure()
            if (_state.value.continuation) TulpaNative.sessionCancelEntry(handle)
            val r = runCatching { JSONObject(TulpaNative.sessionFeed(handle, source)) }.getOrNull()
            listener.flush()
            if (r?.optString("kind") == "error") add(ReplKind.Error, r.optString("text"))
            if (r?.optString("kind") == "more") { TulpaNative.sessionCancelEntry(handle); add(ReplKind.Error, "file is incomplete (unbalanced brackets)") }
            val names = runCatching { JSONArray(TulpaNative.sessionNames(handle)).let { a -> (0 until a.length()).map { a.getString(it) } } }.getOrDefault(emptyList())
            _state.update { it.copy(busy = false, continuation = false, names = names) }
        }
    }

    fun answer(text: String) { add(ReplKind.Input, (_state.value.prompt ?: "") + text); inputQueue.put(text) }

    fun interrupt() {
        if (handle != 0L) TulpaNative.sessionInterrupt(handle)
        if (_state.value.prompt != null) inputQueue.put("\u0000")
    }

    fun cancelEntry() {
        worker.post { if (handle != 0L) TulpaNative.sessionCancelEntry(handle) }
        _state.update { it.copy(continuation = false) }
        add(ReplKind.Info, "entry discarded")
    }

    fun reset() {
        interrupt()
        worker.clearPending()
        worker.post {
            if (handle != 0L) { TulpaNative.sessionFree(handle); handle = 0; sensors.release() }
            _state.value = ReplState()
            add(ReplKind.Info, "session reset")
        }
    }

    fun clearScreen() { _state.update { it.copy(entries = emptyList()) } }

    private fun add(kind: ReplKind, text: String) {
        _state.update { s -> s.copy(entries = (s.entries + ReplEntry(ids++, kind, text)).takeLast(2000)) }
    }

    private fun loadHistory(): MutableList<String> = runCatching {
        val a = JSONArray(historyFile.readText()); MutableList(a.length()) { a.getString(it) }
    }.getOrDefault(mutableListOf())

    private fun saveHistory() {
        runCatching { historyFile.writeText(JSONArray(history.takeLast(300)).toString()) }
    }
}
