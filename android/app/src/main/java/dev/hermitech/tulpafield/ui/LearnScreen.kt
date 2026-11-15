@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)

package dev.hermitech.tulpafield.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
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
import androidx.compose.material.icons.automirrored.filled.MenuBook
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.hermitech.tulpafield.AppViewModel
import dev.hermitech.tulpafield.core.Example

private val CATEGORY_ICON = mapOf(
    "Start here" to Icons.Default.RocketLaunch, "Phone sensors" to Icons.Default.Sensors, "Robotics" to Icons.Default.PrecisionManufacturing,
    "Timing & reactive" to Icons.Default.Timer, "Verification" to Icons.Default.Verified, "Libraries" to Icons.Default.LibraryBooks,
    "Native" to Icons.Default.Memory, "More" to Icons.Default.MoreHoriz,
)

@Composable
fun LearnScreen(vm: AppViewModel, menu: @Composable () -> Unit) {
    var tab by rememberSaveable { mutableIntStateOf(if (vm.settings.seenWelcome) 0 else 1) }
    var example by remember { mutableStateOf<Example?>(null) }
    val doc by vm.guideDoc.collectAsStateWithLifecycle()
    LaunchedEffect(Unit) { if (!vm.settings.seenWelcome) { vm.guideDoc.value = "welcome"; vm.settings.seenWelcome = true } }

    example?.let { e -> BackHandler { example = null }; ExampleDetail(vm, e) { example = null }; return }
    doc?.let { id -> GuideReader(vm, id); return }

    Scaffold(topBar = {
        Column {
            TopAppBar(title = { Text("Learn") }, actions = { menu() })
            TabRow(tab) {
                Tab(tab == 0, { tab = 0 }, text = { Text("Library") }, icon = { Icon(Icons.Default.Apps, null) })
                Tab(tab == 1, { tab = 1 }, text = { Text("Guide") }, icon = { Icon(Icons.AutoMirrored.Filled.MenuBook, null) })
            }
        }
    }) { pad ->
        Box(Modifier.padding(pad)) {
            if (tab == 0) Library(vm) { example = it } else GuideIndex(vm)
        }
    }
}

@Composable
private fun Library(vm: AppViewModel, open: (Example) -> Unit) {
    var q by rememberSaveable { mutableStateOf("") }
    var cat by rememberSaveable { mutableStateOf<String?>(null) }
    val all = vm.examples
    val cats = remember { all.map { it.category }.distinct() }
    val shown = all.filter { (cat == null || it.category == cat) && (q.isBlank() || listOf(it.title, it.summary, it.tags.joinToString(" "), it.id).any { s -> s.contains(q, true) }) }
    LazyColumn(contentPadding = PaddingValues(bottom = 24.dp)) {
        item {
            OutlinedTextField(q, { q = it }, Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp), singleLine = true,
                placeholder = { Text("Search ${all.size} examples") }, leadingIcon = { Icon(Icons.Default.Search, null) })
            Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FilterChip(cat == null, { cat = null }, label = { Text("All") })
                cats.forEach { c -> FilterChip(cat == c, { cat = if (cat == c) null else c }, label = { Text(c) }, leadingIcon = { Icon(CATEGORY_ICON[c] ?: Icons.Default.Folder, null, Modifier.size(16.dp)) }) }
            }
        }
        shown.groupBy { it.category }.forEach { (c, list) ->
            item(key = "h-$c") {
                Row(Modifier.padding(start = 20.dp, top = 18.dp, bottom = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(CATEGORY_ICON[c] ?: Icons.Default.Folder, null, tint = TF.Violet, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(8.dp))
                    Text(c, style = MaterialTheme.typography.titleSmall, color = TF.Violet)
                }
            }
            items(list, key = { it.id }) { e ->
                Card(onClick = { open(e) }, modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
                    colors = CardDefaults.cardColors(containerColor = TF.Surface)) {
                    Row(Modifier.padding(14.dp), verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            Text(e.title, style = MaterialTheme.typography.titleSmall)
                            Text(e.summary, color = TF.Dim, fontSize = 13.sp, maxLines = 2, overflow = TextOverflow.Ellipsis)
                            if (e.tags.isNotEmpty()) Text(e.tags.joinToString("  ·  "), color = TF.Cyan.copy(alpha = 0.8f), fontSize = 11.sp, modifier = Modifier.padding(top = 4.dp))
                        }
                        IconButton(onClick = { vm.runExample(e) }) { Icon(Icons.Default.PlayCircle, "Run", tint = TF.Ok, modifier = Modifier.size(30.dp)) }
                    }
                }
            }
        }
        if (shown.isEmpty()) item { Text("No examples match “$q”.", color = TF.Dim, modifier = Modifier.padding(24.dp)) }
    }
}

