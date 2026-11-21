@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)

package dev.hermitech.tulpafield.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.*
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.hermitech.tulpafield.AppViewModel
import dev.hermitech.tulpafield.BuildConfig
import dev.hermitech.tulpafield.Screen
import dev.hermitech.tulpafield.core.TulpaNative
import java.text.DateFormat
import java.util.Date

/** The welcome page: what Tulpa Field is, where to start, and what you were working on. */
@Composable
fun HomeScreen(vm: AppViewModel, menu: @Composable () -> Unit) {
    val projects by vm.projects.collectAsStateWithLifecycle()
    val ed by vm.editor.collectAsStateWithLifecycle()
    val uri = LocalUriHandler.current
    LaunchedEffect(Unit) { vm.refreshProjects() }

    Scaffold(topBar = {
        TopAppBar(title = {}, actions = {
            IconButton(onClick = { vm.go(Screen.Settings) }) { Icon(Icons.Default.Settings, "Settings") }
            menu()
        }, colors = TopAppBarDefaults.topAppBarColors(containerColor = Color.Transparent))
    }) { pad ->
        Column(Modifier.padding(pad).fillMaxSize().verticalScroll(rememberScrollState()).padding(horizontal = 18.dp)) {
            // ---- hero
            Box(
                Modifier.fillMaxWidth().clip(RoundedCornerShape(22.dp))
                    .background(Brush.linearGradient(listOf(Color(0xFF1A1340), Color(0xFF0B1E33), TF.Bg), Offset.Zero, Offset(900f, 900f)))
                    .border(1.dp, TF.Line, RoundedCornerShape(22.dp)).padding(20.dp),
            ) {
                Canvas(Modifier.matchParentSize()) {   // faint orbit rings behind the logo
                    val c = Offset(size.width * 0.86f, size.height * 0.22f)
                    for (k in 1..4) drawCircle(TF.Violet.copy(alpha = 0.10f / k), radius = 60f * k, center = c, style = androidx.compose.ui.graphics.drawscope.Stroke(2f))
                }
                Column {
                    Icon(painterResourceLogo(), null, tint = Color.Unspecified, modifier = Modifier.size(64.dp))
                    Spacer(Modifier.height(10.dp))
                    Text("Welcome to Tulpa Field", style = MaterialTheme.typography.headlineSmall, fontWeight = FontWeight.Bold)
                    Spacer(Modifier.height(6.dp))
                    Text("Tulpa in your pocket: write code for things that move and sense, run it against your phone's real sensors or a MuJoCo physics model in 3D. Everything runs offline, on the device.",
                        color = TF.Text.copy(alpha = 0.85f), lineHeight = 21.sp)
                    Spacer(Modifier.height(14.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        Button(onClick = { vm.openGuide("welcome") }) { Icon(Icons.AutoMirrored.Filled.MenuBook, null, Modifier.size(18.dp)); Spacer(Modifier.width(6.dp)); Text("Start the tour") }
                        OutlinedButton(onClick = { vm.go(Screen.Learn) }) { Text("Examples") }
                    }
                }
            }

            // ---- continue
            if (ed.file != null) {
                SectionTitle("Continue")
                Card(onClick = { vm.go(Screen.Code) }, colors = CardDefaults.cardColors(containerColor = TF.Surface)) {
                    Row(Modifier.padding(14.dp), verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Default.Code, null, tint = TF.Cyan)
                        Spacer(Modifier.width(12.dp))
                        Column(Modifier.weight(1f)) {
                            Text(ed.file!!.name, fontFamily = TF.Mono, fontWeight = FontWeight.SemiBold)
                            Text(ed.project?.name ?: "", color = TF.Dim, fontSize = 12.sp)
                        }
                        FilledTonalButton(onClick = { vm.runCurrent() }) { Icon(Icons.Default.PlayArrow, null, Modifier.size(18.dp)); Text("Run") }
                    }
                }
            }

            // ---- quick actions
            SectionTitle("Start")
            val actions = listOf(
                Triple("New project", Icons.Default.Add, { vm.go(Screen.Projects); vm.requestNewProject() }),
                Triple("3D simulation", Icons.Default.ViewInAr, { vm.go(Screen.Sim) }),
                Triple("Phone sensors", Icons.Default.Sensors, { vm.go(Screen.Sensors) }),
                Triple("REPL", Icons.Default.Terminal, { vm.go(Screen.Repl) }),
                Triple("Guide", Icons.AutoMirrored.Filled.MenuBook, { vm.openGuide(null) }),
                Triple("All projects", Icons.Default.FolderOpen, { vm.go(Screen.Projects) }),
            )
            actions.chunked(3).forEach { row ->
                Row(Modifier.fillMaxWidth().padding(vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    row.forEach { (label, icon, f) -> QuickTile(label, icon, Modifier.weight(1f), f) }
                }
            }

            // ---- highlights
            SectionTitle("What makes it different")
            Highlight(Icons.Default.Smartphone, "Your phone is the device",
                "phone:: reads the accelerometer, gyro, compass, barometer, light and GPS as unit-checked quantities, and drives vibration, the torch and speech.") { vm.openExample("bubble_level") }
            Highlight(Icons.Default.ViewInAr, "MuJoCo physics + 3D co-simulation",
                "cosim:: programs drive a MuJoCo model on the phone, the same protocol .RAV IDE uses. Watch it live in 3D, push it, reset it.") { vm.openExample("cosim_cartpole") }
            Highlight(Icons.Default.Hub, "State machines you can see",
                "Every machine is drawn as a tappable graph; contracts are fuzzed for counterexamples with one tap.") { vm.openExample("shake_machine") }
            Highlight(Icons.AutoMirrored.Filled.MenuBook, "Learn by running",
                "30+ runnable examples and a guide whose code blocks run with one tap.") { vm.go(Screen.Learn) }

            // ---- recent projects
            if (projects.isNotEmpty()) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    SectionTitle("Recent projects", Modifier.weight(1f))
                    TextButton(onClick = { vm.go(Screen.Projects) }) { Text("All") }
                }
                projects.take(3).forEach { p ->
                    Row(Modifier.fillMaxWidth().clip(RoundedCornerShape(10.dp)).clickable { vm.openProject(p.dir) }.padding(vertical = 10.dp, horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Default.Folder, null, tint = TF.Violet)
                        Spacer(Modifier.width(12.dp))
                        Text(p.name, Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(DateFormat.getDateInstance(DateFormat.MEDIUM).format(Date(p.modified)), color = TF.Dim, fontSize = 12.sp)
                    }
                }
            }

            // ---- footer
            Spacer(Modifier.height(20.dp))
            HorizontalDivider()
            Row(Modifier.padding(vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text("Tulpa Field ${BuildConfig.VERSION_NAME}", fontWeight = FontWeight.SemiBold, fontSize = 13.sp)
                    Text("Tulpa ${runCatching { TulpaNative.version() }.getOrDefault("")} · MuJoCo ${vm.sim.state.value.version}", color = TF.Dim, fontSize = 12.sp)
                }
                TextButton(onClick = { uri.openUri("https://github.com/LoQiseaking69/TulpaLang") }) { Text("GitHub") }
            }
            Spacer(Modifier.height(16.dp))
        }
    }
}

