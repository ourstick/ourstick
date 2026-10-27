package com.ourstick.OurStick

import android.app.Activity
import android.app.DownloadManager
import android.content.ActivityNotFoundException
import android.content.Intent
import android.os.Environment
import android.provider.DocumentsContract
import android.view.WindowManager
import java.io.File
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

@InvokeArg
class OpenFolderArgs {
  lateinit var path: String
}

/**
 * What the app's Rust side (`phone.rs`) needs from Android: the links that
 * [LinkActivity] receives, each kept until the window takes it (a link that
 * arrives before the app is ready waits here; never logged), the screen
 * kept on while a download runs, and a received folder opened in Files
 * (ADR 0027).
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

  /** A folder of the shared storage (`Download/OurStick/…`) in Files; the
   * Downloads app where nothing opens a folder. */
  @Command
  fun openFolder(invoke: Invoke) {
    val args = invoke.parseArgs(OpenFolderArgs::class.java)
    val root = Environment.getExternalStorageDirectory().canonicalPath + "/"
    val folder = File(args.path).canonicalPath
    if (!folder.startsWith(root)) {
      invoke.reject("not in the phone's shared storage")
      return
    }
    val uri = DocumentsContract.buildDocumentUri(
      "com.android.externalstorage.documents",
      "primary:" + folder.removePrefix(root)
    )
    val files = Intent(Intent.ACTION_VIEW)
      .setDataAndType(uri, DocumentsContract.Document.MIME_TYPE_DIR)
      .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    try {
      activity.startActivity(files)
    } catch (_: ActivityNotFoundException) {
      activity.startActivity(
        Intent(DownloadManager.ACTION_VIEW_DOWNLOADS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
      )
    }
    invoke.resolve()
  }
}
