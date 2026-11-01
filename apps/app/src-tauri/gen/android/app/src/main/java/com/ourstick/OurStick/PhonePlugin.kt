package com.ourstick.OurStick

import android.app.Activity
import android.app.DownloadManager
import android.content.ActivityNotFoundException
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.PersistableBundle
import android.os.ext.SdkExtensions
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.view.WindowManager
import androidx.activity.result.ActivityResult
import java.io.File
import app.tauri.annotation.ActivityCallback
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSArray
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

@InvokeArg
class LinkArgs {
  lateinit var link: String
}

@InvokeArg
class ChooseArgs {
  /** "photos", "files" or "folders". */
  var pick: String = "files"
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
  init {
    Documents.start(activity.applicationContext.contentResolver)
  }

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

  /**
   * The system's picker for what to send (ADR 0029): the photo picker,
   * files, or one folder. What is chosen is kept readable after a restart;
   * the answer is each one's URI and name, none if the user went back.
   */
  @Command
  fun choose(invoke: Invoke) {
    val args = invoke.parseArgs(ChooseArgs::class.java)
    val intent = when (args.pick) {
      "photos" -> if (hasPhotoPicker()) {
        Intent(MediaStore.ACTION_PICK_IMAGES)
          .putExtra(MediaStore.EXTRA_PICK_IMAGES_MAX, MediaStore.getPickImagesMaxLimit())
      } else {
        // Its grants can be kept, unlike ACTION_GET_CONTENT's.
        Intent(Intent.ACTION_OPEN_DOCUMENT)
          .addCategory(Intent.CATEGORY_OPENABLE)
          .setType("*/*")
          .putExtra(Intent.EXTRA_MIME_TYPES, arrayOf("image/*", "video/*"))
          .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
      }
      "folders" -> Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)
      else -> Intent(Intent.ACTION_OPEN_DOCUMENT)
        .addCategory(Intent.CATEGORY_OPENABLE)
        .setType("*/*")
        .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
    }
    startActivityForResult(invoke, intent, "chosen")
  }

  private fun hasPhotoPicker(): Boolean =
    Build.VERSION.SDK_INT >= 33 ||
      (Build.VERSION.SDK_INT >= 30 &&
        SdkExtensions.getExtensionVersion(Build.VERSION_CODES.R) >= 2)

  @ActivityCallback
  fun chosen(invoke: Invoke, result: ActivityResult) {
    val found = JSArray()
    val data = result.data
    if (result.resultCode == Activity.RESULT_OK && data != null) {
      val uris = ArrayList<Uri>()
      val clip = data.clipData
      if (clip != null) {
        for (index in 0 until clip.itemCount) uris.add(clip.getItemAt(index).uri)
      } else {
        data.data?.let { uris.add(it) }
      }
      val resolver = activity.contentResolver
      for (uri in uris) {
        try {
          resolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
        } catch (_: SecurityException) {
          // Readable while the app runs only; told as not found later.
        }
        val one = JSObject()
        one.put("uri", uri.toString())
        one.put("name", nameOf(uri) ?: "Unnamed")
        found.put(one)
      }
    }
    val answer = JSObject()
    answer.put("chosen", found)
    invoke.resolve(answer)
  }

  /** The name the provider shows for `uri`; a tree's, its top folder's. */
  private fun nameOf(uri: Uri): String? {
    val document = if (DocumentsContract.isTreeUri(uri)) {
      DocumentsContract.buildDocumentUriUsingTree(uri, DocumentsContract.getTreeDocumentId(uri))
    } else {
      uri
    }
    return try {
      activity.contentResolver
        .query(document, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
        ?.use { if (it.moveToFirst()) it.getString(0) else null }
    } catch (_: Exception) {
      null
    }
  }

  /** The share sheet, with the link as text: it goes straight to a chat
   * (ADR 0029). Never logged. */
  @Command
  fun shareLink(invoke: Invoke) {
    val args = invoke.parseArgs(LinkArgs::class.java)
    val send = Intent(Intent.ACTION_SEND)
      .setType("text/plain")
      .putExtra(Intent.EXTRA_TEXT, args.link)
    activity.startActivity(Intent.createChooser(send, null))
    invoke.resolve()
  }

  /** The link on the clipboard, marked sensitive: Android's clipboard
   * preview shows dots instead (the key works before Android 13 too). */
  @Command
  fun copyLink(invoke: Invoke) {
    val args = invoke.parseArgs(LinkArgs::class.java)
    val clip = ClipData.newPlainText("OurStick link", args.link)
    // Extras exist from Android 8; the key is read from Android 13.
    if (Build.VERSION.SDK_INT >= 26) {
      clip.description.extras = PersistableBundle().apply {
        putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
      }
    }
    val clipboard = activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
    clipboard.setPrimaryClip(clip)
    invoke.resolve()
  }
}
