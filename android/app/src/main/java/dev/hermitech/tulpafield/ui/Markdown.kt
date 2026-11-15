package dev.hermitech.tulpafield.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.CreateNewFolder
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material3.*
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

private sealed interface Block {
    data class Heading(val level: Int, val text: String) : Block
    data class Para(val text: String) : Block
    data class Bullet(val text: String, val ordered: String?) : Block
    data class Code(val lang: String, val code: String) : Block
    data class Table(val rows: List<List<String>>) : Block
    data class Quote(val text: String) : Block
    data object Rule : Block
}

private fun parse(md: String): List<Block> {
    val out = ArrayList<Block>()
    val lines = md.lines()
    var i = 0
    val para = StringBuilder()
    fun flush() { if (para.isNotBlank()) out += Block.Para(para.toString().trim()); para.clear() }
    while (i < lines.size) {
        val l = lines[i]
        val t = l.trim()
        when {
            t.startsWith("```") -> {
                flush()
                val lang = t.removePrefix("```").trim()
                val sb = StringBuilder(); i++
                while (i < lines.size && !lines[i].trim().startsWith("```")) { sb.appendLine(lines[i]); i++ }
                out += Block.Code(lang, sb.toString().trimEnd())
            }
            t.startsWith("#") -> { flush(); val lvl = t.takeWhile { it == '#' }.length; out += Block.Heading(lvl, t.drop(lvl).trim()) }
            t.startsWith("|") -> {
                flush()
                val rows = ArrayList<List<String>>()
                while (i < lines.size && lines[i].trim().startsWith("|")) {
                    val cells = lines[i].trim().trim('|').split("|").map { it.trim() }
                    if (!cells.all { it.matches(Regex(":?-{2,}:?")) }) rows += cells
                    i++
                }
                i--
                out += Block.Table(rows)
            }
            t.startsWith("- ") || t.startsWith("* ") -> {
                flush()
                val sb = StringBuilder(t.drop(2))
                while (i + 1 < lines.size && lines[i + 1].startsWith("  ") && lines[i + 1].isNotBlank() && !lines[i + 1].trim().startsWith("- ")) { i++; sb.append(' ').append(lines[i].trim()) }
                out += Block.Bullet(sb.toString(), null)
            }
            Regex("^\\d+\\. ").containsMatchIn(t) -> {
                flush()
                val num = t.substringBefore(". ")
                val sb = StringBuilder(t.substringAfter(". "))
                while (i + 1 < lines.size && lines[i + 1].startsWith("   ") && lines[i + 1].isNotBlank()) { i++; sb.append(' ').append(lines[i].trim()) }
                out += Block.Bullet(sb.toString(), num)
            }
            t.startsWith(">") -> { flush(); out += Block.Quote(t.trimStart('>', ' ')) }
            t == "---" || t == "***" -> { flush(); out += Block.Rule }
            t.isEmpty() -> flush()
            else -> para.append(t).append(' ')
        }
        i++
    }
    flush()
    return out
}

/** Inline markdown: **bold**, *italic*, `code`, [link](url) (shown as its text). */
fun inline(s: String): AnnotatedString = buildAnnotatedString {
    var i = 0
    while (i < s.length) {
        when {
            s.startsWith("**", i) -> { val e = s.indexOf("**", i + 2); if (e < 0) { append(s.substring(i)); i = s.length } else { withStyle(SpanStyle(fontWeight = FontWeight.Bold, color = TF.Text)) { append(inline(s.substring(i + 2, e))) }; i = e + 2 } }
            s[i] == '`' -> { val e = s.indexOf('`', i + 1); if (e < 0) { append(s.substring(i)); i = s.length } else { withStyle(SpanStyle(fontFamily = TF.Mono, color = TF.Cyan, background = TF.Surface2)) { append(s.substring(i + 1, e)) }; i = e + 1 } }
            s[i] == '[' && s.indexOf("](", i) > 0 && s.indexOf(')', s.indexOf("](", i)) > 0 -> {
                val mid = s.indexOf("](", i); val e = s.indexOf(')', mid)
                withStyle(SpanStyle(color = TF.Violet)) { append(s.substring(i + 1, mid)) }; i = e + 1
            }
            s[i] == '*' && i + 1 < s.length && s[i + 1] != ' ' -> { val e = s.indexOf('*', i + 1); if (e < 0) { append('*'); i++ } else { withStyle(SpanStyle(fontStyle = androidx.compose.ui.text.font.FontStyle.Italic)) { append(s.substring(i + 1, e)) }; i = e + 1 } }
            else -> { append(s[i]); i++ }
        }
    }
}

