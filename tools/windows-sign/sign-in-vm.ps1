# Signs one file for Tauri's bundle.windows.signCommand (see
# apps/app/src-tauri/tauri.windows-sign.conf.json): the signing key is on a
# hardware token at the Linux host, so the file goes into a queue here, and
# tools/windows-sign/sign-on-host.sh, running in the host's terminal, takes it
# over ssh, signs it and puts it back. Tauri calls this for ourstick.exe, the
# NSIS plugins, the uninstaller (through !uninstfinalize) and the installer.
#
# Windows is the judge: the signed file is taken only if Authenticode calls it
# Valid, from the expected signer, with a timestamp, and the host signed the
# very bytes it was given.
#
# ASCII only: Windows PowerShell 5.1 misreads anything else in a file
# without a byte order mark.
param(
  [Parameter(Mandatory = $true)][string]$File,
  [string]$Signer = 'CN=Georgios Kakavelakis'
)
$ErrorActionPreference = 'Stop'

$queue = Join-Path $env:USERPROFILE 'ourstick-sign-queue'
New-Item -ItemType Directory -Force -Path $queue | Out-Null
$id = '{0:yyyyMMdd-HHmmss}-{1}' -f (Get-Date), [guid]::NewGuid().ToString('N').Substring(0, 8)
$job = Join-Path $queue $id
New-Item -ItemType Directory -Path $job | Out-Null

$name = Split-Path -Leaf $File
$unsigned = Join-Path $job 'unsigned.bin'
Copy-Item -LiteralPath $File -Destination $unsigned
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $unsigned).Hash.ToLower()
Set-Content -LiteralPath (Join-Path $job 'name.txt') -Value $name -Encoding ascii
# Last: the host takes only a job whose 'ready' exists.
New-Item -ItemType File -Path (Join-Path $job 'ready') | Out-Null

$done = Join-Path $job 'done'
$failed = Join-Path $job 'failed'
$deadline = (Get-Date).AddMinutes(30)
while (-not (Test-Path -LiteralPath $done)) {
  if (Test-Path -LiteralPath $failed) {
    throw "the host did not sign ${name}: $(Get-Content -Raw -LiteralPath $failed)"
  }
  if ((Get-Date) -gt $deadline) {
    throw "no signature for $name from the host in 30 minutes: is tools/windows-sign/sign-on-host.sh running there?"
  }
  Start-Sleep -Seconds 1
}

$signedHash = (Get-Content -Raw -LiteralPath $done).Trim()
if ($signedHash -ne $hash) {
  throw "the host signed other bytes than $name ($signedHash, not $hash)"
}
$signed = Join-Path $job 'signed.bin'
$signature = Get-AuthenticodeSignature -LiteralPath $signed
if ($signature.Status -ne 'Valid') {
  throw "the signature of $name is $($signature.Status): $($signature.StatusMessage)"
}
if (-not $signature.SignerCertificate.Subject.StartsWith("$Signer,")) {
  throw "$name is signed by $($signature.SignerCertificate.Subject), not $Signer"
}
if ($null -eq $signature.TimeStamperCertificate) {
  throw "the signature of $name has no timestamp"
}
Copy-Item -LiteralPath $signed -Destination $File -Force
Remove-Item -Recurse -Force -LiteralPath $job
Write-Output "signed $name by $($signature.SignerCertificate.Subject), timestamp by $($signature.TimeStamperCertificate.Subject)"
