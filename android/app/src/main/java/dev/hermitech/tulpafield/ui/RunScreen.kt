@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)

package dev.hermitech.tulpafield.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.hermitech.tulpafield.AppViewModel
import dev.hermitech.tulpafield.Screen
import dev.hermitech.tulpafield.core.Gauge
import dev.hermitech.tulpafield.core.LineKind
import dev.hermitech.tulpafield.core.PlotSeries
import dev.hermitech.tulpafield.core.TulpaHost
import kotlinx.coroutines.delay
import java.io.File

@Composable
fun RunScreen(vm: AppViewModel, menu: @Composable () -> Unit) {
    val st by vm.runner.state.collectAsStateWithLifecycle()
    val gauges by TulpaHost.gauges.collectAsStateWithLifecycle()
    val vt by vm.virtualTime.collectAsStateWithLifecycle()
    val args by vm.runArgs.collectAsStateWithLifecycle()
    var argsDialog by remember { mutableStateOf(false) }
    var showPlots by remember { mutableStateOf(true) }
    var runMenu by remember { mutableStateOf(false) }
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(st.running) { while (st.running) { now = System.currentTimeMillis(); delay(200) } }

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Column {
                        Text(if (st.title.isEmpty()) "Run" else st.title, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        val secs = ((if (st.running) now else st.finishedAt) - st.startedAt).coerceAtLeast(0) / 1000.0
                        val label = when {
                            st.running -> "running · %.1f s".format(secs)
                            st.ok == true -> "finished · %.2f s".format(secs)
                            st.ok == false -> if (st.exit?.contains("interrupted") == true) "stopped" else "failed"
                            else -> "idle"
                        }
                        Text(label, style = MaterialTheme.typography.labelSmall, color = when { st.running -> TF.Cyan; st.ok == false -> TF.Err; st.ok == true -> TF.Ok; else -> TF.Dim })
                    }
                },
                actions = {
                    if (st.running) {
                        FilledTonalButton(onClick = { vm.runner.stop() }, colors = ButtonDefaults.filledTonalButtonColors(containerColor = TF.Err.copy(alpha = 0.2f), contentColor = TF.Err)) {
                            Icon(Icons.Default.Stop, null); Spacer(Modifier.width(4.dp)); Text("Stop")
                        }
                    } else {
                        FilledTonalButton(onClick = { vm.runCurrent() }, enabled = vm.editor.value.file != null) {
                            Icon(Icons.Default.PlayArrow, null); Spacer(Modifier.width(4.dp)); Text("Run")
                        }
                    }
                    Box {
                        IconButton(onClick = { runMenu = true }) { Icon(Icons.Default.Tune, "Run options") }
                        DropdownMenu(runMenu, { runMenu = false }) {
                            DropdownMenuItem(text = { Text("Virtual time") }, leadingIcon = { Checkbox(vt, null) }, onClick = { vm.virtualTime.value = !vt })
                            DropdownMenuItem(text = { Text(if (args.isBlank()) "Run with arguments…" else "Arguments: $args") }, leadingIcon = { Icon(Icons.Default.Input, null) }, onClick = { runMenu = false; argsDialog = true })
                            DropdownMenuItem(text = { Text("Clear console") }, leadingIcon = { Icon(Icons.Default.ClearAll, null) }, enabled = !st.running, onClick = { runMenu = false; vm.runner.clear() })
                            DropdownMenuItem(text = { Text("Share output") }, leadingIcon = { Icon(Icons.Default.Share, null) }, onClick = {
                                runMenu = false; vm.share(st.console.joinToString("\n") { it.text }, "Tulpa output")
                            })
                            DropdownMenuItem(text = { Text("Sensors") }, leadingIcon = { Icon(Icons.Default.Sensors, null) }, onClick = { runMenu = false; vm.go(Screen.Sensors) })
                        }
                    }
                    menu()
                },
            )
        },
    ) { pad ->
        Column(Modifier.padding(pad).fillMaxSize().imePadding()) {
            if (st.status != null) {
                Text(st.status!!, color = TF.Cyan, modifier = Modifier.fillMaxWidth().background(TF.Surface2).padding(horizontal = 16.dp, vertical = 6.dp), fontSize = 13.sp)
            }
            if (gauges.isNotEmpty()) {
                Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    gauges.values.forEach { GaugeCard(it) }
                }
            }
            if (st.plots.isNotEmpty()) {
                Row(Modifier.fillMaxWidth().clickable { showPlots = !showPlots }.padding(horizontal = 16.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Default.ShowChart, null, tint = TF.Cyan, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.width(6.dp))
                    Text("Live plots (${st.plots.size})", style = MaterialTheme.typography.labelLarge, color = TF.Dim, modifier = Modifier.weight(1f))
                    Icon(if (showPlots) Icons.Default.ExpandLess else Icons.Default.ExpandMore, null, tint = TF.Dim)
                }
                if (showPlots) {
                    Column(Modifier.heightIn(max = 300.dp).verticalScrollIfNeeded().padding(horizontal = 12.dp)) {
                        st.plots.values.forEachIndexed { i, s -> PlotCard(s, TF.series[i % TF.series.size], st.events.map { it.first }); Spacer(Modifier.height(8.dp)) }
                    }
                }
            }
            if (st.artifacts.isNotEmpty()) {
                Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    st.artifacts.forEach { a ->
                        val f = File(a.path)
                        AssistChip(onClick = { vm.shareFile(f) }, label = { Text(f.name) }, leadingIcon = { Icon(Icons.Default.AttachFile, null, Modifier.size(16.dp)) },
                            trailingIcon = { Icon(Icons.Default.Share, null, Modifier.size(14.dp)) })
                    }
                }
            }
            Console(vm, Modifier.weight(1f))
            if (st.prompt != null) InputRow(st.prompt!!, onSend = { vm.runner.answer(it) }, onEof = { vm.runner.answer(null) })
        }
    }

    if (argsDialog) TextPrompt("Program arguments", "space-separated, read with args()", args, "Set", { argsDialog = false }) { vm.runArgs.value = it; argsDialog = false }
}