/**
 * Renders a markdown document. Tulpa code blocks get Run / Copy / Open-as-project buttons when
 * [onRun] is given.
 */
@Composable
fun Markdown(md: String, onRun: ((String) -> Unit)?, onProject: ((String) -> Unit)? = null) {
    val blocks = remember(md) { parse(md) }
    val clip = LocalClipboardManager.current
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        for (b in blocks) when (b) {
            is Block.Heading -> Text(inline(b.text), style = when (b.level) { 1 -> MaterialTheme.typography.headlineSmall; 2 -> MaterialTheme.typography.titleLarge; else -> MaterialTheme.typography.titleMedium },
                color = if (b.level == 1) TF.Text else TF.Violet, modifier = Modifier.padding(top = if (b.level == 1) 0.dp else 8.dp))
            is Block.Para -> Text(inline(b.text), color = TF.Text.copy(alpha = 0.9f), lineHeight = 22.sp)
            is Block.Bullet -> Row { Text(if (b.ordered != null) "${b.ordered}." else "•", color = TF.Cyan, modifier = Modifier.width(22.dp)); Text(inline(b.text), color = TF.Text.copy(alpha = 0.9f), lineHeight = 22.sp) }
            is Block.Quote -> Text(inline(b.text), color = TF.Dim, modifier = Modifier.background(TF.Surface2, RoundedCornerShape(6.dp)).padding(10.dp))
            is Block.Rule -> HorizontalDivider()
            is Block.Table -> Column(Modifier.fillMaxWidth().background(TF.Surface, RoundedCornerShape(8.dp)).horizontalScroll(rememberScrollState()).padding(8.dp)) {
                b.rows.forEachIndexed { r, row ->
                    Row(Modifier.padding(vertical = 4.dp)) {
                        row.forEachIndexed { c, cell ->
                            Text(inline(cell), modifier = Modifier.widthIn(min = 90.dp, max = 260.dp).padding(end = 12.dp), fontSize = 13.sp,
                                fontWeight = if (r == 0) FontWeight.SemiBold else FontWeight.Normal, color = if (r == 0) TF.Violet else TF.Text)
                        }
                    }
                    if (r == 0) HorizontalDivider()
                }
            }
            is Block.Code -> Surface(color = TF.Bg, shape = RoundedCornerShape(10.dp), border = androidx.compose.foundation.BorderStroke(1.dp, TF.Line)) {
                Column {
                    SelectionContainer {
                        Text(if (b.lang == "tulpa" || b.lang == "rav" || b.lang == "text") Highlighter.highlight(b.code) else AnnotatedString(b.code),
                            style = monoStyle(13), modifier = Modifier.horizontalScroll(rememberScrollState()).padding(12.dp))
                    }
                    if (b.lang == "tulpa" && onRun != null) {
                        HorizontalDivider()
                        Row(Modifier.padding(horizontal = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                            TextButton(onClick = { onRun(b.code) }) { Icon(Icons.Default.PlayArrow, null, tint = TF.Ok); Spacer(Modifier.width(4.dp)); Text("Run") }
                            TextButton(onClick = { clip.setText(AnnotatedString(b.code)) }) { Icon(Icons.Default.ContentCopy, null, Modifier.size(16.dp)); Spacer(Modifier.width(4.dp)); Text("Copy") }
                            if (onProject != null) TextButton(onClick = { onProject(b.code) }) { Icon(Icons.Default.CreateNewFolder, null, Modifier.size(16.dp)); Spacer(Modifier.width(4.dp)); Text("As project") }
                        }
                    }
                }
            }
        }
    }
}
