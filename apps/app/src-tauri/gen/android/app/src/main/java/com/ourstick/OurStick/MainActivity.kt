package com.ourstick.OurStick

import android.os.Bundle
import android.view.View
import androidx.activity.enableEdgeToEdge
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    // The system's bars, a display cutout and the keyboard keep their room:
    // the web view never draws under them (ADR 0027). Done here, as Android
    // 11's WebView gives `env(safe-area-inset-*)` as 0.
    val content = findViewById<View>(android.R.id.content)
    ViewCompat.setOnApplyWindowInsetsListener(content) { view, insets ->
      val kept = insets.getInsets(
        WindowInsetsCompat.Type.systemBars() or
          WindowInsetsCompat.Type.displayCutout() or
          WindowInsetsCompat.Type.ime()
      )
      view.setPadding(kept.left, kept.top, kept.right, kept.bottom)
      WindowInsetsCompat.CONSUMED
    }
  }
}
