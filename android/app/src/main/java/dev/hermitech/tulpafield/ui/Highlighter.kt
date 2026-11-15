package dev.hermitech.tulpafield.ui

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.OffsetMapping
import androidx.compose.ui.text.input.TransformedText
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.text.style.TextDecoration
import dev.hermitech.tulpafield.core.Diagnostic

/** A small hand-written lexer for Tulpa syntax colouring (fast enough to run on every keystroke). */
object Highlighter {
    val keywords = setOf(
        "let", "const", "fn", "def", "return", "if", "elif", "else", "while", "loop", "for", "in", "not", "and", "or",
        "break", "continue", "match", "struct", "machine", "state", "on", "enter", "exit", "when", "always", "after",
        "within", "ensure", "require", "import", "from", "as", "use", "try", "catch", "throw", "spawn", "await", "unit",
        "contract",
    )
    private val constants = setOf("true", "false", "nil", "result")

    fun highlight(text: String, diagnostics: List<Diagnostic> = emptyList()): AnnotatedString {
        val b = AnnotatedString.Builder(text)
        val n = text.length
        var i = 0
        var prevSignificant = ' '
        fun span(s: Int, e: Int, style: SpanStyle) { if (e > s) b.addStyle(style, s, e) }
        while (i < n) {
            val c = text[i]
            when {
                c == '#' || (c == '/' && i + 1 < n && text[i + 1] == '/') -> {
                    val e = text.indexOf('\n', i).let { if (it < 0) n else it }
                    span(i, e, SpanStyle(color = TF.Comment, fontStyle = FontStyle.Italic)); i = e
                }
                c == '/' && i + 1 < n && text[i + 1] == '*' -> {
                    val e = text.indexOf("*/", i + 2).let { if (it < 0) n else it + 2 }
                    span(i, e, SpanStyle(color = TF.Comment, fontStyle = FontStyle.Italic)); i = e
                }
                c == '"' || c == '\'' || ((c == 'f') && i + 1 < n && (text[i + 1] == '"' || text[i + 1] == '\'') && (i == 0 || !text[i - 1].isLetterOrDigit())) -> {

                    val fstr = c == 'f'
                    if (fstr) { span(i, i + 1, SpanStyle(color = TF.Macro)); i++ }
                    val q = text[i]
                    var seg = i
                    i++
                    while (i < n && text[i] != q && text[i] != '\n') {
                        if (text[i] == '\\') { i += 2; continue }
                        if (fstr && text[i] == '{') {
                            val close = text.indexOf('}', i)
                            if (close > 0 && !text.substring(i, close).contains('\n')) {
                                span(seg, i, SpanStyle(color = TF.Str)); span(i, close + 1, SpanStyle(color = TF.Unit))
                                i = close + 1; seg = i; continue
                            }
                        }
                        i++
                    }
                    i = (i + 1).coerceAtMost(n)
                    span(seg, i, SpanStyle(color = TF.Str))
                    prevSignificant = '"'
                }
                c.isDigit() -> {
                    val s = i
                    if (c == '0' && i + 1 < n && text[i + 1] in "xXbB") { i += 2; while (i < n && (text[i].isLetterOrDigit() || text[i] == '_')) i++ }
                    else { while (i < n && (text[i].isDigit() || text[i] == '_' || text[i] == '.' && i + 1 < n && text[i + 1].isDigit() || (text[i] in "eE" && i + 1 < n && (text[i + 1].isDigit() || text[i + 1] == '-')))) { if (text[i] in "eE") i++; i++ } }
                    span(s, i, SpanStyle(color = TF.Num))
                    if (i < n && text[i] == '[') {
                        val e = text.indexOf(']', i)
                        if (e > 0 && !text.substring(i, e).contains('\n') && e - i < 24) { span(i, e + 1, SpanStyle(color = TF.Unit)); i = e + 1 }
                    }
                    prevSignificant = '0'
                }
                c.isLetter() || c == '_' -> {
                    val s = i
                    while (i < n && (text[i].isLetterOrDigit() || text[i] == '_')) i++
                    val w = text.substring(s, i)
                    val next = if (i < n) text[i] else ' '
                    val style = when {
                        w in keywords -> SpanStyle(color = TF.Keyword, fontWeight = FontWeight.SemiBold)
                        w in constants -> SpanStyle(color = TF.Num)
                        next == '!' && (i + 1 >= n || text[i + 1] != '=') -> SpanStyle(color = TF.Macro)
                        next == ':' && i + 1 < n && text[i + 1] == ':' -> SpanStyle(color = TF.Module)
                        next == '(' -> if (w[0].isUpperCase()) SpanStyle(color = TF.Type) else SpanStyle(color = TF.Func)
                        w[0].isUpperCase() -> SpanStyle(color = TF.Type)
                        else -> null
                    }
                    if (style != null) span(s, if (next == '!' && style.color == TF.Macro) i + 1 else i, style)
                    if (i < n && text[i] == '\'') { span(i, i + 1, SpanStyle(color = TF.Macro)); i++ }  // primes: result'
                    prevSignificant = 'a'
                }
                c == '=' && i + 1 < n && text[i + 1] == '>' -> { span(i, i + 2, SpanStyle(color = TF.Keyword)); i += 2 }
                else -> { if (!c.isWhitespace()) prevSignificant = c; i++ }
            }
        }
        for (d in diagnostics) {
            val (s, e) = range(text, d)
            if (e > s) b.addStyle(SpanStyle(textDecoration = TextDecoration.Underline, background = (if (d.error) TF.Err else TF.Warn).copy(alpha = 0.18f)), s, e)
        }
        return b.toAnnotatedString()
    }

    fun range(text: String, d: Diagnostic): Pair<Int, Int> {
        fun off(line: Int, col: Int): Int {
            var o = 0; var l = 0
            while (l < line) { val nl = text.indexOf('\n', o); if (nl < 0) return text.length; o = nl + 1; l++ }
            val end = text.indexOf('\n', o).let { if (it < 0) text.length else it }
            return (o + col).coerceAtMost(end)
        }
        val s = off(d.line, d.col)
        var e = off(d.endLine, d.endCol)
        if (e <= s) e = (s + 1).coerceAtMost(text.length)
        return s to e
    }
}

class TulpaTransformation(private val diagnostics: List<Diagnostic>, private val enabled: Boolean = true) : VisualTransformation {
    override fun filter(text: AnnotatedString): TransformedText =
        TransformedText(if (enabled) Highlighter.highlight(text.text, diagnostics) else text, OffsetMapping.Identity)

    override fun equals(other: Any?) = other is TulpaTransformation && other.diagnostics == diagnostics && other.enabled == enabled
    override fun hashCode() = diagnostics.hashCode() * 31 + enabled.hashCode()
}

