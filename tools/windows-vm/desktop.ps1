# Runs a command in the VM's desktop session and waits for it. Over ssh
# (key login) there is no logon session: the Credential Manager answers
# ERROR_NO_SUCH_LOGON_SESSION, and a subst letter or a window would not
# show on the desktop.
#   powershell -ExecutionPolicy Bypass -File desktop.ps1 "cargo test -p ourstick-core -- --ignored keyring"
# The task line must stay under 261 characters (schtasks): put a longer
# command in a .bat file. Each run has its own log, since a program started
# from it (`start ...`) keeps its log open; the run is over when its
# `.done` file appears (a task not started yet reads "Ready", like one
# that has finished).
param([Parameter(Mandatory)] [string] $Command, [int] $Wait = 3600)
$tag = [guid]::NewGuid().ToString("N").Substring(0, 8)
$log = "$env:USERPROFILE\d-$tag.log"
$done = "$env:USERPROFILE\d-$tag.done"
$run = "cmd /c cd /d $env:USERPROFILE\src\ourstick && $Command > $log 2>&1 & echo.> $done"
schtasks /create /tn OurStickDesktop /tr $run /sc once /st 23:59 /it /ru $env:USERNAME /f | Out-Null
schtasks /run /tn OurStickDesktop | Out-Null
$seconds = 0
while (-not (Test-Path $done) -and $seconds -lt $Wait) {
    Start-Sleep 1
    $seconds++
}
schtasks /delete /tn OurStickDesktop /f | Out-Null
Get-Content -ErrorAction SilentlyContinue $log
Remove-Item -ErrorAction SilentlyContinue $log, $done
