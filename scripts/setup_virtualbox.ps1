# scripts/setup_virtualbox.ps1
# Automated setup and configuration for Aurora-OS in VirtualBox (x86_64)

param(
    [string]$VmName = "microos",
    [switch]$ForceRecreate
)

$ErrorActionPreference = "Stop"

Write-Host "==========================================================" -ForegroundColor Magenta
Write-Host "  Aurora-OS (microos) VirtualBox x86_64 Environment Setup " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Magenta

# 1. Locate VBoxManage
$vboxPaths = @(
    "C:\Program Files\Oracle\VirtualBox\VBoxManage.exe",
    "C:\Program Files (x86)\Oracle\VirtualBox\VBoxManage.exe",
    (Get-Command VBoxManage.exe -ErrorAction SilentlyContinue).Source
)

$vbox = $vboxPaths | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $vbox) {
    Write-Error "VirtualBox (VBoxManage.exe) was not found in standard paths. Please install VirtualBox."
    exit 1
}
Write-Host "[+] Found VBoxManage: $vbox" -ForegroundColor Green

# 2. Check for required build artifacts
$rootDir = Split-Path -Parent $PSScriptRoot
$vdiPath = Join-Path $rootDir "dist\micro-os.vdi"
$isoPath = Join-Path $rootDir "dist\micro-os.iso"
$rawImgPath = Join-Path $rootDir "dist\micro-os-bios.img"

if (-not (Test-Path $vdiPath)) {
    if (Test-Path $rawImgPath) {
        Write-Host "[*] Converting raw BIOS image to VDI..." -ForegroundColor Yellow
        & $vbox convertfromraw $rawImgPath $vdiPath --format VDI
    } else {
        Write-Error "Neither dist\micro-os.vdi nor dist\micro-os-bios.img found. Run 'cargo build' first."
        exit 1
    }
}
Write-Host "[+] Disk image ready: $vdiPath" -ForegroundColor Green

# 3. Check VM existence
$vms = & $vbox list vms
$vmExists = $vms -match "`"$VmName`""

if ($vmExists -and $ForceRecreate) {
    Write-Host "[*] Force recreating VM '$VmName'..." -ForegroundColor Yellow
    & $vbox controlvm $VmName poweroff 2>$null
    Start-Sleep -Seconds 1
    & $vbox unregistervm $VmName --delete
    $vmExists = $false
}

if (-not $vmExists) {
    Write-Host "[*] Creating new 64-bit VirtualBox VM: $VmName" -ForegroundColor Cyan
    & $vbox createvm --name $VmName --ostype "Other_64" --register
} else {
    Write-Host "[+] Existing VM '$VmName' found. Updating configuration..." -ForegroundColor Yellow
}

# 4. Configure VM settings for 64-bit Long Mode compatibility
Write-Host "[*] Configuring 64-bit Long Mode, PAE, IO-APIC, RAM (256MB), VRAM (16MB)..." -ForegroundColor Cyan
& $vbox modifyvm $VmName `
    --ostype "Other_64" `
    --longmode on `
    --pae on `
    --ioapic on `
    --memory 256 `
    --vram 16 `
    --graphicscontroller vboxvga `
    --mouse ps2 `
    --keyboard ps2 `
    --audio none `
    --boot1 disk `
    --boot2 dvd `
    --boot3 none `
    --boot4 none

# 5. Configure Storage Controller (IDE PIIX4)
$vmInfo = & $vbox showvminfo $VmName --machinereadable
$hasIde = $vmInfo -match 'storagecontrollername\d+="IDE"'
if (-not $hasIde) {
    Write-Host "[*] Adding IDE Storage Controller..." -ForegroundColor Cyan
    & $vbox storagectl $VmName --name "IDE" --add ide --controller PIIX4
}

# 6. Attach Virtual Disk
Write-Host "[*] Attaching disk medium ($vdiPath)..." -ForegroundColor Cyan
try {
    & $vbox storageattach $VmName --storagectl "IDE" --port 0 --device 0 --type hdd --medium $vdiPath
} catch {
    Write-Host "[!] Note: If UUID collision occurs, medium is already registered." -ForegroundColor Yellow
}

# 7. Optionally attach ISO
if (Test-Path $isoPath) {
    Write-Host "[*] Attaching optical ISO medium ($isoPath)..." -ForegroundColor Cyan
    try {
        & $vbox storageattach $VmName --storagectl "IDE" --port 1 --device 0 --type dvddrive --medium $isoPath
    } catch {
        # Ignore if already attached
    }
}

Write-Host "==========================================================" -ForegroundColor Green
Write-Host "  Setup Complete! Aurora-OS VM is ready to run." -ForegroundColor Green
Write-Host "  Start it with: .\scripts\run_virtualbox.ps1" -ForegroundColor Yellow
Write-Host "  Or directly in VirtualBox GUI by selecting 'microos' -> Start." -ForegroundColor Yellow
Write-Host "==========================================================" -ForegroundColor Green
