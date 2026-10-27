package com.ourstick.OurStick

import android.app.Activity
import android.content.Intent
import android.os.Bundle

/**
 * Where a sharing link opened from another app arrives (ADR 0027, decision
 * 6). It hands the link to [PhonePlugin] and starts the main activity without
 * it, so the recent tasks never keep the link as the main task's intent. It
 * is never shown, never kept in the recent tasks, and gone once it finishes.
 */
class LinkActivity : Activity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    intent?.dataString?.let { PhonePlugin.arrived(it) }
    startActivity(Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    finish()
  }
}
