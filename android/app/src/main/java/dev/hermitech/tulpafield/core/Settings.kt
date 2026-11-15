package dev.hermitech.tulpafield.core

import android.content.Context
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

data class Prefs(
    val fontSize: Int = 14,
    val autosave: Boolean = true,
    val keepScreenOn: Boolean = true,
    val trials: Int = 300,
    val lineNumbers: Boolean = true,
    val pairBrackets: Boolean = true,
    val haptics: Boolean = true,
)

class Settings(context: Context) {
    private val sp = context.getSharedPreferences("settings", Context.MODE_PRIVATE)
    private val _prefs = MutableStateFlow(load())
    val prefs: StateFlow<Prefs> = _prefs

    private fun load() = Prefs(
        fontSize = sp.getInt("fontSize", 14),
        autosave = sp.getBoolean("autosave", true),
        keepScreenOn = sp.getBoolean("keepScreenOn", true),
        trials = sp.getInt("trials", 300),
        lineNumbers = sp.getBoolean("lineNumbers", true),
        pairBrackets = sp.getBoolean("pairBrackets", true),
        haptics = sp.getBoolean("haptics", true),
    )

    fun update(f: (Prefs) -> Prefs) {
        val p = f(_prefs.value)
        _prefs.value = p
        sp.edit()
            .putInt("fontSize", p.fontSize).putBoolean("autosave", p.autosave)
            .putBoolean("keepScreenOn", p.keepScreenOn).putInt("trials", p.trials)
            .putBoolean("lineNumbers", p.lineNumbers).putBoolean("pairBrackets", p.pairBrackets)
            .putBoolean("haptics", p.haptics)
            .apply()
    }

    var lastProject: String?
        get() = sp.getString("lastProject", null)
        set(v) { sp.edit().putString("lastProject", v).apply() }
    var lastFile: String?
        get() = sp.getString("lastFile", null)
        set(v) { sp.edit().putString("lastFile", v).apply() }
    var seenWelcome: Boolean
        get() = sp.getBoolean("seenWelcome", false)
        set(v) { sp.edit().putBoolean("seenWelcome", v).apply() }
}
