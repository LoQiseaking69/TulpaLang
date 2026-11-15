@file:OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)

package dev.hermitech.tulpafield.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.*
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.MenuBook
import androidx.compose.material.icons.filled.*
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.hermitech.tulpafield.AppViewModel
import dev.hermitech.tulpafield.Screen

private data class Tab(val screen: Screen, val label: String, val icon: ImageVector)

private val TABS = listOf(
    Tab(Screen.Home, "Home", Icons.Default.Home),
    Tab(Screen.Code, "Code", Icons.Default.Code),
    Tab(Screen.Run, "Run", Icons.Default.PlayCircle),
    Tab(Screen.Sim, "Sim", Icons.Default.ViewInAr),
    Tab(Screen.Repl, "REPL", Icons.Default.Terminal),
    Tab(Screen.Learn, "Learn", Icons.AutoMirrored.Filled.MenuBook),
)

@Composable
fun MainScreen(vm: AppViewModel, onImport: () -> Unit, onExit: () -> Unit) {
    val screen by vm.screen.collectAsStateWithLifecycle()
    val run by vm.runner.state.collectAsStateWithLifecycle()
    BackHandler { if (!vm.back()) onExit() }

    val menu: @Composable () -> Unit = { ToolsMenu(vm) }
    val imeOpen = WindowInsets.isImeVisible
    val isTab = TABS.any { it.screen == screen } && !imeOpen

    Scaffold(
        bottomBar = {
            if (isTab) NavigationBar(containerColor = TF.Surface) {
                TABS.forEach { t ->
                    NavigationBarItem(
                        selected = screen == t.screen,
                        onClick = { vm.go(t.screen) },
                        icon = {
                            if (t.screen == Screen.Run && run.running) BadgedBox(badge = { Badge(containerColor = TF.Ok) }) { Icon(t.icon, t.label) }
                            else Icon(t.icon, t.label)
                        },
                        label = { Text(t.label, maxLines = 1, fontSize = androidx.compose.ui.unit.TextUnit(11f, androidx.compose.ui.unit.TextUnitType.Sp)) },
                        alwaysShowLabel = true,
                        colors = NavigationBarItemDefaults.colors(indicatorColor = TF.Violet.copy(alpha = 0.25f), selectedIconColor = TF.Cyan, selectedTextColor = TF.Cyan),
                    )
                }
            }
        },
        contentWindowInsets = WindowInsets(0),
    ) { pad ->
        Box(Modifier.padding(pad).fillMaxSize()) {
            when (screen) {
                Screen.Home -> HomeScreen(vm, menu)
                Screen.Projects -> ProjectsScreen(vm, onImport, menu)
                Screen.Sim -> SimScreen(vm, menu)
                Screen.Code -> EditorScreen(vm, menu)
                Screen.Run -> RunScreen(vm, menu)
                Screen.Repl -> ReplScreen(vm, menu)
                Screen.Learn -> LearnScreen(vm, menu)
                Screen.Sensors -> SensorsScreen(vm)
                Screen.Machines -> MachinesScreen(vm)
                Screen.Verify -> VerifyScreen(vm)
                Screen.Bytecode -> BytecodeScreen(vm)
                Screen.Settings -> SettingsScreen(vm)
            }
        }
    }
}

@Composable
private fun ToolsMenu(vm: AppViewModel) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { Icon(Icons.Default.Apps, "Instruments") }
        DropdownMenu(open, { open = false }) {
            listOf(
                Triple("Sensors", Icons.Default.Sensors, Screen.Sensors),
                Triple("Machines", Icons.Default.Hub, Screen.Machines),
                Triple("Verify & tests", Icons.Default.Verified, Screen.Verify),
                Triple("Bytecode", Icons.Default.Memory, Screen.Bytecode),
                Triple("Settings", Icons.Default.Settings, Screen.Settings),
            ).forEach { (label, icon, s) ->
                DropdownMenuItem(text = { Text(label) }, leadingIcon = { Icon(icon, null) }, onClick = { open = false; vm.go(s) })
            }
        }
    }
}
