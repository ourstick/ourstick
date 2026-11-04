# First logon in the Windows VM (run from autounattend.xml): OpenSSH Server,
# key login only, PowerShell as its shell. The public key comes from
# authorized_key next to this script on the unattended ISO (drive F:).
Start-Transcript -Path C:\setup-ssh.log
Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0
Set-Service -Name sshd -StartupType Automatic
Start-Service sshd
if (-not (Get-NetFirewallRule -Name 'OpenSSH-Server-In-TCP' -ErrorAction SilentlyContinue)) {
    New-NetFirewallRule -Name 'OpenSSH-Server-In-TCP' -DisplayName 'OpenSSH Server (sshd)' `
        -Enabled True -Direction Inbound -Protocol TCP -Action Allow -LocalPort 22 -Profile Any
} else {
    Set-NetFirewallRule -Name 'OpenSSH-Server-In-TCP' -Enabled True -Profile Any
}
$keys = 'C:\ProgramData\ssh\administrators_authorized_keys'
Copy-Item -Path (Join-Path $PSScriptRoot 'authorized_key') -Destination $keys
icacls $keys /inheritance:r /grant 'Administrators:F' /grant 'SYSTEM:F'
$config = 'C:\ProgramData\ssh\sshd_config'
# Before the first Match block: lines after it apply only to that block
# (Windows' own file ends with "Match Group administrators").
$lines = Get-Content -Path $config
$match = ($lines | Select-String -Pattern '^\s*Match\s' | Select-Object -First 1).LineNumber
$ours = @('PasswordAuthentication no', 'KbdInteractiveAuthentication no')
if ($match) {
    $lines = $lines[0..($match - 2)] + $ours + $lines[($match - 1)..($lines.Count - 1)]
} else {
    $lines = $lines + $ours
}
Set-Content -Path $config -Encoding ascii -Value $lines
New-ItemProperty -Path 'HKLM:\SOFTWARE\OpenSSH' -Name DefaultShell `
    -Value 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe' -PropertyType String -Force
# The VM runs unattended: never turn the display off or sleep (a locked
# desktop has no screenshots and no subst).
powercfg /change monitor-timeout-ac 0
powercfg /change standby-timeout-ac 0
Restart-Service sshd
Stop-Transcript