@Composable
private fun Modifier.verticalScrollIfNeeded() = this.then(Modifier.verticalScroll(rememberScrollState()))

@Composable
private fun Console(vm: AppViewModel, modifier: Modifier) {
    val st by vm.runner.state.collectAsStateWithLifecycle()
    val prefs by vm.settings.prefs.collectAsStateWithLifecycle()
    val list = rememberLazyListState()
    var follow by remember { mutableStateOf(true) }
    LaunchedEffect(st.console.size) { if (follow && st.console.isNotEmpty()) list.scrollToItem(st.console.size - 1) }
    LaunchedEffect(list.isScrollInProgress) {
        if (list.isScrollInProgress) {
            val info = list.layoutInfo
            follow = (info.visibleItemsInfo.lastOrNull()?.index ?: 0) >= info.totalItemsCount - 2
        }
    }
    if (st.console.isEmpty()) {
        EmptyState(modifier, Icons.Default.Terminal, "Nothing has run yet",
            "Run the open file from here or from the editor — or try an example from the Library.",
            "Library" to { vm.go(Screen.Learn) })
        return
    }
    val style = monoStyle((prefs.fontSize - 1).coerceAtLeast(10))
    SelectionContainer(modifier.fillMaxWidth().background(TF.Bg)) {
        LazyColumn(state = list, contentPadding = PaddingValues(12.dp)) {
            items(st.console, key = { it.id }) { l ->
                val color = when (l.kind) { LineKind.Out -> TF.Text; LineKind.Err -> TF.Err; LineKind.Info -> TF.Violet; LineKind.Event -> TF.Cyan; LineKind.Input -> TF.Ok }
                Text(l.text, style = style.copy(color = color, fontWeight = if (l.kind == LineKind.Info) FontWeight.SemiBold else FontWeight.Normal),
                    modifier = if (l.line != null) Modifier.clickable { vm.reveal(l.line - 1) } else Modifier)
            }
        }
    }
}

@Composable
fun InputRow(prompt: String, onSend: (String) -> Unit, onEof: (() -> Unit)?) {
    var v by remember { mutableStateOf("") }
    Row(Modifier.fillMaxWidth().background(TF.Surface2).padding(8.dp), verticalAlignment = Alignment.CenterVertically) {
        OutlinedTextField(v, { v = it }, Modifier.weight(1f), singleLine = true,
            label = { Text(prompt.ifBlank { "input" }, maxLines = 1) }, textStyle = monoStyle(14),
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
            keyboardActions = KeyboardActions(onSend = { onSend(v); v = "" }))
        if (onEof != null) TextButton(onClick = onEof) { Text("EOF") }
        IconButton(onClick = { onSend(v); v = "" }) { Icon(Icons.AutoMirrored.Filled.Send, "Send", tint = TF.Cyan) }
    }
}

