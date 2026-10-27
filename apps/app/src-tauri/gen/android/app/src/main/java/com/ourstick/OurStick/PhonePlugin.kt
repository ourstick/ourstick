package com.ourstick.OurStick

import android.app.Activity
import android.view.WindowManager
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class SetHandlerArgs {
  lateinit var handler: Channel
}

@InvokeArg
class KeepScreenOnArgs {
  var on: Boolean = false
}

/**
 * What the app's Rust side (`phone.rs`) needs from Android: the links that
 * [LinkActivity] receives, each kept until the window takes it (a link that
 * arrives before the app is ready waits here; never logged), and the screen
 * kept on while a download runs (ADR 0027).
 */
@TauriPlugin
class PhonePlugin(private val activity: Activity) : Plugin(activity) {
  companion object {
    private val lock = Any()
    private var waiting: String? = null
    private var handler: Channel? = null

    fun arrived(link: String) {
      synchronized(lock) {
        val to = handler
        if (to == null) waiting = link else send(to, link)
      }
    }

    private fun send(to: Channel, link: String) {
      val event = JSObject()
      event.put("link", link)
      to.send(event)
    }
  }

  @Command
  fun setHandler(invoke: Invoke) {
    val args = invoke.parseArgs(SetHandlerArgs::class.java)
    synchronized(lock) {
      handler = args.handler
      waiting?.let { send(args.handler, it) }
      waiting = null
    }
    invoke.resolve()
  }

  @Command
  fun keepScreenOn(invoke: Invoke) {
    val args = invoke.parseArgs(KeepScreenOnArgs::class.java)
    activity.runOnUiThread {
      val flag = WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON
      if (args.on) activity.window.addFlags(flag) else activity.window.clearFlags(flag)
    }
    invoke.resolve()
  }
}
