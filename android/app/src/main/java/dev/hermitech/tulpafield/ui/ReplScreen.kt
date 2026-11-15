@file:OptIn(ExperimentalMaterial3Api::class)

package dev.hermitech.tulpafield.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.*
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.hermitech.tulpafield.AppViewModel
import dev.hermitech.tulpafield.core.ReplKind

@Composable
fun ReplScreen(vm: AppViewModel, menu: @Composable () -> Unit) {
    val st by vm.repl.state.collectAsStateWithLifecycle()
    val prefs by vm.settings.prefs.collectAsStateWithLifecycle()
    val ed by vm.editor.collectAsStateWithLifecycle()
    var input by remember { mutableStateOf(TextFieldValue("")) }
    var histIdx by remember { mutableIntStateOf(-1) }
    var more by remember { mutableStateOf(false) }
    val list = rememberLazyListState()
    LaunchedEffect(st.entries.size) { if (st.entries.isNotEmpty()) list.animateScrollToItem(st.entries.size - 1) }
    val style = monoStyle(prefs.fontSize)

    fun submit() {
        val t = input.text
        if (st.prompt != null) vm.repl.answer(t) else vm.repl.submit(t)
        input = TextFieldValue(""); histIdx = -1
    }
    fun recall(delta: Int) {
        val h = vm.repl.history
        if (h.isEmpty()) return
        histIdx = when { histIdx < 0 && delta < 0 -> h.size - 1; histIdx < 0 -> return; else -> (histIdx + delta) }
        if (histIdx >= h.size) { histIdx = -1; input = TextFieldValue(""); return }
        histIdx = histIdx.coerceIn(0, h.size - 1)
        input = TextFieldValue(h[histIdx], TextRange(h[histIdx].length))
    }

    Scaffold(topBar = {
        TopAppBar(
            title = {
                Column {
                    Text("REPL")
                    Text(if (st.busy) "evaluating…" else "${st.names.size} names defined", style = MaterialTheme.typography.labelSmall, color = if (st.busy) TF.Cyan else TF.Dim)
                }
            },
            actions = {
                if (st.busy) IconButton(onClick = { vm.repl.interrupt() }) { Icon(Icons.Default.Stop, "Interrupt", tint = TF.Err) }
                IconButton(onClick = { ed.file?.let { vm.repl.load(ed.text.text, it.name) } }, enabled = ed.file != null) { Icon(Icons.Default.UploadFile, "Load open file") }
                Box {
                    IconButton(onClick = { more = true }) { Icon(Icons.Default.MoreVert, "More") }
                    DropdownMenu(more, { more = false }) {
                        DropdownMenuItem(text = { Text("Load ${ed.file?.name ?: "file"}") }, enabled = ed.file != null, leadingIcon = { Icon(Icons.Default.UploadFile, null) }, onClick = { more = false; vm.repl.load(ed.text.text, ed.file!!.name) })
                        DropdownMenuItem(text = { Text("Discard unfinished entry") }, enabled = st.continuation, leadingIcon = { Icon(Icons.AutoMirrored.Filled.Backspace, null) }, onClick = { more = false; vm.repl.cancelEntry() })
                        DropdownMenuItem(text = { Text("Clear screen") }, leadingIcon = { Icon(Icons.Default.ClearAll, null) }, onClick = { more = false; vm.repl.clearScreen() })
                        DropdownMenuItem(text = { Text("Reset session") }, leadingIcon = { Icon(Icons.Default.RestartAlt, null) }, onClick = { more = false; vm.repl.reset() })
                    }
                }
                menu()
            },
        )
    }) { pad ->
        Column(Modifier.padding(pad).fillMaxSize().imePadding()) {
            SelectionContainer(Modifier.weight(1f).fillMaxWidth().background(TF.Bg)) {
                LazyColumn(state = list, contentPadding = PaddingValues(12.dp)) {
                    items(st.entries, key = { it.id }) { e ->
                        val color = when (e.kind) { ReplKind.Input -> TF.Violet; ReplKind.Output -> TF.Text; ReplKind.Value -> TF.Cyan; ReplKind.Error -> TF.Err; ReplKind.Info -> TF.Dim }
                        Text(if (e.kind == ReplKind.Value) "= ${e.text}" else e.text, style = style.copy(color = color), modifier = Modifier.padding(vertical = 1.dp))
                    }
                }
            }
            if (st.names.isNotEmpty() && input.text.isNotEmpty()) {
                val word = input.text.takeLastWhile { it.isLetterOrDigit() || it == '_' }
                val sugg = if (word.isEmpty()) emptyList() else st.names.filter { it.startsWith(word) && it != word }.take(12)
                if (sugg.isNotEmpty()) Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).background(TF.Surface2).padding(4.dp), horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    sugg.forEach { s -> SuggestionChip(onClick = { val t = input.text.dropLast(word.length) + s; input = TextFieldValue(t, TextRange(t.length)) }, label = { Text(s, fontFamily = TF.Mono) }) }
                }
            }
            Row(Modifier.fillMaxWidth().background(TF.Surface).horizontalScroll(rememberScrollState()).padding(4.dp), horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                listOf("↑", "↓", "{", "}", "(", ")", "[", "]", "\"", "::", ".", "=", "phone::").forEach { k ->
                    FilledTonalButton(onClick = {
                        when (k) { "↑" -> recall(-1); "↓" -> recall(1); else -> { val p = input.selection.start; val t = input.text.substring(0, p) + k + input.text.substring(input.selection.end); input = TextFieldValue(t, TextRange(p + k.length)) } }
                    }, contentPadding = PaddingValues(horizontal = 10.dp), modifier = Modifier.height(36.dp)) { Text(k, fontFamily = TF.Mono, fontSize = 14.sp) }
                }
            }
            Row(Modifier.fillMaxWidth().background(TF.Surface2).padding(horizontal = 10.dp, vertical = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(when { st.prompt != null -> st.prompt!!.ifBlank { "?" }; st.continuation -> "…"; else -> "›" }, style = style.copy(color = TF.Violet), modifier = Modifier.padding(end = 8.dp))
                BasicTextField(input, { input = it }, Modifier.weight(1f), textStyle = style, cursorBrush = SolidColor(TF.Cyan),
                    visualTransformation = TulpaTransformation(emptyList()),
                    keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false, imeAction = ImeAction.Send),
                    keyboardActions = KeyboardActions(onSend = { submit() }), maxLines = 6)
                IconButton(onClick = { submit() }) { Icon(Icons.AutoMirrored.Filled.Send, "Send", tint = TF.Cyan) }
            }
        }
    }
}
