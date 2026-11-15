package dev.hermitech.tulpafield.core

import android.content.Context
import android.net.Uri
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

data class Project(val name: String, val dir: File, val files: Int, val modified: Long)

data class Example(
    val id: String,
    val title: String,
    val category: String,
    val summary: String,
    val tags: List<String>,
    val virtualTime: Boolean,
    val dir: File,
    val args: List<String> = emptyList(),
) {
    val cosim: Boolean get() = runCatching { main.readText().contains("cosim::") }.getOrDefault(false)
    val main: File get() = File(dir, "main.rav")
    val expected: File get() = File(dir, "expected.txt")
}

data class GuideDoc(val id: String, val title: String, val section: String, val asset: String)

/**
 * On-device storage:
 *   files/projects/<name>/…   editable projects
 *   files/library/<id>/…      bundled examples (refreshed on every app update)
 *   files/stdlib/…            the std/ library (TULPA_PATH points here)
 */
class Workspace(private val context: Context) {
    val root = File(context.filesDir, "projects").apply { mkdirs() }
    val library = File(context.filesDir, "library")
    val stdlib = File(context.filesDir, "stdlib")
    /** Bundled MJCF models for co-simulation (examples reference them as ../../models/…). */
    val models = File(context.filesDir, "models")

    /** Copies bundled assets when the app version changes (or on first launch). */
    fun installAssets(version: String) {
        val marker = File(context.filesDir, ".assets-version")
        if (marker.exists() && marker.readText() == version && library.exists() && stdlib.exists() && models.exists()) return
        library.deleteRecursively(); stdlib.deleteRecursively(); models.deleteRecursively()
        copyAssetDir("examples", library)
        copyAssetDir("stdlib", stdlib)
        copyAssetDir("models", models)
        marker.writeText(version)
    }

    private fun copyAssetDir(asset: String, dest: File) {
        val am = context.assets
        val children = am.list(asset) ?: return
        if (children.isEmpty()) {
            dest.parentFile?.mkdirs()
            am.open(asset).use { input -> dest.outputStream().use { input.copyTo(it) } }
            return
        }
        dest.mkdirs()
        for (c in children) copyAssetDir("$asset/$c", File(dest, c))
    }

    // ------------------------------------------------------------------ projects

    fun projects(): List<Project> =
        (root.listFiles()?.filter { it.isDirectory } ?: emptyList())
            .map { d -> Project(d.name, d, d.walkTopDown().count { it.isFile }, d.walkTopDown().maxOf { it.lastModified() }) }
            .sortedByDescending { it.modified }

    fun files(project: File): List<File> =
        project.walkTopDown().filter { it.isFile && !it.name.startsWith(".") }.sortedBy { it.relativeTo(project).path }.toList()

    fun uniqueName(base: String): String {
        val clean = base.replace(Regex("[^A-Za-z0-9_\\- ]"), "").trim().ifEmpty { "project" }
        if (!File(root, clean).exists()) return clean
        var i = 2
        while (File(root, "$clean $i").exists()) i++
        return "$clean $i"
    }

    fun createProject(name: String, template: Template): File {
        val dir = File(root, uniqueName(name)).apply { mkdirs() }
        File(dir, "main.rav").writeText(template.source)
        return dir
    }

    fun copyExample(e: Example): File {
        val dir = File(root, uniqueName(e.title))
        e.dir.copyRecursively(dir)
        File(dir, "expected.txt").delete()
        return dir
    }

    fun newFile(project: File, name: String): File {
        val n = if (name.contains('.')) name else "$name.rav"
        val f = File(project, n)
        f.parentFile?.mkdirs()
        if (!f.exists()) f.writeText("# ${f.nameWithoutExtension}\n\n")
        return f
    }

    fun rename(f: File, newName: String): File {
        val target = File(f.parentFile, newName)
        if (!target.exists()) f.renameTo(target)
        return target
    }

    fun delete(f: File) { f.deleteRecursively() }

    fun duplicate(dir: File): File {
        val copy = File(root, uniqueName(dir.name + " copy"))
        dir.copyRecursively(copy)
        return copy
    }

