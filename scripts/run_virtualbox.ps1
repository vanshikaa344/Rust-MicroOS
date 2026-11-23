# scripts/run_virtualbox.ps1
# Start the Aurora-OS VirtualBox VM

param(
    [string]$VmName = "microos",
    [switch]$Headless
)

$vboxPaths = @(
    "C:\Program Files\Oracle\VirtualBox\VBoxManage.exe",
    "C:\Program Files (x86)\Oracle\VirtualBox\VBoxManage.exe",
    (Get-Command VBoxManage.exe -ErrorAction SilentlyContinue).Source
)

$vbox = $vboxPaths | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $vbox) {
    Write-Error "VirtualBox (VBoxManage.exe) was not found."
    exit 1
}

$startType = if ($Headless) { "headless" } else { "gui" }
Write-Host "Starting '$VmName' ($startType mode)..." -ForegroundColor Cyan
& $vbox startvm $VmName --type $startType
