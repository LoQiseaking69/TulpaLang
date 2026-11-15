package dev.hermitech.tulpafield.core

/** Callbacks from a running Tulpa program (invoked on the program's own thread). */
interface RunListener {
    /** Batched program output, including `@@tlm {json}` telemetry lines. */
    fun onOutput(text: String)

    /** `input(prompt)`: block until the user answers; `null` = end of input. */
    fun onInput(prompt: String): String?
}

/**
 * JNI surface of `libtulpa_android.so` (android/native). The Rust side holds the whole Tulpa
 * runtime: interpreter, bytecode VM, units, contracts, state machines, std/ libraries and the
 * `phone::` host module.
 *
 * Threading: [run], [tests], [verify] execute on the calling thread, which needs a large stack
 * (see [TulpaThreads]). A REPL session must stay on the thread that created it.
 */
object TulpaNative {
    init {
        System.loadLibrary("tulpa_android")
    }

    @JvmStatic external fun init(host: Class<*>)
    @JvmStatic external fun version(): String

    @JvmStatic external fun newCancel(): Long
    @JvmStatic external fun cancel(handle: Long)
    @JvmStatic external fun freeCancel(handle: Long)

    /** Returns `null` on success, otherwise the error message ("interrupted" after [cancel]). */
    @JvmStatic external fun run(
        source: String, file: String, baseDir: String, args: Array<String>,
        virtualTime: Boolean, listener: RunListener, cancel: Long,
    ): String?

    /** `{passed, failed:[{name, error}]}` or `{error}`. */
    @JvmStatic external fun tests(source: String, file: String, baseDir: String, listener: RunListener, cancel: Long): String

    /** `[{name, ok, passed, discarded, counterexample, error}]` or `{error}`. */
    @JvmStatic external fun verify(source: String, baseDir: String, trials: Int, cancel: Long): String

    @JvmStatic external fun sessionNew(listener: RunListener, baseDir: String): Long
    /** `{kind: "more"|"value"|"error", text}` */
    @JvmStatic external fun sessionFeed(handle: Long, line: String): String
    @JvmStatic external fun sessionInterrupt(handle: Long)
    @JvmStatic external fun sessionCancelEntry(handle: Long)
    @JvmStatic external fun sessionNames(handle: Long): String
    @JvmStatic external fun sessionFree(handle: Long)

    @JvmStatic external fun diagnostics(source: String): String
    @JvmStatic external fun completion(source: String, line: Int, col: Int): String
    @JvmStatic external fun hover(source: String, line: Int, col: Int): String?
    @JvmStatic external fun symbols(source: String): String
    @JvmStatic external fun definition(source: String, line: Int, col: Int): String
    @JvmStatic external fun machines(source: String): String
    @JvmStatic external fun bytecode(source: String): String
    @JvmStatic external fun modules(): String

    @JvmStatic external fun pushSensor(kind: Int, values: FloatArray)
    @JvmStatic external fun pushLocation(lat: Double, lon: Double, alt: Double, accuracy: Double, speed: Double, bearing: Double)
    @JvmStatic external fun resetSensors()
}

/** Threads with the large native stack the Tulpa evaluator needs. */
object TulpaThreads {
    const val STACK: Long = 64L * 1024 * 1024

    fun start(name: String, body: () -> Unit): Thread =
        Thread(null, { body() }, name, STACK).apply { isDaemon = true; start() }

    /** A single long-lived thread (REPL, analysis) fed through a queue. */
    class Worker(name: String, stack: Long = STACK) {
        private val queue = java.util.concurrent.LinkedBlockingQueue<() -> Unit>()
        private val thread = Thread(null, {
            while (true) {
                val job = try { queue.take() } catch (_: InterruptedException) { return@Thread }
                try { job() } catch (t: Throwable) { android.util.Log.e("TulpaWorker", "job failed", t) }
            }
        }, name, stack).apply { isDaemon = true; start() }

        fun post(job: () -> Unit) { queue.put(job) }
        fun clearPending() { queue.clear() }
    }
}
