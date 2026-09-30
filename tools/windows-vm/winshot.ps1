# Screenshot of the VM's desktop session, for checks from the host. QEMU's
# screendump sees nothing once the virtio GPU driver is in; run through
# desktop.ps1 (a locked desktop gives no picture).
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
$bounds = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$bitmap = New-Object System.Drawing.Bitmap $bounds.Width, $bounds.Height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.CopyFromScreen($bounds.Location, [System.Drawing.Point]::Empty, $bounds.Size)
$bitmap.Save("$env:USERPROFILE\winshot.png", [System.Drawing.Imaging.ImageFormat]::Png)