@Composable
private fun SectionTitle(t: String, modifier: Modifier = Modifier) =
    Text(t, style = MaterialTheme.typography.titleSmall, color = TF.Violet, modifier = modifier.padding(top = 22.dp, bottom = 8.dp))

@Composable
private fun QuickTile(label: String, icon: ImageVector, modifier: Modifier, onClick: () -> Unit) {
    Surface(onClick = onClick, modifier = modifier.height(84.dp), shape = RoundedCornerShape(14.dp), color = TF.Surface, border = androidx.compose.foundation.BorderStroke(1.dp, TF.Line)) {
        Column(Modifier.padding(10.dp), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
            Icon(icon, null, tint = TF.Cyan)
            Spacer(Modifier.height(6.dp))
            Text(label, fontSize = 12.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

@Composable
private fun Highlight(icon: ImageVector, title: String, body: String, onClick: () -> Unit) {
    Card(onClick = onClick, modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp), colors = CardDefaults.cardColors(containerColor = TF.Surface)) {
        Row(Modifier.padding(14.dp)) {
            Box(Modifier.size(38.dp).background(TF.Violet.copy(alpha = 0.18f), RoundedCornerShape(10.dp)), contentAlignment = Alignment.Center) { Icon(icon, null, tint = TF.Violet) }
            Spacer(Modifier.width(12.dp))
            Column {
                Text(title, style = MaterialTheme.typography.titleSmall)
                Text(body, color = TF.Dim, fontSize = 13.sp, lineHeight = 18.sp)
            }
        }
    }
}
