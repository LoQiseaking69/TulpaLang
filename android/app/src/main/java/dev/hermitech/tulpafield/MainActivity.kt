package dev.hermitech.tulpafield

import android.Manifest
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import android.view.WindowManager
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.Box
import androidx.compose.ui.Modifier
import androidx.core.content.FileProvider
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import dev.hermitech.tulpafield.core.TulpaHost
import dev.hermitech.tulpafield.ui.MainScreen
import dev.hermitech.tulpafield.ui.TF
import dev.hermitech.tulpafield.ui.TulpaTheme
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch

class MainActivity : ComponentActivity() {
    private val vm: AppViewModel by viewModels()

    private val openDoc = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) vm.import(uri, displayName(uri))
    }
    private val openModel = registerForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) vm.importModel(uri, displayName(uri))
    }
    private val locationPerm = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { r ->
        if (r.values.any { it }) vm.locationGranted() else vm.toast("Location denied — phone::location() will report no fix")
    }
    private val notifPerm = registerForActivityResult(ActivityResultContracts.RequestPermission()) { }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        vm.importModelLauncher = { openModel.launch(arrayOf("text/xml", "application/xml", "*/*")) }
        setContent {
            TulpaTheme {
                Box(Modifier.fillMaxSize().background(TF.Bg).safeDrawingPadding()) {
                    MainScreen(vm, onImport = { openDoc.launch(arrayOf("*/*")) }, onExit = { moveTaskToBack(true) })
                }
            }
        }
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                launch { vm.events.collect(::handle) }
                launch {
                    combine(vm.runner.state, TulpaHost.keepAwake, vm.settings.prefs) { r, k, p -> k || (r.running && p.keepScreenOn) }.collect { on ->
                        if (on) window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                        else window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                    }
                }
            }
        }
        if (savedInstanceState == null) handleIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleIntent(intent)
    }

    override fun onStop() {
        vm.autosave()
        super.onStop()
    }

    private fun handleIntent(i: Intent?) {
        when (i?.action) {
            Intent.ACTION_VIEW -> i.data?.let { vm.import(it, displayName(it)) }
            Intent.ACTION_SEND -> {
                @Suppress("DEPRECATION")
                val stream = if (Build.VERSION.SDK_INT >= 33) i.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java) else i.getParcelableExtra(Intent.EXTRA_STREAM)
                val text = i.getStringExtra(Intent.EXTRA_TEXT)
                when {
                    stream != null -> vm.import(stream, displayName(stream))
                    !text.isNullOrBlank() -> vm.importText(text, (i.getStringExtra(Intent.EXTRA_SUBJECT) ?: "shared") + ".rav")
                }
            }
        }
    }

    private fun displayName(uri: Uri): String? = runCatching {
        contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0) else null
        }
    }.getOrNull()

    private fun handle(e: UiEvent) {
        when (e) {
            is UiEvent.Toast -> Toast.makeText(this, e.text, Toast.LENGTH_SHORT).show()
            is UiEvent.Share -> {
                val uri = FileProvider.getUriForFile(this, "$packageName.files", e.file)
                startActivity(Intent.createChooser(Intent(Intent.ACTION_SEND).apply {
                    type = e.mime
                    putExtra(Intent.EXTRA_STREAM, uri)
                    putExtra(Intent.EXTRA_SUBJECT, e.file.name)
                    addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                }, "Share ${e.file.name}"))
            }
            is UiEvent.ShareText -> startActivity(Intent.createChooser(Intent(Intent.ACTION_SEND).apply {
                type = "text/plain"; putExtra(Intent.EXTRA_TEXT, e.text); putExtra(Intent.EXTRA_SUBJECT, e.title)
            }, e.title))
            UiEvent.RequestLocation -> locationPerm.launch(arrayOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION))
            UiEvent.RequestNotifications -> if (Build.VERSION.SDK_INT >= 33) notifPerm.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }
}
