package dev.hermitech.tulpafield.ui

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

object TF {
    val Bg = Color(0xFF070A14)
    val Surface = Color(0xFF0E1324)
    val Surface2 = Color(0xFF151B33)
    val Line = Color(0xFF232B4A)
    val Violet = Color(0xFF7C5CFF)
    val Cyan = Color(0xFF3FD0FF)
    val Text = Color(0xFFE6E9F5)
    val Dim = Color(0xFF8A93B8)
    val Ok = Color(0xFF4ADE80)
    val Warn = Color(0xFFFFB547)
    val Err = Color(0xFFFF5C7A)

    // syntax
    val Keyword = Color(0xFFB79CFF)
    val Str = Color(0xFF9BE59B)
    val Num = Color(0xFFFFB86C)
    val Unit = Color(0xFF3FD0FF)
    val Comment = Color(0xFF5D6790)
    val Func = Color(0xFF82AAFF)
    val Type = Color(0xFFFFD580)
    val Macro = Color(0xFFFF7EB6)
    val Module = Color(0xFF5EEAD4)

    val series = listOf(Cyan, Violet, Color(0xFFFF7EB6), Color(0xFF4ADE80), Color(0xFFFFB547), Color(0xFF82AAFF))

    val Mono = FontFamily.Monospace
}

private val scheme = darkColorScheme(
    primary = TF.Violet, onPrimary = Color.White,
    secondary = TF.Cyan, onSecondary = Color(0xFF00121A),
    tertiary = Color(0xFFFF7EB6),
    background = TF.Bg, onBackground = TF.Text,
    surface = TF.Surface, onSurface = TF.Text,
    surfaceVariant = TF.Surface2, onSurfaceVariant = TF.Dim,
    surfaceContainer = TF.Surface, surfaceContainerHigh = TF.Surface2, surfaceContainerHighest = TF.Surface2,
    surfaceContainerLow = TF.Surface, surfaceContainerLowest = TF.Bg,
    outline = TF.Line, outlineVariant = TF.Line,
    error = TF.Err,
    primaryContainer = Color(0xFF2A1F66), onPrimaryContainer = Color(0xFFE2DAFF),
    secondaryContainer = Color(0xFF0D3446), onSecondaryContainer = Color(0xFFCDEFFF),
)

@Composable
fun TulpaTheme(content: @Composable () -> Unit) {
    val base = Typography()
    MaterialTheme(
        colorScheme = scheme,
        typography = base.copy(
            titleLarge = base.titleLarge.copy(fontWeight = FontWeight.SemiBold, letterSpacing = 0.2.sp),
        ),
        content = content,
    )
}

fun monoStyle(size: Int) = TextStyle(fontFamily = TF.Mono, fontSize = size.sp, lineHeight = (size * 1.45f).sp, color = TF.Text)