    /** Imports a shared or opened file into an "Inbox" project; returns the new file. */
    fun import(uri: Uri, displayName: String?): File? {
        val text = context.contentResolver.openInputStream(uri)?.use { it.readBytes().decodeToString() } ?: return null
        return importText(text, displayName ?: uri.lastPathSegment?.substringAfterLast('/') ?: "imported.rav")
    }

    fun importText(text: String, name: String): File {
        val inbox = File(root, "Inbox").apply { mkdirs() }
        var n = name.substringAfterLast('/').ifEmpty { "imported.rav" }
        if (!n.endsWith(".rav") && !n.endsWith(".tlp")) n += ".rav"
        var f = File(inbox, n)
        var i = 2
        while (f.exists()) { f = File(inbox, n.substringBeforeLast('.') + " $i." + n.substringAfterLast('.')); i++ }
        f.writeText(text)
        return f
    }

    // ------------------------------------------------------------------ library & guide

    fun examples(): List<Example> {
        val catalog = runCatching {
            JSONArray(context.assets.open("catalog.json").bufferedReader().readText())
        }.getOrDefault(JSONArray())
        val known = HashSet<String>()
        val out = ArrayList<Example>()
        for (i in 0 until catalog.length()) {
            val o: JSONObject = catalog.getJSONObject(i)
            val id = o.getString("id")
            val dir = File(library, id)
            if (!File(dir, "main.rav").exists()) continue
            known += id
            val tags = o.optJSONArray("tags")?.let { a -> (0 until a.length()).map { a.getString(it) } } ?: emptyList()
            val args = o.optJSONArray("args")?.let { a -> (0 until a.length()).map { a.getString(it) } } ?: emptyList()
            out += Example(id, o.getString("title"), o.getString("category"), o.optString("summary"), tags, o.optBoolean("virtualTime"), dir, args)
        }
        // anything bundled but not catalogued still shows up
        library.listFiles()?.filter { it.isDirectory && it.name !in known && File(it, "main.rav").exists() }?.forEach { d ->
            val first = File(d, "main.rav").useLines { l -> l.firstOrNull { it.startsWith("#") }?.trimStart('#', ' ') ?: "" }
            out += Example(d.name, d.name.replace('_', ' ').replaceFirstChar { it.uppercase() }, "More", first, emptyList(), false, d)
        }
        return out
    }

    fun guide(): List<GuideDoc> {
        val list = runCatching { JSONArray(context.assets.open("guide.json").bufferedReader().readText()) }.getOrDefault(JSONArray())
        return (0 until list.length()).map {
            val o = list.getJSONObject(it)
            GuideDoc(o.getString("id"), o.getString("title"), o.getString("section"), o.getString("asset"))
        }.filter { d -> runCatching { context.assets.open(d.asset).close(); true }.getOrDefault(false) }
    }

    fun readAsset(path: String): String = context.assets.open(path).bufferedReader().readText()
}

/** Starting points for a new project. */
enum class Template(val title: String, val source: String) {
    Blank("Blank", "# New Tulpa program\n\nprint(\"hello from Tulpa Field\")\n"),
    Sensors("Phone sensor loop", """
        # Read the phone's sensors 10× per second; plot and gauge them.

        every!(100[ms], 300, {
            let a = phone::accel()
            tlm::plot("az", a[2].in("m/s^2"))
            phone::gauge("tilt", phone::tilt(), 0[deg], 90[deg])
        })
    """.trimIndent() + "\n"),
    Machine("State machine", """
        # A state machine with guarded transitions and a safety envelope.

        machine Light {
            state Off { on toggle => On }
            state On {
                enter { print("on") }
                on toggle => Off
                on overheat(t) if t > 80 => Fault
            }
            state Fault { on reset => Off }
        }

        let l = Light()
        l.toggle()
        l.overheat(95)
        print(l.state, l.history)
    """.trimIndent() + "\n"),
    Contract("Contract + check()", """
        # Specify a function, then let check() fuzz it for counterexamples.

        fn stopping_distance(v: [m/s], decel: [m/s^2]) -> [m]
            require v >= 0[m/s] and decel > 0[m/s^2]
            ensure result' >= 0[m]
        {
            v ** 2 / (2 * decel)
        }

        print(stopping_distance(13.9[m/s], 6.5[m/s^2]))
        print(check(stopping_distance))
    """.trimIndent() + "\n"),
}
