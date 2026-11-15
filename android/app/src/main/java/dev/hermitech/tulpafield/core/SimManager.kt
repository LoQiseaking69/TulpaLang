package dev.hermitech.tulpafield.core

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import org.json.JSONObject
import java.io.File

data class SimModel(
    val handle: Long,
    val path: String,
    val name: String,
    val info: JSONObject,
    val center: FloatArray,
    val extent: Float,
    val meshes: Int,
) {
    val actuators get() = info.optJSONArray("actuators")?.length() ?: 0
    val bodies get() = (info.optJSONArray("bodies")?.length() ?: 1) - 1
    val joints get() = info.optJSONArray("joints")?.length() ?: 0
    val sensors get() = info.optJSONArray("sensors")?.length() ?: 0
    val mass get() = info.optDouble("total_mass", 0.0)
}

data class SimState(
    val model: SimModel? = null,
    val loading: Boolean = false,
    val error: String? = null,
    val address: String? = null,      // local co-sim server, once started
    val realtime: Double = 1.0,
    val version: String = "",
)

/**
 * Owns the on-device MuJoCo model shared by the 3D view and co-simulation. A program that
 * uses `cosim::` gets `TULPA_COSIM` pointing at this model's server (or at a remote HermiSim
 * when one is configured in Settings).
 */
class SimManager(private val workspace: Workspace) {
    private val _state = MutableStateFlow(SimState(version = runCatching { TulpaNative.simVersion() }.getOrDefault("")))
    val state: StateFlow<SimState> = _state

    /** Bumped whenever the loaded model changes, so the renderer re-uploads meshes. */
    @Volatile var generation = 0; private set

    val handle: Long get() = _state.value.model?.handle ?: 0L

    /** Bundled models (files/models) and MJCF files in [project]. */
    fun models(project: File?): List<File> {
        val bundled = workspace.models.listFiles { f -> f.extension == "xml" }?.sortedBy { it.name } ?: emptyList()
        val own = project?.walkTopDown()?.maxDepth(3)?.filter { it.isFile && it.extension == "xml" && isMjcf(it) }?.toList() ?: emptyList()
        return own + bundled
    }

    private fun isMjcf(f: File) = runCatching { f.bufferedReader().use { r -> r.readText().take(4000).contains("<mujoco") } }.getOrDefault(false)

    /**
     * The model a program in [dir] should co-simulate: `hermisim.json` "model" (HermiSim
     * workspaces), else the first MJCF in the folder, else the model already open.
     */
    fun modelFor(dir: File): File? {
        var d: File? = dir
        repeat(3) {
            val cfg = File(d ?: return@repeat, "hermisim.json")
            if (cfg.exists()) {
                val m = runCatching { JSONObject(cfg.readText()).optString("model") }.getOrDefault("")
                if (m.isNotBlank()) {
                    val f = File(cfg.parentFile, m).canonicalFile
                    if (f.exists()) return f
                    val byName = File(workspace.models, File(m).name)
                    if (byName.exists()) return byName
                }
            }
            d = d?.parentFile
        }
        dir.listFiles { f -> f.extension == "xml" }?.firstOrNull { isMjcf(it) }?.let { return it }
        return _state.value.model?.path?.let(::File)
    }

    @Synchronized
    fun load(file: File): String? {
        val cur = _state.value.model
        if (cur != null && File(cur.path).canonicalPath == file.canonicalPath) return null
        _state.value = _state.value.copy(loading = true, error = null)
        val r = JSONObject(TulpaNative.simLoad(file.path))
        if (r.has("error")) {
            _state.value = _state.value.copy(loading = false, error = r.getString("error"))
            return r.getString("error")
        }
        val h = r.getLong("handle")
        val info = JSONObject(TulpaNative.simInfo(h) ?: "{}")
        val st = info.optJSONArray("stat")
        val model = SimModel(
            h, file.path, info.optString("model").ifBlank { file.nameWithoutExtension }, info,
            floatArrayOf(st?.optDouble(0)?.toFloat() ?: 0f, st?.optDouble(1)?.toFloat() ?: 0f, st?.optDouble(2)?.toFloat() ?: 0f),
            st?.optDouble(3)?.toFloat() ?: 1f, info.optInt("nmesh"),
        )
        cur?.let { TulpaNative.simFree(it.handle) }
        TulpaNative.simControl(h, "realtime", _state.value.realtime)
        generation++
        _state.value = _state.value.copy(model = model, loading = false, address = null, error = null)
        return null
    }

    fun unload() {
        _state.value.model?.let { TulpaNative.simFree(it.handle) }
        generation++
        _state.value = _state.value.copy(model = null, address = null)
    }

    fun control(cmd: String, arg: Double = 0.0): String = if (handle == 0L) "no model loaded" else TulpaNative.simControl(handle, cmd, arg)

    fun setRealtime(r: Double) {
        _state.value = _state.value.copy(realtime = r)
        control("realtime", r)
    }

    /**
     * Called before running [source] from [dir]. Returns the co-sim address the program will
     * use (also exported as TULPA_COSIM), or an error. Programs without `cosim::` clear it.
     */
    fun prepareRun(source: String, dir: File, remote: String?): Result<String?> {
        if (!source.contains("cosim::")) { TulpaNative.setEnv("TULPA_COSIM", null); return Result.success(null) }
        if (!remote.isNullOrBlank()) {
            TulpaNative.setEnv("TULPA_COSIM", remote.trim())
            return Result.success(remote.trim())
        }
        val model = modelFor(dir) ?: File(workspace.models, "starter_model.xml").takeIf { it.exists() }
            ?: return Result.failure(IllegalStateException("cosim: no model — open one in the Sim tab or add a .xml MJCF to the project"))
        load(model)?.let { return Result.failure(IllegalStateException("cosim: cannot load ${model.name}: $it")) }
        control("pause")
        control("reset")
        val addr = TulpaNative.simServe(handle, false)
        if (addr.startsWith("error")) return Result.failure(IllegalStateException(addr))
        _state.value = _state.value.copy(address = addr)
        TulpaNative.setEnv("TULPA_COSIM", addr)
        return Result.success(addr)
    }
}
