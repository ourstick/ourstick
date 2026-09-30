# Runs a command in the VM's desktop session and waits for it. Over ssh
# (key login) there is no logon session: the Credential Manager answers
# ERROR_NO_SUCH_LOGON_SESSION, and a subst letter or a window would not
# show on the desktop.
#   powershell -File desktop.ps1 "cargo test -p ourstick-core -- --ignored keyring"
param([Parameter(Mandatory)] [string] $Command)
$log = "$env:USERPROFILE\desktop.log"
Remove-Item -ErrorAction SilentlyContinue $log
$run = "cmd /c cd /d $env:USERPROFILE\ourstick && $Command > $log 2>&1"
schtasks /create /tn OurStickDesktop /tr $run /sc once /st 23:59 /it /ru $env:USERNAME /f | Out-Null
schtasks /run /tn OurStickDesktop | Out-Null
do {
    Start-Sleep 2
    $status = (schtasks /query /tn OurStickDesktop /fo csv /nh).Split(',')[2].Trim('"')
} while ($status -eq 'Running')
schtasks /delete /tn OurStickDesktop /f | Out-Null
Get-Content $log
