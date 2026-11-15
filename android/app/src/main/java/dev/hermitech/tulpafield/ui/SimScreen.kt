@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)

package dev.hermitech.tulpafield.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.*
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.hermitech.tulpafield.AppViewModel
import dev.hermitech.tulpafield.Screen
import dev.hermitech.tulpafield.sim.OrbitCamera
import dev.hermitech.tulpafield.sim.SimHud
import dev.hermitech.tulpafield.sim.SimView
import kotlinx.coroutines.delay
import java.io.File

@Composable
fun SimScreen(vm: AppViewModel, menu: @Composable () -> Unit) {
    val st by vm.sim.state.collectAsStateWithLifecycle()
    val run by vm.runner.state.collectAsStateWithLifecycle()
    val ed by vm.editor.collectAsStateWithLifecycle()
    val ctx = LocalContext.current
    val camera = remember { OrbitCamera() }
    val hud = remember { SimHud() }
    var view by remember { mutableStateOf<SimView?>(null) }
    var picker by remember { mutableStateOf(false) }
    var infoSheet by remember { mutableStateOf(false) }
    var tick by remember { mutableIntStateOf(0) }
    LaunchedEffect(Unit) { while (true) { delay(200); tick++ } }
    LaunchedEffect(Unit) { vm.ensureSimModel() }
    DisposableEffect(Unit) { onDispose { view?.onPause() } }

    Scaffold(topBar = {
        TopAppBar(
            title = {
                Column(Modifier.clickableNoRipple { picker = true }) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text(st.model?.name ?: "Sim", maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Icon(Icons.Default.ArrowDropDown, null, tint = TF.Dim)
                    }
                    Text(
                        st.model?.let { "${it.bodies} bodies · ${it.actuators} actuators · MuJoCo ${st.version}" } ?: "MuJoCo ${st.version} · no model",
                        style = MaterialTheme.typography.labelSmall, color = TF.Dim, maxLines = 1,
                    )
                }
            },
            actions = {
                IconButton(onClick = { infoSheet = true }, enabled = st.model != null) { Icon(Icons.Default.Info, "Model info") }
                IconButton(onClick = { picker = true }) { Icon(Icons.Default.ViewInAr, "Choose model") }
                menu()
            },
        )
    }) { pad ->
        Box(Modifier.padding(pad).fillMaxSize().background(TF.Bg)) {
            AndroidView(
                factory = { c -> SimView(c, vm.sim, camera, hud).also { view = it } },
                modifier = Modifier.fillMaxSize(),
                onRelease = { it.onPause() },
            )
            // HUD
            val t = tick
            Column(Modifier.align(Alignment.TopStart).padding(12.dp)) {
                Surface(color = TF.Bg.copy(alpha = 0.72f), shape = RoundedCornerShape(10.dp)) {
                    Column(Modifier.padding(horizontal = 10.dp, vertical = 6.dp)) {
                        Text(if (t >= 0) "t = %.2f s".format(hud.time) else "", fontFamily = TF.Mono, fontSize = 13.sp, color = TF.Text)
                        Text(
                            when {
                                hud.clients > 0 -> "● co-sim: driven by ${run.title.ifBlank { "program" }}"
                                hud.playing -> "● free-running"
                                else -> "paused"
                            },
                            fontSize = 11.sp, color = if (hud.clients > 0) TF.Ok else if (hud.playing) TF.Cyan else TF.Dim,
                        )
                        Text("%.0f fps · %d geoms".format(hud.fps, hud.geoms), fontSize = 10.sp, color = TF.Dim)
                    }
                }
            }
            if (st.loading) CircularProgressIndicator(Modifier.align(Alignment.Center))
            st.error?.let {
                Surface(color = TF.Err.copy(alpha = 0.15f), shape = RoundedCornerShape(10.dp), modifier = Modifier.align(Alignment.Center).padding(24.dp)) {
                    Text(it, color = TF.Err, modifier = Modifier.padding(12.dp), fontFamily = TF.Mono, fontSize = 12.sp)
                }
            }
            if (st.model == null && !st.loading) {
                Column(Modifier.align(Alignment.Center).padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    Text("No model loaded", style = MaterialTheme.typography.titleMedium)
                    Text("Pick a MuJoCo model, or run a program that uses cosim::", color = TF.Dim, fontSize = 13.sp)
                    Spacer(Modifier.height(12.dp))
                    Button(onClick = { picker = true }) { Text("Choose model") }
                }
            }
            // running program strip
            if (run.running || run.console.isNotEmpty()) {
                Surface(color = TF.Bg.copy(alpha = 0.8f), shape = RoundedCornerShape(10.dp), modifier = Modifier.align(Alignment.BottomCenter).padding(start = 12.dp, end = 12.dp, bottom = 92.dp).fillMaxWidth()) {
                    Row(Modifier.padding(start = 10.dp, end = 4.dp, top = 2.dp, bottom = 2.dp), verticalAlignment = Alignment.CenterVertically) {
                        Text(run.console.lastOrNull { it.text.isNotBlank() }?.text ?: "", fontSize = 11.sp, fontFamily = TF.Mono, color = TF.Dim,
                            maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                        if (run.running) IconButton(onClick = { vm.runner.stop() }) { Icon(Icons.Default.Stop, "Stop", tint = TF.Err) }
                        else IconButton(onClick = { vm.go(Screen.Run) }) { Icon(Icons.Default.Terminal, "Console", tint = TF.Dim) }
                    }
                }
            }
            // controls
            Row(
                Modifier.align(Alignment.BottomCenter).padding(12.dp).background(TF.Bg.copy(alpha = 0.78f), RoundedCornerShape(16.dp))
                    .horizontalScroll(rememberScrollState()).padding(horizontal = 6.dp, vertical = 2.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                val driven = hud.clients > 0
                IconButton(onClick = { vm.sim.control(if (hud.playing) "pause" else "play") }, enabled = st.model != null && !driven) {
                    Icon(if (hud.playing) Icons.Default.Pause else Icons.Default.PlayArrow, if (hud.playing) "Pause" else "Play", tint = TF.Cyan)
                }
                IconButton(onClick = { vm.sim.control("reset") }, enabled = st.model != null && !driven) { Icon(Icons.Default.RestartAlt, "Reset") }
                IconButton(onClick = { vm.sim.control("push", (camera.azimuth + 180.0) % 360.0) }, enabled = st.model != null) { Icon(Icons.Default.PanTool, "Push") }
                IconButton(onClick = { view?.renderer?.reframe = true }) { Icon(Icons.Default.CenterFocusStrong, "Frame model") }
                FilterChip(st.realtime > 0.0, { vm.sim.setRealtime(if (st.realtime > 0.0) 0.0 else 1.0) },
                    label = { Text(if (st.realtime > 0.0) "real time" else "max speed", fontSize = 12.sp) })
                Spacer(Modifier.width(4.dp))
                FilledTonalButton(
                    onClick = { if (run.running) vm.runner.stop() else vm.runCurrent() },
                    enabled = run.running || ed.file != null,
                    contentPadding = PaddingValues(horizontal = 12.dp),
                ) {
                    Icon(if (run.running) Icons.Default.Stop else Icons.Default.PlayCircle, null, Modifier.size(18.dp))
                    Spacer(Modifier.width(4.dp)); Text(if (run.running) "Stop" else "Run ${ed.file?.name ?: ""}", maxLines = 1)
                }
            }
            Text("drag: orbit · two fingers: pan · pinch: zoom · double-tap: frame", color = TF.Dim.copy(alpha = 0.7f), fontSize = 10.sp,
                modifier = Modifier.align(Alignment.BottomCenter).padding(bottom = 72.dp))
        }
    }

    if (picker) ModalBottomSheet({ picker = false }) {
        val models = remember(ed.project) { vm.sim.models(ed.project) }
        val import = vm.importModelLauncher
        Text("Models", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 20.dp))
        LazyColumn(Modifier.padding(bottom = 24.dp)) {
            items(models, key = { it.path }) { f ->
                val bundled = f.parentFile == vm.workspace.models
                ListItem(
                    leadingContent = { Icon(if (bundled) Icons.Default.ViewInAr else Icons.Default.Description, null, tint = if (st.model?.path == f.path) TF.Cyan else TF.Violet) },
                    headlineContent = { Text(f.nameWithoutExtension) },
                    supportingContent = { Text(if (bundled) "bundled" else f.relativeTo(ed.project ?: f.parentFile!!).path, color = TF.Dim, fontSize = 12.sp) },
                    modifier = Modifier.clickableNoRipple { picker = false; vm.loadSimModel(f) },
                    colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow),
                )
            }
            item {
                TextButton(onClick = { picker = false; import() }, modifier = Modifier.padding(horizontal = 12.dp)) {
                    Icon(Icons.Default.FileOpen, null); Spacer(Modifier.width(8.dp)); Text("Import an MJCF model (.xml)…")
                }
                Text("Programs pick their model from hermisim.json (\"model\"), else the first MJCF in their folder, else the model open here.",
                    color = TF.Dim, fontSize = 12.sp, modifier = Modifier.padding(horizontal = 20.dp))
            }
        }
    }

    if (infoSheet) st.model?.let { m ->
        ModalBottomSheet({ infoSheet = false }) {
            Column(Modifier.padding(horizontal = 20.dp).padding(bottom = 28.dp)) {
                Text(m.name, style = MaterialTheme.typography.titleMedium)
                Text(File(m.path).name, color = TF.Dim, fontSize = 12.sp, fontFamily = TF.Mono)
                Spacer(Modifier.height(8.dp))
                Text("${m.bodies} bodies · ${m.joints} joints · ${m.actuators} actuators · ${m.sensors} sensors · %.1f kg · dt %.4f s".format(m.mass, m.info.optDouble("timestep")), fontSize = 13.sp)
                st.address?.let { Text("co-sim server: $it", color = TF.Cyan, fontFamily = TF.Mono, fontSize = 12.sp, modifier = Modifier.padding(top = 6.dp)) }
                Spacer(Modifier.height(10.dp))
                Text("Actuators", style = MaterialTheme.typography.labelLarge, color = TF.Violet)
                val acts = m.info.optJSONArray("actuators")
                FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    for (i in 0 until (acts?.length() ?: 0)) {
                        val a = acts!!.getJSONObject(i)
                        AssistChip(onClick = {}, label = { Text(a.optString("name"), fontFamily = TF.Mono, fontSize = 11.sp) })
                    }
                }
                Spacer(Modifier.height(10.dp))
                OutlinedButton(onClick = { infoSheet = false; vm.newCosimProject(m) }) { Icon(Icons.Default.CreateNewFolder, null); Spacer(Modifier.width(6.dp)); Text("New co-sim project for this model") }
            }
        }
    }
}

@Composable
fun Modifier.clickableNoRipple(onClick: () -> Unit): Modifier =
    this.clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onClick)
