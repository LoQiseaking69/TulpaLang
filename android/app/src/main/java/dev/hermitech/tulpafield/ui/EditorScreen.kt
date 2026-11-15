@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class, ExperimentalLayoutApi::class)

package dev.hermitech.tulpafield.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.*
import androidx.compose.material.icons.automirrored.filled.Redo
import androidx.compose.material.icons.automirrored.filled.Undo
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.hermitech.tulpafield.AppViewModel
import dev.hermitech.tulpafield.Screen
import kotlinx.coroutines.launch

private val UNITS = listOf(
    "m", "cm", "mm", "km", "s", "ms", "us", "min", "h", "Hz", "kHz",
    "deg", "rad", "rad/s", "m/s", "km/h", "m/s^2", "g", "kg", "N", "N*m", "J", "W", "kW",
    "V", "mV", "A", "mA", "ohm", "kohm", "F", "uF", "Pa", "kPa", "hPa", "bar", "K", "degC", "lux",
)

@Composable
fun EditorScreen(vm: AppViewModel, menu: @Composable () -> Unit) {
    val st by vm.editor.collectAsStateWithLifecycle()
    val prefs by vm.settings.prefs.collectAsStateWithLifecycle()
    val run by vm.runner.state.collectAsStateWithLifecycle()
    val scope = rememberCoroutineScope()

    var fileMenu by remember { mutableStateOf(false) }
    var overflow by remember { mutableStateOf(false) }
    var dialog by remember { mutableStateOf<String?>(null) }   // newfile | rename | delete
    var sheet by remember { mutableStateOf<String?>(null) }    // problems | outline | inspect | units
    var inspectText by remember { mutableStateOf<String?>(null) }
    var findOpen by remember { mutableStateOf(false) }

    if (st.project == null || st.file == null) {
        Scaffold(topBar = { TopAppBar(title = { Text("Code") }, actions = { menu() }) }) { pad ->
            EmptyState(Modifier.padding(pad), Icons.Default.Code, "No file open",
                "Open a project, or copy an example from the Library to start editing.",
                "Projects" to { vm.go(Screen.Projects) }, "Library" to { vm.go(Screen.Learn) })
        }
        return
    }

    val errors = st.diagnostics.count { it.error }
    val warnings = st.diagnostics.size - errors

    Scaffold(
        topBar = {
            TopAppBar(
                title = {
                    Box {
                        Row(Modifier.clip8().clickable { fileMenu = true }.padding(horizontal = 6.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                            Column {
                                Text(st.file!!.relativeTo(st.project!!).path + if (st.dirty) " •" else "", style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Text(st.project!!.name, style = MaterialTheme.typography.labelSmall, color = TF.Dim, maxLines = 1)
                            }
                            Icon(Icons.Default.ArrowDropDown, null, tint = TF.Dim)
                        }
                        DropdownMenu(fileMenu, { fileMenu = false }) {
                            st.files.forEach { f ->
                                DropdownMenuItem(
                                    text = { Text(f.relativeTo(st.project!!).path, fontFamily = TF.Mono, fontSize = 13.sp) },
                                    leadingIcon = { Icon(if (f.extension == "rav") Icons.Default.Description else Icons.AutoMirrored.Filled.InsertDriveFile, null, tint = if (f == st.file) TF.Cyan else TF.Dim) },
                                    onClick = { fileMenu = false; vm.openFile(f) })
                            }
                            HorizontalDivider()
                            DropdownMenuItem(text = { Text("New file…") }, leadingIcon = { Icon(Icons.AutoMirrored.Filled.NoteAdd, null) }, onClick = { fileMenu = false; dialog = "newfile" })
                            DropdownMenuItem(text = { Text("Rename file…") }, leadingIcon = { Icon(Icons.Default.DriveFileRenameOutline, null) }, onClick = { fileMenu = false; dialog = "rename" })
                            DropdownMenuItem(text = { Text("Share file") }, leadingIcon = { Icon(Icons.Default.Share, null) }, onClick = { fileMenu = false; vm.save(true); vm.shareFile(st.file!!) })
                            DropdownMenuItem(text = { Text("Delete file") }, leadingIcon = { Icon(Icons.Default.Delete, null, tint = TF.Err) }, onClick = { fileMenu = false; dialog = "delete" })
                            DropdownMenuItem(text = { Text("Refresh file list") }, leadingIcon = { Icon(Icons.Default.Refresh, null) }, onClick = { fileMenu = false; vm.refreshFiles() })
                        }
                    }
                },
                actions = {
                    if (st.diagnostics.isNotEmpty()) {
                        TextButton(onClick = { sheet = "problems" }) {
                            Icon(if (errors > 0) Icons.Default.Error else Icons.Default.Warning, null, tint = if (errors > 0) TF.Err else TF.Warn, modifier = Modifier.size(18.dp))
                            Spacer(Modifier.width(4.dp)); Text("${if (errors > 0) errors else warnings}", color = if (errors > 0) TF.Err else TF.Warn)
                        }
                    }
                    IconButton(onClick = { vm.undo() }, enabled = vm.canUndo) { Icon(Icons.AutoMirrored.Filled.Undo, "Undo") }
                    IconButton(onClick = { if (run.running) vm.go(Screen.Run) else vm.runCurrent() }) {
                        Icon(if (run.running) Icons.Default.Pending else Icons.Default.PlayArrow, "Run", tint = TF.Ok)
                    }
                    Box {
                        IconButton(onClick = { overflow = true }) { Icon(Icons.Default.MoreVert, "More") }
                        DropdownMenu(overflow, { overflow = false }) {
                            DropdownMenuItem(text = { Text("Redo") }, leadingIcon = { Icon(Icons.AutoMirrored.Filled.Redo, null) }, enabled = vm.canRedo, onClick = { overflow = false; vm.redo() })
                            DropdownMenuItem(text = { Text("Save") }, leadingIcon = { Icon(Icons.Default.Save, null) }, onClick = { overflow = false; vm.save() })
                            DropdownMenuItem(text = { Text("Find / replace") }, leadingIcon = { Icon(Icons.Default.Search, null) }, onClick = { overflow = false; findOpen = true })
                            DropdownMenuItem(text = { Text("Outline") }, leadingIcon = { Icon(Icons.Default.AccountTree, null) }, onClick = { overflow = false; vm.refreshSymbols(); sheet = "outline" })
                            DropdownMenuItem(text = { Text("Inspect at cursor") }, leadingIcon = { Icon(Icons.Default.Info, null) }, onClick = {
                                overflow = false; scope.launch { inspectText = vm.hoverAtCursor() ?: "Nothing to show here — place the cursor on a name."; sheet = "inspect" }
                            })
                            DropdownMenuItem(text = { Text("Go to definition") }, leadingIcon = { Icon(Icons.Default.ArrowOutward, null) }, onClick = { overflow = false; vm.gotoDefinition() })
                            HorizontalDivider()
                            DropdownMenuItem(text = { Text("Machines") }, leadingIcon = { Icon(Icons.Default.Hub, null) }, onClick = { overflow = false; vm.go(Screen.Machines) })
                            DropdownMenuItem(text = { Text("Verify & tests") }, leadingIcon = { Icon(Icons.Default.Verified, null) }, onClick = { overflow = false; vm.go(Screen.Verify) })
                            DropdownMenuItem(text = { Text("Bytecode") }, leadingIcon = { Icon(Icons.Default.Memory, null) }, onClick = { overflow = false; vm.go(Screen.Bytecode) })
                            DropdownMenuItem(text = { Text("Load into REPL") }, leadingIcon = { Icon(Icons.Default.Terminal, null) }, onClick = { overflow = false; vm.loadIntoRepl(st.text.text, st.file!!.name) })
                        }
                    }
                },
            )
        },
    ) { pad ->
        Column(Modifier.padding(pad).fillMaxSize().imePadding()) {
            if (findOpen) FindBar(vm) { findOpen = false }
            CodeArea(vm, prefs.fontSize, prefs.lineNumbers, prefs.pairBrackets, Modifier.weight(1f))
            if (st.completions.isNotEmpty()) {
                LazyRow(Modifier.fillMaxWidth().background(TF.Surface2).padding(vertical = 4.dp), contentPadding = PaddingValues(horizontal = 6.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    items(st.completions, key = { it.label }) { c ->
                        Surface(onClick = { vm.applyCompletion(c) }, shape = RoundedCornerShape(8.dp), color = TF.Surface, border = androidx.compose.foundation.BorderStroke(1.dp, TF.Line)) {
                            Row(Modifier.padding(horizontal = 10.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically) {
                                Text(kindGlyph(c.kind), color = kindColor(c.kind), fontSize = 11.sp, fontFamily = TF.Mono)
                                Spacer(Modifier.width(6.dp))
                                Text(c.label, fontFamily = TF.Mono, fontSize = 13.sp)
                            }
                        }
                    }
                }
            }
            if (!st.readOnly) SymbolBar(vm, onUnits = { sheet = "units" })
        }
    }

    when (sheet) {
        "problems" -> ModalBottomSheet({ sheet = null }) {
            Text("Problems", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 20.dp))
            LazyColumn(Modifier.padding(bottom = 24.dp)) {
                items(st.diagnostics) { d ->
                    ListItem(
                        leadingContent = { Icon(if (d.error) Icons.Default.Error else Icons.Default.Warning, null, tint = if (d.error) TF.Err else TF.Warn) },
                        headlineContent = { Text(d.message) },
                        supportingContent = { Text("line ${d.line + 1}, column ${d.col + 1}", color = TF.Dim) },
                        modifier = Modifier.clickable { sheet = null; vm.reveal(d.line, d.col) },
                        colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow),
                    )
                }
            }
        }
        "outline" -> ModalBottomSheet({ sheet = null }) {
            val syms by vm.symbols.collectAsStateWithLifecycle()
            Text("Outline", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 20.dp))
            if (syms.isEmpty()) Text("No top-level definitions.", color = TF.Dim, modifier = Modifier.padding(20.dp))
            LazyColumn(Modifier.padding(bottom = 24.dp)) {
                items(syms) { s ->
                    ListItem(
                        leadingContent = { Text(symbolGlyph(s.kind), fontFamily = TF.Mono, color = symbolColor(s.kind), fontWeight = FontWeight.Bold) },
                        headlineContent = { Text(s.name, fontFamily = TF.Mono) },
                        supportingContent = { if (s.detail.isNotBlank()) Text(s.detail, color = TF.Dim, maxLines = 1, overflow = TextOverflow.Ellipsis, fontFamily = TF.Mono, fontSize = 12.sp) },
                        trailingContent = { Text("${s.line + 1}", color = TF.Dim) },
                        modifier = Modifier.clickable { sheet = null; vm.reveal(s.line) },
                        colors = ListItemDefaults.colors(containerColor = MaterialTheme.colorScheme.surfaceContainerLow),
                    )
                }
            }
        }
        "inspect" -> ModalBottomSheet({ sheet = null }) {
            Column(Modifier.padding(horizontal = 20.dp).padding(bottom = 32.dp).verticalScroll(rememberScrollState())) {
                Text("Inspect", style = MaterialTheme.typography.titleMedium)
                Spacer(Modifier.height(8.dp))
                Markdown(inspectText ?: "", onRun = null)
                Spacer(Modifier.height(12.dp))
                OutlinedButton(onClick = { sheet = null; vm.gotoDefinition() }) { Text("Go to definition") }
            }
        }
        "units" -> ModalBottomSheet({ sheet = null }) {
            Text("Units", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(horizontal = 20.dp))
            FlowRow(Modifier.padding(16.dp).padding(bottom = 24.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                UNITS.forEach { u ->
                    AssistChip(onClick = { sheet = null; vm.insert("[$u]") }, label = { Text("[$u]", fontFamily = TF.Mono, color = TF.Unit) })
                }
            }
        }
    }

    when (dialog) {
        "newfile" -> TextPrompt("New file", "name (e.g. lib/drive.rav)", "", "Create", { dialog = null }) { vm.newFile(it); dialog = null }
        "rename" -> TextPrompt("Rename file", "new name", st.file!!.name, "Rename", { dialog = null }) { vm.renameFile(it); dialog = null }
        "delete" -> AlertDialog(onDismissRequest = { dialog = null },
            title = { Text("Delete ${st.file!!.name}?") }, text = { Text("This can't be undone.") },
            confirmButton = { TextButton(onClick = { dialog = null; vm.deleteFile() }) { Text("Delete", color = TF.Err) } },
            dismissButton = { TextButton(onClick = { dialog = null }) { Text("Cancel") } })
    }
}

@Composable
private fun CodeArea(vm: AppViewModel, fontSize: Int, lineNumbers: Boolean, pair: Boolean, modifier: Modifier) {
    val st by vm.editor.collectAsStateWithLifecycle()
    val style = monoStyle(fontSize)
    val density = LocalDensity.current
    val lineHeightPx = with(density) { style.lineHeight.toPx() }
    val vScroll = rememberScrollState()
    val hScroll = rememberScrollState()
    val screenW = LocalConfiguration.current.screenWidthDp.dp
    val lines = remember(st.text.text) { st.text.text.count { it == '\n' } + 1 }
    val errLines = remember(st.diagnostics) { st.diagnostics.groupBy { it.line }.mapValues { (_, v) -> v.any { it.error } } }
    val transformation = remember(st.diagnostics) { TulpaTransformation(st.diagnostics) }
    val cursorLine = remember(st.text.selection.start, st.text.text) { st.text.text.substring(0, st.text.selection.start.coerceIn(0, st.text.text.length)).count { it == '\n' } }

    LaunchedEffect(st.revealLine) {
        st.revealLine?.let { l ->
            vScroll.animateScrollTo(((l - 3).coerceAtLeast(0) * lineHeightPx).toInt())
            vm.revealHandled()
        }
    }

    Row(modifier.fillMaxWidth().background(TF.Bg).verticalScroll(vScroll)) {
        if (lineNumbers) {
            val gutter = remember(lines, errLines, cursorLine) {
                buildAnnotatedString {
                    for (i in 0 until lines) {
                        val e = errLines[i]
                        withStyle(SpanStyle(color = when (e) { true -> TF.Err; false -> TF.Warn; null -> if (i == cursorLine) TF.Text else TF.Comment })) {
                            append((if (e != null) "● " else "") + (i + 1).toString())
                        }
                        if (i < lines - 1) append('\n')
                    }
                }
            }
            Text(gutter, style = style.copy(color = TF.Comment, textAlign = androidx.compose.ui.text.style.TextAlign.End),
                modifier = Modifier.background(TF.Surface).padding(start = 6.dp, end = 8.dp, top = 8.dp, bottom = 200.dp).widthIn(min = 28.dp))
        }
        Box(Modifier.horizontalScroll(hScroll)) {
            BasicTextField(
                value = st.text,
                onValueChange = { nv -> vm.edit(smartEdit(st.text, nv, pair)) },
                textStyle = style,
                readOnly = st.readOnly,
                cursorBrush = SolidColor(TF.Cyan),
                visualTransformation = transformation,
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false, keyboardType = KeyboardType.Ascii),
                modifier = Modifier.widthIn(min = screenW).padding(start = 8.dp, end = 48.dp, top = 8.dp, bottom = 200.dp),
            )
        }
    }
}

/** Auto-indent, bracket pairing and over-typing of closers. */
fun smartEdit(old: TextFieldValue, new: TextFieldValue, pair: Boolean): TextFieldValue {
    val o = old.text; val n = new.text
    if (n.length != o.length + 1 || new.selection.length != 0) return new
    val pos = new.selection.start
    if (pos <= 0 || pos > n.length) return new
    if (o.substring(0, pos - 1) != n.substring(0, pos - 1) || o.substring(pos - 1) != n.substring(pos)) return new
    val c = n[pos - 1]
    val next = n.getOrNull(pos)
    when {
        c == '\n' -> {
            val lineStart = n.lastIndexOf('\n', pos - 2) + 1
            val prevLine = n.substring(lineStart, pos - 1)
            val indent = prevLine.takeWhile { it == ' ' || it == '\t' }
            val opens = prevLine.trimEnd().endsWith("{") || prevLine.trimEnd().endsWith("(") || prevLine.trimEnd().endsWith("[")
            val extra = if (opens) "    " else ""
            return if (opens && next != null && next in "})]") {
                val ins = indent + extra + "\n" + indent
                TextFieldValue(n.substring(0, pos) + ins + n.substring(pos), TextRange(pos + indent.length + extra.length))
            } else {
                val ins = indent + extra
                TextFieldValue(n.substring(0, pos) + ins + n.substring(pos), TextRange(pos + ins.length))
            }
        }
        !pair -> return new
        c in ")]}\"" && next == c && (c != '"' || o.getOrNull(pos - 2) != '\\') ->
            return TextFieldValue(o, TextRange(pos))          // type over the auto-inserted closer
        c in "([{" && (next == null || next.isWhitespace() || next in ")]},") -> {
            val close = when (c) { '(' -> ')'; '[' -> ']'; else -> '}' }
            return TextFieldValue(n.substring(0, pos) + close + n.substring(pos), TextRange(pos))
        }
        c == '"' && (next == null || next.isWhitespace() || next in ")]},") && (pos < 2 || !n[pos - 2].isLetterOrDigit() || n[pos - 2] == 'f') -> {
            return TextFieldValue(n.substring(0, pos) + "\"" + n.substring(pos), TextRange(pos))
        }
        c == '}' -> {
            // dedent a closing brace typed on an otherwise blank line
            val lineStart = n.lastIndexOf('\n', pos - 2) + 1
            val before = n.substring(lineStart, pos - 1)
            if (before.isNotEmpty() && before.isBlank() && before.length >= 4) {
                val t = n.substring(0, lineStart) + before.drop(4) + n.substring(pos - 1)
                return TextFieldValue(t, TextRange(pos - 4))
            }
        }
    }
    return new
}

@Composable
private fun SymbolBar(vm: AppViewModel, onUnits: () -> Unit) {
    val keys = listOf("⇥", "←", "→", "{", "}", "(", ")", "[", "]", "\"", ":", "::", "=>", ".", ",", "|", "=", "#", "!")
    Row(Modifier.fillMaxWidth().background(TF.Surface).horizontalScroll(rememberScrollState()).padding(horizontal = 4.dp, vertical = 4.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        Key("tab") { vm.insert("    ") }
        keys.forEach { k ->
            Key(k) {
                when (k) {
                    "⇥" -> vm.caretToLineEnd()
                    "←" -> vm.moveCaret(-1)
                    "→" -> vm.moveCaret(1)
                    "=>" -> vm.insert(" => ")
                    else -> vm.insert(k)
                }
            }
        }
        Key("[m]", accent = true, onLong = onUnits) { vm.insert("[m]") }
        Key("[s]", accent = true, onLong = onUnits) { vm.insert("[s]") }
        Key("[deg]", accent = true, onLong = onUnits) { vm.insert("[deg]") }
        Key("units…", accent = true) { onUnits() }
    }
}

@Composable
private fun Key(label: String, accent: Boolean = false, onLong: (() -> Unit)? = null, onClick: () -> Unit) {
    Box(
        Modifier.clip8().background(TF.Surface2).border(1.dp, TF.Line, RoundedCornerShape(8.dp))
            .combinedClickable(onClick = onClick, onLongClick = onLong)
            .heightIn(min = 38.dp).widthIn(min = 38.dp).padding(horizontal = 10.dp),
        contentAlignment = Alignment.Center,
    ) { Text(label, fontFamily = TF.Mono, fontSize = 15.sp, color = if (accent) TF.Unit else TF.Text) }
}

@Composable
private fun FindBar(vm: AppViewModel, onClose: () -> Unit) {
    var q by remember { mutableStateOf("") }
    var r by remember { mutableStateOf("") }
    val st by vm.editor.collectAsStateWithLifecycle()
    val count = remember(q, st.text.text) { if (q.isEmpty()) 0 else Regex(Regex.escape(q)).findAll(st.text.text).count() }
    fun find(forward: Boolean) {
        if (q.isEmpty()) return
        val t = st.text.text
        val from = if (forward) st.text.selection.max else st.text.selection.min - 1
        val i = if (forward) t.indexOf(q, from).let { if (it < 0) t.indexOf(q) else it } else t.lastIndexOf(q, from.coerceAtLeast(0)).let { if (it < 0) t.lastIndexOf(q) else it }
        if (i >= 0) {
            vm.edit(st.text.copy(selection = TextRange(i, i + q.length)))
            vm.reveal(t.substring(0, i).count { it == '\n' }, 0)
            vm.edit(vm.editor.value.text.copy(selection = TextRange(i, i + q.length)))
        }
    }
    Column(Modifier.fillMaxWidth().background(TF.Surface2).padding(horizontal = 8.dp, vertical = 4.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            OutlinedTextField(q, { q = it }, Modifier.weight(1f), singleLine = true, placeholder = { Text("find") },
                textStyle = monoStyle(14), supportingText = null, keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                keyboardActions = KeyboardActions(onSearch = { find(true) }))
            Text(" $count ", color = TF.Dim)
            IconButton(onClick = { find(false) }) { Icon(Icons.Default.KeyboardArrowUp, "Previous") }
            IconButton(onClick = { find(true) }) { Icon(Icons.Default.KeyboardArrowDown, "Next") }
            IconButton(onClick = onClose) { Icon(Icons.Default.Close, "Close") }
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            OutlinedTextField(r, { r = it }, Modifier.weight(1f), singleLine = true, placeholder = { Text("replace with") }, textStyle = monoStyle(14))
            TextButton(onClick = {
                if (q.isNotEmpty() && count > 0) vm.edit(TextFieldValue(st.text.text.replace(q, r), TextRange(0)))
            }) { Text("Replace all") }
        }
    }
}

fun kindGlyph(k: Int) = when (k) { 2 -> "."; 11 -> "u"; 3 -> "ƒ"; 14 -> "kw"; 6 -> "v"; 9 -> "::"; 22 -> "S"; 7 -> "M"; 21 -> "c"; else -> "·" }
fun kindColor(k: Int) = when (k) { 11 -> TF.Unit; 2 -> TF.Func; 3 -> TF.Func; 14 -> TF.Keyword; 9 -> TF.Module; 7, 22 -> TF.Type; else -> TF.Dim }
fun symbolGlyph(k: Int) = when (k) { 12 -> "ƒ"; 23 -> "S"; 5 -> "M"; 14 -> "c"; 13 -> "v"; else -> "·" }
fun symbolColor(k: Int) = when (k) { 12 -> TF.Func; 23, 5 -> TF.Type; 14 -> TF.Num; else -> TF.Dim }
