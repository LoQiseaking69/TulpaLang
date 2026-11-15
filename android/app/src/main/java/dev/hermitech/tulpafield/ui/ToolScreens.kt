@file:OptIn(ExperimentalMaterial3Api::class)

package dev.hermitech.tulpafield.ui

import android.Manifest
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.drawText
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.hermitech.tulpafield.AppViewModel
import dev.hermitech.tulpafield.BuildConfig
import dev.hermitech.tulpafield.core.MachineGraph
import dev.hermitech.tulpafield.core.SensorKind
import dev.hermitech.tulpafield.core.TulpaNative
import kotlin.math.PI
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.hypot
import kotlin.math.sin

@Composable
private fun ToolBar(title: String, subtitle: String?, vm: AppViewModel, actions: @Composable RowScope.() -> Unit = {}) {
    TopAppBar(
        title = { Column { Text(title); if (subtitle != null) Text(subtitle, style = MaterialTheme.typography.labelSmall, color = TF.Dim, maxLines = 1, overflow = TextOverflow.Ellipsis) } },
        navigationIcon = { IconButton(onClick = { vm.back() }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") } },
        actions = actions,
    )
}

// ---------------------------------------------------------------------------------- sensors

@Composable
fun SensorsScreen(vm: AppViewModel) {
    DisposableEffect(Unit) { vm.sensors.acquire(); onDispose { vm.sensors.release() } }
    val latest by vm.sensors.latest.collectAsStateWithLifecycle()
    var catalog by remember { mutableStateOf(vm.sensors.catalog()) }
    var locOn by remember { mutableStateOf(vm.sensors.locationEnabled) }
    val locPerm = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { r ->
        if (r.values.any { it }) { vm.sensors.setLocationEnabled(true); locOn = true; catalog = vm.sensors.catalog() }
    }
    val actPerm = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { ok ->
        if (ok) { vm.sensors.release(); vm.sensors.acquire(); catalog = vm.sensors.catalog() }
    }
    Scaffold(topBar = { ToolBar("Sensors", "live readings · what phone:: can see", vm) }) { pad ->
        LazyColumn(Modifier.padding(pad), contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            item {
                Card(colors = CardDefaults.cardColors(containerColor = TF.Surface2)) {
                    Column(Modifier.padding(14.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Icon(Icons.Default.LocationOn, null, tint = TF.Cyan)
                            Spacer(Modifier.width(10.dp))
                            Column(Modifier.weight(1f)) { Text("Location for phone::location()"); Text("GPS stays off unless enabled here or a program asks for it.", color = TF.Dim, fontSize = 12.sp) }
                            Switch(locOn, { on ->
                                if (on && !vm.sensors.hasLocationPermission()) locPerm.launch(arrayOf(Manifest.permission.ACCESS_FINE_LOCATION, Manifest.permission.ACCESS_COARSE_LOCATION))
                                else { vm.sensors.setLocationEnabled(on); locOn = on }
                            })
                        }
                        if (Build.VERSION.SDK_INT >= 29) TextButton(onClick = { actPerm.launch(Manifest.permission.ACTIVITY_RECOGNITION) }) { Text("Allow step counter") }
                    }
                }
            }
            items(catalog, key = { it.kind }) { s ->
                val r = latest[s.kind]
                Card(colors = CardDefaults.cardColors(containerColor = TF.Surface)) {
                    Column(Modifier.padding(14.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Box(Modifier.size(8.dp).background(if (!s.available) TF.Dim else if (r != null) TF.Ok else TF.Warn, RoundedCornerShape(4.dp)))
                            Spacer(Modifier.width(8.dp))
                            Text(s.title, style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f))
                            Text(s.unit, color = TF.Unit, fontSize = 12.sp)
                        }
                        Text(s.call, fontFamily = TF.Mono, color = TF.Cyan, fontSize = 12.sp, modifier = Modifier.padding(top = 2.dp))
                        val value = when {
                            !s.available -> s.vendor
                            r == null -> "waiting for data…"
                            s.kind == SensorKind.ORIENTATION -> r.values.take(3).joinToString("   ") { "%.1f°".format(Math.toDegrees(it.toDouble())) }
                            s.kind == SensorKind.BATTERY -> "%.0f%%".format(r.values[0] * 100) + if (r.values.getOrElse(1) { 0f } > 0f) " · charging" else ""
                            s.kind == SensorKind.LOCATION -> "%.5f, %.5f  ±%.0f m".format(r.values[0], r.values[1], r.values[3])
                            else -> r.values.take(3).joinToString("   ") { fmt(it) }
                        }
                        Text(value, fontFamily = TF.Mono, fontSize = 18.sp, modifier = Modifier.padding(top = 6.dp), color = if (s.available) TF.Text else TF.Dim)
                        if (s.available) Text(s.vendor, color = TF.Dim, fontSize = 11.sp)
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------- machines

@Composable
fun MachinesScreen(vm: AppViewModel) {
    val graphs by vm.machines.collectAsStateWithLifecycle()
    val ed by vm.editor.collectAsStateWithLifecycle()
    val clip = LocalClipboardManager.current
    Scaffold(topBar = { ToolBar("Machines", ed.file?.name, vm) { IconButton(onClick = { vm.refreshMachines() }) { Icon(Icons.Default.Refresh, "Refresh") } } }) { pad ->
        if (graphs.isEmpty()) {
            EmptyState(Modifier.padding(pad), Icons.Default.Hub, "No machines here", "This file doesn't declare a machine. Try the State machine template or the Shake alarm example.")
            return@Scaffold
        }
        LazyColumn(Modifier.padding(pad), contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            items(graphs, key = { it.name }) { g ->
                Card(colors = CardDefaults.cardColors(containerColor = TF.Surface)) {
                    Column(Modifier.padding(12.dp)) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text("machine ${g.name}", fontFamily = TF.Mono, fontWeight = FontWeight.Bold, color = TF.Type, modifier = Modifier.weight(1f))
                            Text("${g.states.size} states · ${g.transitions.size} transitions", color = TF.Dim, fontSize = 12.sp)
                        }
                        MachineCanvas(g) { line -> vm.reveal(line - 1) }
                        Text("Tap a state to jump to it.", color = TF.Dim, fontSize = 11.sp)
                        Spacer(Modifier.height(8.dp))
                        g.transitions.forEach { t ->
                            Row(Modifier.fillMaxWidth().clickable { vm.reveal(t.line - 1) }.padding(vertical = 5.dp), verticalAlignment = Alignment.CenterVertically) {
                                Text(if (t.global) "∀" else t.from, fontFamily = TF.Mono, fontSize = 13.sp, color = TF.Type, modifier = Modifier.widthIn(min = 70.dp))
                                Text(" —${t.event}${if (t.params.isNotEmpty()) "(" + t.params.joinToString() + ")" else ""}→ ", fontFamily = TF.Mono, fontSize = 13.sp, color = TF.Cyan)
                                Text(if (t.internal) "(internal)" else t.to, fontFamily = TF.Mono, fontSize = 13.sp, color = if (t.internal) TF.Dim else TF.Type)
                                Spacer(Modifier.weight(1f))
                                Text("${t.line}", color = TF.Dim, fontSize = 11.sp)
                            }
                            if (t.guard != null) Text("    if ${t.guard}", fontFamily = TF.Mono, fontSize = 12.sp, color = TF.Warn)
                        }
                        Row {
                            TextButton(onClick = { clip.setText(AnnotatedString(g.mermaid)) }) { Icon(Icons.Default.ContentCopy, null, Modifier.size(16.dp)); Spacer(Modifier.width(6.dp)); Text("Copy Mermaid") }
                            TextButton(onClick = { vm.share(g.mermaid, "${g.name}.mmd") }) { Icon(Icons.Default.Share, null, Modifier.size(16.dp)); Spacer(Modifier.width(6.dp)); Text("Share") }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun MachineCanvas(g: MachineGraph, onTap: (Int) -> Unit) {
    val measurer = rememberTextMeasurer()
    val n = g.states.size
    val height = if (n <= 2) 180.dp else if (n <= 4) 260.dp else 320.dp
    val positions = remember(g) { HashMap<String, Offset>() }
    Canvas(
        Modifier.fillMaxWidth().height(height).padding(vertical = 8.dp).pointerInput(g) {
            detectTapGestures { p ->
                val hit = positions.minByOrNull { (_, o) -> hypot(o.x - p.x, o.y - p.y) }
                if (hit != null && hypot(hit.value.x - p.x, hit.value.y - p.y) < 120f) g.states.firstOrNull { it.name == hit.key }?.let { onTap(it.line) }
            }
        },
    ) {
        val w = size.width; val h = size.height
        val cx = w / 2; val cy = h / 2
        val rx = (w / 2 - 110f).coerceAtLeast(60f); val ry = (h / 2 - 50f).coerceAtLeast(40f)
        val pos = g.states.mapIndexed { i, s ->
            val a = if (n == 1) 0.0 else -PI / 2 + 2 * PI * i / n + if (n == 2) PI / 2 else 0.0
            s.name to if (n == 1) Offset(cx, cy) else Offset(cx + (rx * cos(a)).toFloat(), cy + (ry * sin(a)).toFloat())
        }.toMap()
        positions.clear(); positions.putAll(pos)
        val nodeR = 46f
        // edges
        val pairCount = HashMap<Pair<String, String>, Int>()
        for (t in g.transitions) {
            if (t.global) continue
            val a = pos[t.from] ?: continue
            if (t.internal || t.from == t.to) {
                drawCircle(TF.Cyan.copy(alpha = 0.6f), 18f, Offset(a.x, a.y - nodeR - 12f), style = Stroke(2f))
                drawText(measurer, t.event, Offset(a.x + 14f, a.y - nodeR - 40f), TextStyle(color = TF.Cyan, fontSize = 10.sp, fontFamily = TF.Mono))
                continue
            }
            val b = pos[t.to] ?: continue
            val k = pairCount.merge(t.from to t.to, 1) { x, y -> x + y }!! - 1
            val reverse = g.transitions.any { it.from == t.to && it.to == t.from && !it.global }
            val bend = (if (reverse) 40f else 0f) + k * 22f
            arrow(a, b, nodeR, bend, if (t.guard != null) TF.Warn else TF.Cyan)
            val mid = Offset((a.x + b.x) / 2, (a.y + b.y) / 2)
            val ang = atan2(b.y - a.y, b.x - a.x)
            val lp = Offset(mid.x - sin(ang) * (bend + 10f), mid.y + cos(ang) * (bend + 10f))
            val label = t.event + if (t.guard != null) " ?" else ""
            val lay = measurer.measure(label, TextStyle(fontSize = 10.sp, fontFamily = TF.Mono))
            drawRect(TF.Surface, Offset(lp.x - lay.size.width / 2 - 3, lp.y - lay.size.height / 2), androidx.compose.ui.geometry.Size(lay.size.width + 6f, lay.size.height.toFloat()))
            drawText(lay, color = if (t.guard != null) TF.Warn else TF.Cyan, topLeft = Offset(lp.x - lay.size.width / 2, lp.y - lay.size.height / 2))
        }
        // nodes
        for (s in g.states) {
            val c = pos.getValue(s.name)
            drawCircle(TF.Surface2, nodeR, c)
            drawCircle(if (s.name == g.initial) TF.Violet else TF.Line, nodeR, c, style = Stroke(3f))
            if (s.name == g.initial) drawCircle(TF.Violet, nodeR + 7f, c, style = Stroke(2f))
            val label = (if (s.enter) "▸" else "") + s.name + (if (s.exit) "◂" else "")
            val lay = measurer.measure(label, TextStyle(fontSize = 11.sp, fontWeight = FontWeight.SemiBold, color = TF.Text))
            drawText(lay, topLeft = Offset(c.x - lay.size.width / 2, c.y - lay.size.height / 2))
        }
        val globals = g.transitions.filter { it.global }
        if (globals.isNotEmpty()) drawText(measurer, "∀ " + globals.joinToString("  ") { "${it.event}→${it.to}" }, Offset(8f, h - 36f),
            TextStyle(color = TF.Macro, fontSize = 10.sp, fontFamily = TF.Mono))
    }
}

private fun DrawScope.arrow(a: Offset, b: Offset, r: Float, bend: Float, color: Color) {
    val ang = atan2(b.y - a.y, b.x - a.x)
    val nx = -sin(ang); val ny = cos(ang)
    val start = Offset(a.x + cos(ang) * r + nx * bend * 0.3f, a.y + sin(ang) * r + ny * bend * 0.3f)
    val end = Offset(b.x - cos(ang) * (r + 6f) + nx * bend * 0.3f, b.y - sin(ang) * (r + 6f) + ny * bend * 0.3f)
    val ctrl = Offset((a.x + b.x) / 2 + nx * bend * 1.6f, (a.y + b.y) / 2 + ny * bend * 1.6f)
    val p = Path().apply { moveTo(start.x, start.y); quadraticTo(ctrl.x, ctrl.y, end.x, end.y) }
    drawPath(p, color.copy(alpha = 0.8f), style = Stroke(2.5f))
    val ea = atan2(end.y - ctrl.y, end.x - ctrl.x)
    val head = Path().apply {
        moveTo(end.x, end.y)
        lineTo(end.x - cos(ea - 0.4f) * 16f, end.y - sin(ea - 0.4f) * 16f)
        lineTo(end.x - cos(ea + 0.4f) * 16f, end.y - sin(ea + 0.4f) * 16f)
        close()
    }
    drawPath(head, color)
}

// ---------------------------------------------------------------------------------- verify

@Composable
fun VerifyScreen(vm: AppViewModel) {
    val st by vm.verify.collectAsStateWithLifecycle()
    val ed by vm.editor.collectAsStateWithLifecycle()
    val prefs by vm.settings.prefs.collectAsStateWithLifecycle()
    var tab by remember { mutableIntStateOf(0) }
    Scaffold(topBar = {
        Column {
            ToolBar("Verify", ed.file?.name, vm)
            TabRow(tab) {
                Tab(tab == 0, { tab = 0 }, text = { Text("Contracts") })
                Tab(tab == 1, { tab = 1 }, text = { Text("Tests") })
            }
        }
    }) { pad ->
        Column(Modifier.padding(pad).fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            if (ed.file == null) { Text("Open a file first.", color = TF.Dim); return@Column }
            if (tab == 0) {
                Text("Fuzzes every function with an ensure clause or a return type using ${prefs.trials} trials per function, shrinking failures to a minimal counterexample.", color = TF.Dim, fontSize = 13.sp)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (st.running) {
                        CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 3.dp); Spacer(Modifier.width(12.dp))
                        OutlinedButton(onClick = { vm.stopVerify() }) { Text("Stop") }
                    } else Button(onClick = { vm.runVerify() }) { Icon(Icons.Default.Verified, null); Spacer(Modifier.width(6.dp)); Text("Check contracts") }
                }
                st.error?.let { Text(it, color = TF.Err, fontFamily = TF.Mono, fontSize = 13.sp) }
                st.results?.let { rs ->
                    if (rs.isEmpty()) Text("No contracted functions in this file. Add `require`/`ensure` clauses or a return type.", color = TF.Dim)
                    val failed = rs.count { !it.ok }
                    if (rs.isNotEmpty()) Text(if (failed == 0) "All ${rs.size} functions hold ✓" else "$failed of ${rs.size} functions fail", color = if (failed == 0) TF.Ok else TF.Err, fontWeight = FontWeight.SemiBold)
                    rs.forEach { r ->
                        Card(colors = CardDefaults.cardColors(containerColor = TF.Surface)) {
                            Column(Modifier.padding(12.dp)) {
                                Row(verticalAlignment = Alignment.CenterVertically) {
                                    Icon(if (r.ok) Icons.Default.CheckCircle else Icons.Default.Cancel, null, tint = if (r.ok) TF.Ok else TF.Err)
                                    Spacer(Modifier.width(8.dp))
                                    Text(r.name, fontFamily = TF.Mono, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
                                    Text("${r.passed} passed · ${r.discarded} discarded", color = TF.Dim, fontSize = 12.sp)
                                }
                                if (!r.ok) {
                                    if (r.counterexample.isNotEmpty()) Text("counterexample: ${r.name}(${r.counterexample.joinToString(", ")})", fontFamily = TF.Mono, fontSize = 13.sp, color = TF.Warn, modifier = Modifier.padding(top = 6.dp))
                                    r.error?.let { Text(it, fontFamily = TF.Mono, fontSize = 12.sp, color = TF.Err, modifier = Modifier.padding(top = 4.dp)) }
                                    if (r.counterexample.isNotEmpty()) TextButton(onClick = { vm.tryCounterexample(r) }) { Icon(Icons.Default.Terminal, null, Modifier.size(16.dp)); Spacer(Modifier.width(6.dp)); Text("Try in REPL") }
                                }
                            }
                        }
                    }
                }
            } else {
                Text("Runs every define_test! block in the file.", color = TF.Dim, fontSize = 13.sp)
                if (st.testsRunning) CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 3.dp)
                else Button(onClick = { vm.runTests() }) { Icon(Icons.Default.Science, null); Spacer(Modifier.width(6.dp)); Text("Run tests") }
                st.tests?.let { t ->
                    if (t.error != null) Text(t.error, color = TF.Err, fontFamily = TF.Mono, fontSize = 13.sp)
                    else {
                        Text("${t.passed} passed, ${t.failed.size} failed", color = if (t.failed.isEmpty()) TF.Ok else TF.Err, fontWeight = FontWeight.SemiBold)
                        if (t.passed == 0 && t.failed.isEmpty()) Text("No tests found. Add one with define_test!(\"name\", { assert!(…) }).", color = TF.Dim)
                        t.failed.forEach { (n, e) ->
                            Card(colors = CardDefaults.cardColors(containerColor = TF.Surface)) {
                                Column(Modifier.padding(12.dp)) { Text("✗ $n", fontFamily = TF.Mono, color = TF.Err); Text(e, fontFamily = TF.Mono, fontSize = 12.sp, color = TF.Dim) }
                            }
                        }
                    }
                    if (t.output.isNotBlank()) {
                        Text("Output", style = MaterialTheme.typography.labelLarge, color = TF.Dim)
                        Surface(color = TF.Bg, shape = RoundedCornerShape(8.dp)) { Text(t.output, style = monoStyle(12), modifier = Modifier.padding(10.dp)) }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------- bytecode

@Composable
fun BytecodeScreen(vm: AppViewModel) {
    val code by vm.bytecode.collectAsStateWithLifecycle()
    val ed by vm.editor.collectAsStateWithLifecycle()
    Scaffold(topBar = { ToolBar("Bytecode", ed.file?.name, vm) { IconButton(onClick = { vm.refreshBytecode() }) { Icon(Icons.Default.Refresh, "Refresh") } } }) { pad ->
        Column(Modifier.padding(pad).fillMaxSize()) {
            Text("Each function is compiled to stack bytecode on first call; functions using features outside the bytecode tier stay interpreted.",
                color = TF.Dim, fontSize = 12.sp, modifier = Modifier.padding(16.dp))
            SelectionContainer(Modifier.weight(1f)) {
                Text(code, style = monoStyle(12), modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).horizontalScroll(rememberScrollState()).padding(16.dp))
            }
        }
    }
}

// ---------------------------------------------------------------------------------- settings

@Composable
fun SettingsScreen(vm: AppViewModel) {
    val p by vm.settings.prefs.collectAsStateWithLifecycle()
    val notif = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { }
    Scaffold(topBar = { ToolBar("Settings", null, vm) }) { pad ->
        Column(Modifier.padding(pad).fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text("Editor", style = MaterialTheme.typography.titleSmall, color = TF.Violet)
            Text("Font size: ${p.fontSize} sp")
            Slider(p.fontSize.toFloat(), { v -> vm.settings.update { it.copy(fontSize = v.toInt()) } }, valueRange = 10f..22f, steps = 11)
            Text("let speed = 0.8[m/s]", style = monoStyle(p.fontSize).copy(color = TF.Dim))
            SwitchRow("Line numbers", p.lineNumbers) { v -> vm.settings.update { it.copy(lineNumbers = v) } }
            SwitchRow("Pair brackets and quotes", p.pairBrackets) { v -> vm.settings.update { it.copy(pairBrackets = v) } }
            SwitchRow("Save automatically", p.autosave) { v -> vm.settings.update { it.copy(autosave = v) } }
            HorizontalDivider(Modifier.padding(vertical = 8.dp))
            Text("Running", style = MaterialTheme.typography.titleSmall, color = TF.Violet)
            SwitchRow("Keep screen on while a program runs", p.keepScreenOn) { v -> vm.settings.update { it.copy(keepScreenOn = v) } }
            Text("Contract trials per function: ${p.trials}")
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf(100, 300, 1000, 3000).forEach { n -> FilterChip(p.trials == n, { vm.settings.update { it.copy(trials = n) } }, label = { Text("$n") }) }
            }
            if (Build.VERSION.SDK_INT >= 33) TextButton(onClick = { notif.launch(Manifest.permission.POST_NOTIFICATIONS) }) { Text("Allow notifications for phone::notify") }
            HorizontalDivider(Modifier.padding(vertical = 8.dp))
            Text("About", style = MaterialTheme.typography.titleSmall, color = TF.Violet)
            Text("Tulpa Field ${BuildConfig.VERSION_NAME}", fontWeight = FontWeight.SemiBold)
            Text("TulpaLang runtime ${runCatching { TulpaNative.version() }.getOrDefault("?")} · interpreter, bytecode VM, units, contracts, machines, signals, std/ libraries", color = TF.Dim, fontSize = 13.sp)
            val mods = remember { vm.modules.keys.sorted().joinToString(" · ") }
            Text("Modules: $mods", color = TF.Dim, fontSize = 12.sp, fontFamily = TF.Mono)
            Text("Part of the HermiSim family. Everything runs on-device; no network access is used.", color = TF.Dim, fontSize = 13.sp)
        }
    }
}

@Composable
private fun SwitchRow(label: String, value: Boolean, onChange: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().clickable { onChange(!value) }.padding(vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
        Text(label, Modifier.weight(1f)); Switch(value, onChange)
    }
}
