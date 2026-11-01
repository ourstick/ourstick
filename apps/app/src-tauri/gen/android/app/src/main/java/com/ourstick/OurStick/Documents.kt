package com.ourstick.OurStick

import android.content.ContentResolver
import android.database.Cursor
import android.net.Uri
import android.os.CancellationSignal
import android.os.ParcelFileDescriptor
import android.system.Os
import android.system.OsConstants
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import android.provider.OpenableColumns
import java.io.FileInputStream
import java.io.FileNotFoundException
import java.nio.ByteBuffer
import java.nio.channels.FileChannel
import java.io.IOException
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

/**
 * The files the user chose with the system's pickers, read where they are
 * (ADR 0029), for the app's Rust side (`documents/`), which calls these
 * over JNI from its own threads. A chosen file or folder is named by its
 * content URI; a file stays open here and Rust asks for its bytes at a
 * position. Nothing here logs a URI or a name.
 *
 * Facts are `[kind, size, last modified in ms]`: kind 0 a file, 1 a
 * folder, 2 anything else; a size or time the provider does not tell is -1.
 */
object Documents {
  private const val FILE = 0L
  private const val FOLDER = 1L
  private const val OTHER = 2L
  private const val UNKNOWN = -1L

  @Volatile private var resolver: ContentResolver? = null
  private val opened = ConcurrentHashMap<Int, Opened>()
  private val next = AtomicInteger(1)
  /** Cancels an open that takes too long (a cloud file being fetched). */
  private val timer = Executors.newSingleThreadScheduledExecutor()
  private const val OPEN_SECONDS = 30L

  private class Opened(val uri: Uri, val file: ParcelFileDescriptor, val channel: FileChannel)

  /** Set once by [PhonePlugin] when the app starts. */
  fun start(resolver: ContentResolver) {
    this.resolver = resolver
  }

  private fun resolver(): ContentResolver =
    resolver ?: throw IllegalStateException("the app has not started")

  /** A tree URI stands for its top folder's document. */
  private fun document(text: String): Uri {
    val uri = Uri.parse(text)
    if (DocumentsContract.isTreeUri(uri) && !isDocument(uri)) {
      return DocumentsContract.buildDocumentUriUsingTree(
        uri, DocumentsContract.getTreeDocumentId(uri)
      )
    }
    return uri
  }

  private fun isDocument(uri: Uri): Boolean =
    uri.pathSegments.size >= 4 && uri.pathSegments[2] == "document"

  private fun kind(mime: String?, flags: Int): Long = when {
    mime == Document.MIME_TYPE_DIR -> FOLDER
    flags and Document.FLAG_VIRTUAL_DOCUMENT != 0 -> OTHER
    else -> FILE
  }

  private fun long(cursor: Cursor, column: Int): Long =
    if (column < 0 || cursor.isNull(column)) UNKNOWN else cursor.getLong(column)

  /** The facts of the file or folder at `uri`. Every column is looked up
   * by name: the photo picker's provider has fewer than a documents one. */
  @JvmStatic
  fun facts(uri: String): LongArray {
    val document = document(uri)
    val cursor = resolver().query(document, null, null, null, null)
      ?: throw FileNotFoundException("not found")
    cursor.use {
      if (!it.moveToFirst()) throw FileNotFoundException("not found")
      val mimeAt = it.getColumnIndex(Document.COLUMN_MIME_TYPE)
      val mime = if (mimeAt >= 0) it.getString(mimeAt) else resolver().getType(document)
      val flagsAt = it.getColumnIndex(Document.COLUMN_FLAGS)
      val flags = if (flagsAt < 0 || it.isNull(flagsAt)) 0 else it.getInt(flagsAt)
      val kind = kind(mime, flags)
      return longArrayOf(
        kind,
        if (kind == FILE) long(it, it.getColumnIndex(OpenableColumns.SIZE)) else 0L,
        long(it, it.getColumnIndex(Document.COLUMN_LAST_MODIFIED))
      )
    }
  }

  /**
   * The folder at `uri`'s entries, five strings each: name, URI, kind,
   * size, last modified (as in [facts]).
   */
  @JvmStatic
  fun entries(uri: String): Array<String> {
    val folder = document(uri)
    val children = DocumentsContract.buildChildDocumentsUriUsingTree(
      folder, DocumentsContract.getDocumentId(folder)
    )
    val columns = arrayOf(
      Document.COLUMN_DISPLAY_NAME, Document.COLUMN_DOCUMENT_ID, Document.COLUMN_MIME_TYPE,
      Document.COLUMN_FLAGS, Document.COLUMN_SIZE, Document.COLUMN_LAST_MODIFIED
    )
    val found = ArrayList<String>()
    val cursor = resolver().query(children, columns, null, null, null)
      ?: throw FileNotFoundException("not found")
    cursor.use {
      while (it.moveToNext()) {
        val name = it.getString(0) ?: continue
        val id = it.getString(1) ?: continue
        val kind = kind(it.getString(2), if (it.isNull(3)) 0 else it.getInt(3))
        found.add(name)
        found.add(DocumentsContract.buildDocumentUriUsingTree(folder, id).toString())
        found.add(kind.toString())
        found.add((if (kind == FILE) long(it, 4) else 0L).toString())
        found.add(long(it, 5).toString())
      }
    }
    return found.toTypedArray()
  }

  /**
   * Opens the file at `uri` for reading; its handle. Only a file on the
   * phone: one its provider would fetch for longer than [OPEN_SECONDS], or
   * that is not a regular file (a cloud provider's pipe), is refused.
   */
  @JvmStatic
  fun open(uri: String): Int {
    val document = document(uri)
    val signal = CancellationSignal()
    val cancel = timer.schedule({ signal.cancel() }, OPEN_SECONDS, TimeUnit.SECONDS)
    val opening: ParcelFileDescriptor?
    try {
      opening = resolver().openFileDescriptor(document, "r", signal)
    } finally {
      cancel.cancel(false)
    }
    val file = opening ?: throw FileNotFoundException("not found")
    try {
      if (!OsConstants.S_ISREG(Os.fstat(file.fileDescriptor).st_mode)) {
        throw IOException("not a file on this phone")
      }
      val channel = FileInputStream(file.fileDescriptor).channel
      val handle = next.getAndIncrement()
      opened[handle] = Opened(document, file, channel)
      return handle
    } catch (error: Exception) {
      file.close()
      throw error
    }
  }

  /** Fills `buffer` from `position` of the file `handle` as far as it can;
   * how many bytes, 0 at the end. */
  @JvmStatic
  fun readAt(handle: Int, position: Long, buffer: ByteArray): Int {
    val file = opened[handle] ?: throw IllegalStateException("not open")
    val into = ByteBuffer.wrap(buffer)
    while (into.hasRemaining()) {
      val n = file.channel.read(into, position + into.position())
      if (n < 0) break
    }
    return into.position()
  }

  /** The facts of the open file `handle`, from its provider as [facts]. */
  @JvmStatic
  fun openFacts(handle: Int): LongArray {
    val file = opened[handle] ?: throw IllegalStateException("not open")
    return facts(file.uri.toString())
  }

  @JvmStatic
  fun close(handle: Int) {
    opened.remove(handle)?.let {
      it.channel.close()
      it.file.close()
    }
  }
}
