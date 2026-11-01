package com.ourstick.OurStick

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.Build
import androidx.annotation.RequiresApi

/**
 * Keeps OurStick sending while it is not in front (ADR 0029): a
 * user-initiated data transfer job (Android 14+), held while at least one
 * thing is sent. The job does no work of its own; while it runs, the
 * app's process keeps the network and is not frozen, and Rust serves as
 * before. Started only while the app is visible (Android's rule), so a
 * wish made in the background waits for the app to come to the front.
 * Notifications are never asked for: without them Android still runs the
 * job and shows it in its task manager.
 */
class SendingJob : JobService() {
  override fun onStartJob(params: JobParameters): Boolean {
    if (!wanted) return false
    if (Build.VERSION.SDK_INT >= 34) {
      setNotification(params, NOTICE, notice(this), JOB_END_NOTIFICATION_POLICY_REMOVE)
    }
    synchronized(lock) { running = this to params }
    return true
  }

  override fun onStopJob(params: JobParameters): Boolean {
    synchronized(lock) { running = null }
    // Asked again by Android later, if still wanted.
    return wanted
  }

  companion object {
    private const val JOB = 0x5e4d
    private const val NOTICE = 0x5e4d
    private const val CHANNEL = "sending"
    private val lock = Any()
    @Volatile private var wanted = false
    private var running: Pair<SendingJob, JobParameters>? = null

    /** Sending goes on in the background from now on, or no longer;
     * before Android 14, only while the app is in front. */
    fun keep(context: Context, on: Boolean) {
      wanted = on
      if (Build.VERSION.SDK_INT < 34) return
      if (on) start(context) else stop(context)
    }

    /** The app came to the front: a job still wished for starts now. */
    fun resumed(context: Context) {
      if (wanted && Build.VERSION.SDK_INT >= 34) start(context)
    }

    @RequiresApi(34)
    private fun start(context: Context) {
      val jobs = context.getSystemService(JobScheduler::class.java)
      if (jobs.getPendingJob(JOB) != null) return
      val network = NetworkRequest.Builder()
        .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
        .build()
      val job = JobInfo.Builder(JOB, ComponentName(context, SendingJob::class.java))
        .setUserInitiated(true)
        .setRequiredNetwork(network)
        .setEstimatedNetworkBytes(1L shl 30, 1L shl 30)
        .build()
      try {
        jobs.schedule(job)
      } catch (_: RuntimeException) {
        // Not visible right now: started when the app is in front again.
      }
    }

    @RequiresApi(34)
    private fun stop(context: Context) {
      context.getSystemService(JobScheduler::class.java).cancel(JOB)
      synchronized(lock) {
        running?.let { (job, params) -> job.jobFinished(params, false) }
        running = null
      }
    }

    @RequiresApi(34)
    private fun notice(context: Context): Notification {
      val notifications = context.getSystemService(NotificationManager::class.java)
      notifications.createNotificationChannel(
        NotificationChannel(CHANNEL, "Sending", NotificationManager.IMPORTANCE_LOW)
      )
      val open = PendingIntent.getActivity(
        context, 0,
        Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE
      )
      return Notification.Builder(context, CHANNEL)
        .setContentTitle("OurStick is sending")
        .setContentText("It goes on while this phone is on and online.")
        .setSmallIcon(android.R.drawable.stat_sys_upload)
        .setContentIntent(open)
        .setOngoing(true)
        .build()
    }
  }
}