@Composable
private fun ExampleDetail(vm: AppViewModel, e: Example, onBack: () -> Unit) {
    val files = remember(e) { e.dir.walkTopDown().filter { it.isFile && it.name != "expected.txt" }.sortedBy { it.path }.toList() }
    var current by remember(e) { mutableStateOf(e.main) }
    val code = remember(current) { runCatching { current.readText() }.getOrDefault("") }
    val expected = remember(e) { e.expected.takeIf { it.exists() }?.readText() }
    var showExpected by remember { mutableStateOf(false) }
    Scaffold(topBar = {
        TopAppBar(title = { Column { Text(e.title, maxLines = 1, overflow = TextOverflow.Ellipsis); Text(e.category, style = MaterialTheme.typography.labelSmall, color = TF.Dim) } },
            navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") } })
    }, bottomBar = {
        BottomAppBar(containerColor = TF.Surface) {
            Button(onClick = { vm.runExample(e) }, Modifier.padding(start = 12.dp)) { Icon(Icons.Default.PlayArrow, null); Spacer(Modifier.width(6.dp)); Text("Run") }
            Spacer(Modifier.width(8.dp))
            OutlinedButton(onClick = { vm.copyExample(e) }) { Icon(Icons.Default.CreateNewFolder, null); Spacer(Modifier.width(6.dp)); Text("Open as project") }
            Spacer(Modifier.weight(1f))
            IconButton(onClick = { vm.loadIntoRepl(e.main.readText(), e.title) }) { Icon(Icons.Default.Terminal, "Load into REPL") }
        }
    }) { pad ->
        Column(Modifier.padding(pad).fillMaxSize().verticalScroll(rememberScrollState()).padding(16.dp)) {
            Text(e.summary, color = TF.Text.copy(alpha = 0.9f))
            if (e.virtualTime) Text("Runs on the virtual clock.", color = TF.Cyan, fontSize = 12.sp, modifier = Modifier.padding(top = 4.dp))
            if (files.size > 1) {
                Row(Modifier.padding(top = 12.dp).horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    files.forEach { f -> FilterChip(f == current, { current = f }, label = { Text(f.relativeTo(e.dir).path, fontFamily = TF.Mono, fontSize = 12.sp) }) }
                }
            }
            Spacer(Modifier.height(12.dp))
            Surface(color = TF.Bg, shape = RoundedCornerShape(10.dp), border = androidx.compose.foundation.BorderStroke(1.dp, TF.Line)) {
                SelectionContainer { Text(Highlighter.highlight(code), style = monoStyle(12), modifier = Modifier.horizontalScroll(rememberScrollState()).padding(12.dp)) }
            }
            if (expected != null) {
                TextButton(onClick = { showExpected = !showExpected }, modifier = Modifier.padding(top = 8.dp)) {
                    Icon(if (showExpected) Icons.Default.ExpandLess else Icons.Default.ExpandMore, null); Text("Expected output")
                }
                if (showExpected) Surface(color = TF.Surface, shape = RoundedCornerShape(10.dp)) {
                    Text(expected, style = monoStyle(12).copy(color = TF.Dim), modifier = Modifier.horizontalScroll(rememberScrollState()).padding(12.dp))
                }
            }
        }
    }
}

@Composable
private fun GuideIndex(vm: AppViewModel) {
    LazyColumn(contentPadding = PaddingValues(vertical = 8.dp)) {
        vm.guide.groupBy { it.section }.forEach { (section, docs) ->
            item { Text(section, style = MaterialTheme.typography.titleSmall, color = TF.Violet, modifier = Modifier.padding(start = 20.dp, top = 16.dp, bottom = 4.dp)) }
            items(docs, key = { it.id }) { d ->
                ListItem(
                    headlineContent = { Text(d.title) },
                    leadingContent = { Icon(if (section == "Reference") Icons.Default.Article else Icons.AutoMirrored.Filled.MenuBook, null, tint = TF.Cyan) },
                    trailingContent = { Icon(Icons.Default.ChevronRight, null, tint = TF.Dim) },
                    modifier = Modifier.clickable { vm.guideDoc.value = d.id },
                    colors = ListItemDefaults.colors(containerColor = TF.Bg),
                )
            }
        }
    }
}

@Composable
private fun GuideReader(vm: AppViewModel, id: String) {
    val docs = vm.guide
    val idx = docs.indexOfFirst { it.id == id }.coerceAtLeast(0)
    val d = docs.getOrNull(idx) ?: run { vm.guideDoc.value = null; return }
    val md = remember(d) { runCatching { vm.workspace.readAsset(d.asset) }.getOrDefault("# Missing\n\nThis chapter is not bundled.") }
    val scroll = rememberScrollState()
    LaunchedEffect(id) { scroll.scrollTo(0) }
    BackHandler { vm.guideDoc.value = null }
    Scaffold(topBar = {
        TopAppBar(title = { Column { Text(d.title, maxLines = 1, overflow = TextOverflow.Ellipsis); Text(d.section, style = MaterialTheme.typography.labelSmall, color = TF.Dim) } },
            navigationIcon = { IconButton(onClick = { vm.guideDoc.value = null }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") } })
    }) { pad ->
        Column(Modifier.padding(pad).fillMaxSize().verticalScroll(scroll).padding(horizontal = 18.dp, vertical = 12.dp)) {
            Markdown(md, onRun = { vm.runSnippet(it, d.title) }, onProject = { vm.snippetToProject(it, d.title) })
            Spacer(Modifier.height(24.dp))
            Row(Modifier.fillMaxWidth()) {
                docs.getOrNull(idx - 1)?.let { p -> OutlinedButton(onClick = { vm.guideDoc.value = p.id }) { Text("‹ ${p.title}", maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 130.dp)) } }
                Spacer(Modifier.weight(1f))
                docs.getOrNull(idx + 1)?.let { n -> Button(onClick = { vm.guideDoc.value = n.id }) { Text("${n.title} ›", maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = 150.dp)) } }
            }
            Spacer(Modifier.height(32.dp))
        }
    }
}
