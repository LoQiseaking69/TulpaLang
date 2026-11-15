@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalFoundationApi::class)

package dev.hermitech.tulpafield.ui

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.selection.selectable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.hermitech.tulpafield.AppViewModel
import dev.hermitech.tulpafield.Screen
import dev.hermitech.tulpafield.core.Project
import dev.hermitech.tulpafield.core.Template
import java.text.DateFormat
import java.util.Date

@Composable
fun ProjectsScreen(vm: AppViewModel, onImport: () -> Unit, menu: @Composable () -> Unit) {
    val projects by vm.projects.collectAsStateWithLifecycle()
    val ed by vm.editor.collectAsStateWithLifecycle()
    var newDialog by remember { mutableStateOf(false) }
    var menuFor by remember { mutableStateOf<Project?>(null) }
    var renameFor by remember { mutableStateOf<Project?>(null) }
    var deleteFor by remember { mutableStateOf<Project?>(null) }
    val wantNew by vm.newProjectRequest.collectAsStateWithLifecycle()
    LaunchedEffect(Unit) { vm.refreshProjects() }
    LaunchedEffect(wantNew) { if (wantNew) { newDialog = true; vm.newProjectRequest.value = false } }

    Scaffold(
        topBar = {
            TopAppBar(navigationIcon = { IconButton(onClick = { vm.back() }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, "Back") } }, title = {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(painterResourceLogo(), null, tint = androidx.compose.ui.graphics.Color.Unspecified, modifier = Modifier.size(30.dp))
                    Spacer(Modifier.width(10.dp))
                    Column { Text("Projects", fontWeight = FontWeight.SemiBold); Text("TulpaLang ${runCatching { dev.hermitech.tulpafield.core.TulpaNative.version() }.getOrDefault("")}", style = MaterialTheme.typography.labelSmall, color = TF.Dim) }
                }
            }, actions = {
                IconButton(onClick = onImport) { Icon(Icons.Default.FileOpen, "Import a file") }
                menu()
            })
        },
        floatingActionButton = { ExtendedFloatingActionButton(onClick = { newDialog = true }, icon = { Icon(Icons.Default.Add, null) }, text = { Text("New project") }, containerColor = TF.Violet) },
    ) { pad ->
        if (projects.isEmpty()) {
            EmptyState(Modifier.padding(pad), Icons.Default.FolderOpen, "No projects yet",
                "Start from a template, or open an example from the Library and make it yours.",
                "New project" to { newDialog = true }, "Library" to { vm.go(Screen.Learn) })
            return@Scaffold
        }
        LazyColumn(Modifier.padding(pad), contentPadding = PaddingValues(bottom = 96.dp, top = 4.dp)) {
            items(projects, key = { it.dir.path }) { p ->
                Box {
                    Card(
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 5.dp)
                            .combinedClickable(onClick = { vm.openProject(p.dir) }, onLongClick = { menuFor = p }),
                        colors = CardDefaults.cardColors(containerColor = if (ed.project == p.dir) TF.Surface2 else TF.Surface),
                    ) {
                        Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                            Icon(if (p.name == "Inbox") Icons.Default.Inbox else Icons.Default.Folder, null, tint = if (ed.project == p.dir) TF.Cyan else TF.Violet)
                            Spacer(Modifier.width(14.dp))
                            Column(Modifier.weight(1f)) {
                                Text(p.name, style = MaterialTheme.typography.titleMedium)
                                Text("${p.files} file${if (p.files == 1) "" else "s"} · ${DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(p.modified))}", color = TF.Dim, fontSize = 12.sp)
                            }
                            IconButton(onClick = { menuFor = p }) { Icon(Icons.Default.MoreVert, "Project menu", tint = TF.Dim) }
                        }
                    }
                    DropdownMenu(menuFor == p, { menuFor = null }) {
                        DropdownMenuItem(text = { Text("Open") }, leadingIcon = { Icon(Icons.Default.FolderOpen, null) }, onClick = { menuFor = null; vm.openProject(p.dir) })
                        DropdownMenuItem(text = { Text("Rename…") }, leadingIcon = { Icon(Icons.Default.DriveFileRenameOutline, null) }, onClick = { menuFor = null; renameFor = p })
                        DropdownMenuItem(text = { Text("Duplicate") }, leadingIcon = { Icon(Icons.Default.ContentCopy, null) }, onClick = { menuFor = null; vm.duplicateProject(p) })
                        DropdownMenuItem(text = { Text("Share as .zip") }, leadingIcon = { Icon(Icons.Default.Share, null) }, onClick = { menuFor = null; vm.shareProject(p) })
                        DropdownMenuItem(text = { Text("Delete…", color = TF.Err) }, leadingIcon = { Icon(Icons.Default.Delete, null, tint = TF.Err) }, onClick = { menuFor = null; deleteFor = p })
                    }
                }
            }
        }
    }

    if (newDialog) NewProjectDialog({ newDialog = false }) { n, t -> newDialog = false; vm.newProject(n, t) }
    renameFor?.let { p -> TextPrompt("Rename project", "name", p.name, "Rename", { renameFor = null }) { vm.renameProject(p, it); renameFor = null } }
    deleteFor?.let { p ->
        AlertDialog(onDismissRequest = { deleteFor = null }, title = { Text("Delete “${p.name}”?") },
            text = { Text("All ${p.files} files in this project will be removed. This can't be undone.") },
            confirmButton = { TextButton(onClick = { vm.deleteProject(p); deleteFor = null }) { Text("Delete", color = TF.Err) } },
            dismissButton = { TextButton(onClick = { deleteFor = null }) { Text("Cancel") } })
    }
}

@Composable
private fun NewProjectDialog(onDismiss: () -> Unit, onCreate: (String, Template) -> Unit) {
    var name by remember { mutableStateOf("") }
    var tpl by remember { mutableStateOf(Template.Sensors) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("New project") },
        text = {
            Column {
                OutlinedTextField(name, { name = it }, label = { Text("Name") }, singleLine = true, placeholder = { Text(tpl.title) })
                Spacer(Modifier.height(12.dp))
                Text("Start from", style = MaterialTheme.typography.labelLarge, color = TF.Dim)
                Template.entries.forEach { t ->
                    Row(Modifier.fillMaxWidth().selectable(tpl == t, onClick = { tpl = t }).padding(vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                        RadioButton(tpl == t, { tpl = t })
                        Text(t.title)
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = { onCreate(name.ifBlank { tpl.title }, tpl) }) { Text("Create") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

@Composable
fun painterResourceLogo() = androidx.compose.ui.res.painterResource(dev.hermitech.tulpafield.R.drawable.ic_launcher_foreground)
