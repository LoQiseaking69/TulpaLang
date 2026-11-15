package dev.hermitech.tulpafield

import android.app.Application
import android.system.Os
import dev.hermitech.tulpafield.core.ReplManager
import dev.hermitech.tulpafield.core.RunManager
import dev.hermitech.tulpafield.core.SensorBridge
import dev.hermitech.tulpafield.core.SimManager
import dev.hermitech.tulpafield.core.Settings
import dev.hermitech.tulpafield.core.TulpaHost
import dev.hermitech.tulpafield.core.TulpaNative
import dev.hermitech.tulpafield.core.Workspace

class TulpaFieldApp : Application() {
    lateinit var workspace: Workspace; private set
    lateinit var sensors: SensorBridge; private set
    lateinit var runner: RunManager; private set
    lateinit var repl: ReplManager; private set
    lateinit var settings: Settings; private set
    lateinit var sim: SimManager; private set

    override fun onCreate() {
        super.onCreate()
        workspace = Workspace(this)
        workspace.installAssets("${BuildConfig.VERSION_NAME}+${BuildConfig.VERSION_CODE}+${packageManager.getPackageInfo(packageName, 0).lastUpdateTime}")
        // import "std/rtl" etc. resolve against the bundled library
        Os.setenv("TULPA_PATH", workspace.stdlib.absolutePath, true)
        TulpaNative.init(TulpaHost::class.java)
        TulpaHost.init(this)
        settings = Settings(this)
        sensors = SensorBridge(this)
        runner = RunManager(sensors)
        repl = ReplManager(this, sensors)
        sim = SimManager(workspace)
        TulpaHost.onSimNotify = { method, params -> runner.simNotify(method, params) }
    }
}
