package com.ourstick.OurStick

import android.app.Activity
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

/**
 * Hands the links that [LinkActivity] receives to the app's Rust side
 * (`receiving::opened`), which keeps each until the window takes it. A link
 * that arrives before the app is ready waits here. Never logged.
 */
@TauriPlugin
class LinkPlugin(activity: Activity) : Plugin(activity) {
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
}
