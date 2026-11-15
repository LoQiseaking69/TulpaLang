package dev.hermitech.tulpafield.ui

import androidx.compose.foundation.layout.*
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp

fun Modifier.clip8() = this.clip(RoundedCornerShape(8.dp))

@Composable
fun EmptyState(modifier: Modifier, icon: ImageVector, title: String, body: String, vararg actions: Pair<String, () -> Unit>) {
    Column(modifier.fillMaxSize().padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
        Icon(icon, null, tint = TF.Violet, modifier = Modifier.size(56.dp))
        Spacer(Modifier.height(16.dp))
        Text(title, style = MaterialTheme.typography.titleLarge)
        Spacer(Modifier.height(8.dp))
        Text(body, color = TF.Dim, textAlign = TextAlign.Center)
        Spacer(Modifier.height(20.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            actions.forEachIndexed { i, (label, f) ->
                if (i == 0) Button(onClick = f) { Text(label) } else OutlinedButton(onClick = f) { Text(label) }
            }
        }
    }
}

@Composable
fun TextPrompt(title: String, label: String, initial: String, confirm: String, onDismiss: () -> Unit, onConfirm: (String) -> Unit) {
    var v by remember { mutableStateOf(initial) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = { OutlinedTextField(v, { v = it }, label = { Text(label) }, singleLine = true) },
        confirmButton = { TextButton(onClick = { if (v.isNotBlank()) onConfirm(v.trim()) }) { Text(confirm) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

fun fmt(v: Float): String = when {
    v.isNaN() -> "—"
    v == 0f -> "0"
    kotlin.math.abs(v) >= 10000 || kotlin.math.abs(v) < 0.01 -> "%.3g".format(v)
    kotlin.math.abs(v) >= 100 -> "%.1f".format(v)
    else -> "%.3f".format(v).trimEnd('0').trimEnd('.')
}
