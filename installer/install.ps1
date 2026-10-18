# installer/install.ps1 — M4 (Windows)
# 1) regsvr32 sidecam_vcam.dll (admin sekali) 2) firewall rule privat 3) cleanup uninstall
param([switch]$Uninstall)
$ErrorActionPreference='Stop'
$ruleName='SideCam Receiver 8080'
$dll='sidecam_vcam.dll'
if($Uninstall){
  Write-Host '[SideCam] Uninstall...'
  try{ regsvr32 /u /s $dll }catch{}
  try{ Remove-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue }catch{}
  Write-Host '[SideCam] Uninstall selesai.'
  exit 0
}
Write-Host '[SideCam] Install — butuh admin untuk regsvr32 + firewall...'
if(-not ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)){
  Write-Host 'Jalankan sebagai Administrator (klik kanan Run as administrator).' -ForegroundColor Yellow; exit 1
}
if(Test-Path $dll){ regsvr32 /s $dll; Write-Host "[SideCam] $dll terdaftar" }
else { Write-Host "[SideCam] $dll tidak ditemukan — skip DirectShow (placeholder mode)" -ForegroundColor Yellow }
# firewall privat only
try {
  New-NetFirewallRule -DisplayName $ruleName -Direction Inbound -LocalPort 8080 -Protocol TCP -Action Allow -Profile Private -ErrorAction Stop | Out-Null
  Write-Host '[SideCam] Firewall rule Private 8080 ditambah'
} catch { Write-Host "[SideCam] Firewall rule gagal: $_" -ForegroundColor Yellow }
Write-Host '[SideCam] Selesai. Buka SideCam, HP scan QR di https://IP:8080'