@Composable
private fun GaugeCard(g: Gauge) {
    Surface(shape = RoundedCornerShape(14.dp), color = TF.Surface2, modifier = Modifier.width(156.dp)) {
        Column(Modifier.padding(12.dp)) {
            Text(g.name, style = MaterialTheme.typography.labelMedium, color = TF.Dim, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Row(verticalAlignment = Alignment.Bottom) {
                Text(fmt(g.value.toFloat()), fontSize = 28.sp, fontWeight = FontWeight.Bold, fontFamily = TF.Mono, color = TF.Text)
                Spacer(Modifier.width(4.dp))
                Text(g.unit, color = TF.Unit, fontSize = 13.sp, modifier = Modifier.padding(bottom = 4.dp))
            }
            if (g.min != null && g.max != null && g.max > g.min) {
                val frac = ((g.value - g.min) / (g.max - g.min)).toFloat().coerceIn(0f, 1f)
                LinearProgressIndicator(progress = { frac }, modifier = Modifier.fillMaxWidth().height(6.dp), color = TF.Cyan, trackColor = TF.Line)
            }
        }
    }
}

@Composable
private fun PlotCard(s: PlotSeries, color: androidx.compose.ui.graphics.Color, events: List<Float>) {
    Surface(shape = RoundedCornerShape(12.dp), color = TF.Surface) {
        Column(Modifier.padding(10.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Box(Modifier.size(8.dp).background(color, RoundedCornerShape(4.dp)))
                Spacer(Modifier.width(6.dp))
                Text(s.name, style = MaterialTheme.typography.labelLarge, modifier = Modifier.weight(1f))
                Text((s.last?.let { fmt(it) } ?: "—") + (s.unit?.let { " $it" } ?: ""), fontFamily = TF.Mono, color = color, fontSize = 14.sp)
            }
            Spacer(Modifier.height(4.dp))
            val n = minOf(s.count, s.v.size)
            val start = if (s.count > s.v.size) s.count % s.v.size else 0
            var lo = Float.POSITIVE_INFINITY; var hi = Float.NEGATIVE_INFINITY
            for (k in 0 until n) { val v = s.v[(start + k) % s.v.size]; if (v < lo) lo = v; if (v > hi) hi = v }
            if (hi - lo < 1e-9f) { hi += 0.5f; lo -= 0.5f }
            val t0 = if (n > 0) s.t[start] else 0f
            val t1 = if (n > 0) s.t[(start + n - 1) % s.v.size] else 1f
            Row {
                Column(Modifier.height(90.dp).width(44.dp), verticalArrangement = Arrangement.SpaceBetween) {
                    Text(fmt(hi), fontSize = 10.sp, color = TF.Dim, fontFamily = TF.Mono)
                    Text(fmt(lo), fontSize = 10.sp, color = TF.Dim, fontFamily = TF.Mono)
                }
                Canvas(Modifier.fillMaxWidth().height(90.dp)) {
                    val w = size.width; val h = size.height
                    for (g in 0..3) { val y = h * g / 3f; drawLine(TF.Line, Offset(0f, y), Offset(w, y), 1f, pathEffect = PathEffect.dashPathEffect(floatArrayOf(4f, 6f))) }
                    if (n < 2) return@Canvas
                    val span = (t1 - t0).coerceAtLeast(1e-6f)
                    for (e in events) if (e in t0..t1) { val x = (e - t0) / span * w; drawLine(TF.Macro.copy(alpha = 0.6f), Offset(x, 0f), Offset(x, h), 1.5f) }
                    val p = Path()
                    for (k in 0 until n) {
                        val i = (start + k) % s.v.size
                        val x = (s.t[i] - t0) / span * w
                        val y = h - (s.v[i] - lo) / (hi - lo) * h
                        if (k == 0) p.moveTo(x, y) else p.lineTo(x, y)
                    }
                    drawPath(p, color, style = Stroke(width = 2.5f))
                }
            }
            Text("%.1f s".format(t1 - t0) + " window · ${s.count} samples", fontSize = 10.sp, color = TF.Dim)
        }
    }
}
