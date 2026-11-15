package dev.hermitech.tulpafield.core

import org.json.JSONArray
import org.json.JSONObject

data class Diagnostic(val line: Int, val col: Int, val endLine: Int, val endCol: Int, val message: String, val error: Boolean)
data class Completion(val label: String, val kind: Int, val detail: String)
data class Symbol(val name: String, val kind: Int, val line: Int, val detail: String)

data class MachineState(val name: String, val line: Int, val enter: Boolean, val exit: Boolean)
data class MachineTransition(
    val from: String, val to: String, val event: String, val params: List<String>,
    val guard: String?, val internal: Boolean, val global: Boolean, val line: Int,
)
data class MachineGraph(val name: String, val initial: String, val states: List<MachineState>, val transitions: List<MachineTransition>, val mermaid: String)

data class VerifyResult(val name: String, val ok: Boolean, val passed: Int, val discarded: Int, val counterexample: List<String>, val error: String?)

/** Editor intelligence, executed on a dedicated big-stack worker (the parser is recursive). */
object Analysis {
    val worker = TulpaThreads.Worker("tulpa-analysis", 32L * 1024 * 1024)

    fun diagnostics(src: String): List<Diagnostic> = runCatching {
        val a = JSONArray(TulpaNative.diagnostics(src))
        (0 until a.length()).map {
            val o = a.getJSONObject(it)
            val r = o.getJSONObject("range")
            val s = r.getJSONObject("start"); val e = r.getJSONObject("end")
            Diagnostic(s.getInt("line"), s.getInt("character"), e.getInt("line"), e.getInt("character"),
                o.optString("message"), o.optInt("severity", 1) == 1)
        }
    }.getOrDefault(emptyList())

    fun completion(src: String, line: Int, col: Int): List<Completion> = runCatching {
        val a = JSONArray(TulpaNative.completion(src, line, col))
        (0 until a.length()).map {
            val o = a.getJSONObject(it)
            Completion(o.optString("label"), o.optInt("kind"), o.optString("detail"))
        }
    }.getOrDefault(emptyList())

    fun symbols(src: String): List<Symbol> = runCatching {
        val a = JSONArray(TulpaNative.symbols(src))
        (0 until a.length()).map {
            val o = a.getJSONObject(it)
            val line = o.optJSONObject("range")?.optJSONObject("start")?.optInt("line")
                ?: o.optJSONObject("location")?.optJSONObject("range")?.optJSONObject("start")?.optInt("line") ?: 0
            Symbol(o.optString("name"), o.optInt("kind"), line, o.optString("detail"))
        }
    }.getOrDefault(emptyList())

    fun hover(src: String, line: Int, col: Int): String? = runCatching { TulpaNative.hover(src, line, col) }.getOrNull()

    /** Zero-based (line, col) of the declaration, or null. */
    fun definition(src: String, line: Int, col: Int): Pair<Int, Int>? = runCatching {
        val s = TulpaNative.definition(src, line, col)
        if (s == "null") null else JSONObject(s).let { it.getInt("line") to it.getInt("col") }
    }.getOrNull()

    fun machines(src: String): List<MachineGraph> = runCatching {
        val a = JSONArray(TulpaNative.machines(src))
        (0 until a.length()).map { i ->
            val o = a.getJSONObject(i)
            val st = o.getJSONArray("states")
            val tr = o.getJSONArray("transitions")
            MachineGraph(
                o.getString("name"), o.getString("initial"),
                (0 until st.length()).map { st.getJSONObject(it).let { s -> MachineState(s.getString("name"), s.optInt("line"), s.optBoolean("enter"), s.optBoolean("exit")) } },
                (0 until tr.length()).map {
                    val t = tr.getJSONObject(it)
                    val ps = t.optJSONArray("params")
                    MachineTransition(t.getString("from"), t.getString("to"), t.getString("event"),
                        ps?.let { p -> (0 until p.length()).map { k -> p.getString(k) } } ?: emptyList(),
                        if (t.isNull("guard")) null else t.optString("guard"), t.optBoolean("internal"), t.optBoolean("global"), t.optInt("line"))
                },
                o.optString("mermaid"),
            )
        }
    }.getOrDefault(emptyList())

    fun parseVerify(json: String): Pair<List<VerifyResult>, String?> {
        if (json.trimStart().startsWith("{")) return emptyList<VerifyResult>() to JSONObject(json).optString("error")
        val a = JSONArray(json)
        return (0 until a.length()).map {
            val o = a.getJSONObject(it)
            val cex = o.optJSONArray("counterexample")
            VerifyResult(o.getString("name"), o.optBoolean("ok"), o.optInt("passed"), o.optInt("discarded"),
                cex?.let { c -> (0 until c.length()).map { k -> c.getString(k) } } ?: emptyList(),
                if (o.isNull("error")) null else o.optString("error"))
        } to null
    }

    fun modules(): Map<String, List<String>> = runCatching {
        val o = JSONObject(TulpaNative.modules())
        o.keys().asSequence().associateWith { k -> o.getJSONArray(k).let { a -> (0 until a.length()).map { a.getString(it) } } }
    }.getOrDefault(emptyMap())
}
