<#
=============================================================================
 OpenClaw Fleet — enrolador de máquinas WINDOWS

 Enrola ESTA máquina (de tu propiedad/administración) en tu flota:
   1) Instala Tailscale y la une a tu Headscale.
   2) Habilita OpenSSH Server y crea un usuario de automatización con tu clave.
   3) (Opcional) Instala RustDesk apuntando a tu servidor, para control gráfico.
   4) Reporta hostname + IP de malla para que el bot la vea en el inventario.

 Uso típico (one-liner), en PowerShell COMO ADMINISTRADOR:
   irm https://tu-servidor/install.ps1 | iex ; Install-Fleet `
     -Server  https://fleet.tudominio.com `
     -AuthKey <FLEET_AUTHKEY> `
     -SshPubKey "ssh-ed25519 AAAA... bot@control" `
     -RustdeskHost fleet.tudominio.com `
     -RustdeskKey <RUSTDESK_PUBKEY>

 (o descargá el archivo y corré:  .\install.ps1 -Server ... -AuthKey ...)
 Omití -Rustdesk* si no querés control gráfico en esa máquina.
=============================================================================
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory=$true)][string]$Server,
  [Parameter(Mandatory=$true)][string]$AuthKey,
  [string]$SshPubKey = "",
  [string]$RustdeskHost = "",
  [string]$RustdeskKey = "",
  [string]$AutoUser = "fleet"
)

function Install-Fleet {
  param($Server,$AuthKey,$SshPubKey="",$RustdeskHost="",$RustdeskKey="",$AutoUser="fleet")

  $ErrorActionPreference = "Stop"
  function Log($m){ Write-Host ">> $m" -ForegroundColor Cyan }

  if (-not ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()
        ).IsInRole([Security.Principal.WindowsBuiltinRole]::Administrator)) {
    throw "Ejecutá PowerShell como Administrador."
  }

  # --- 1) Tailscale + unión a Headscale -----------------------------------
  $tailscale = "$env:ProgramFiles\Tailscale\tailscale.exe"
  if (-not (Test-Path $tailscale)) {
    Log "Instalando Tailscale..."
    $msi = "$env:TEMP\tailscale.msi"
    Invoke-WebRequest "https://pkgs.tailscale.com/stable/tailscale-setup-latest-amd64.msi" -OutFile $msi
    Start-Process msiexec.exe -ArgumentList "/i `"$msi`" /quiet /norestart" -Wait
  }

  Log "Uniendo la máquina a la malla ($Server)..."
  & $tailscale up --login-server=$Server --authkey=$AuthKey --accept-routes `
      --hostname=$env:COMPUTERNAME --unattended --reset

  # --- 2) OpenSSH Server + usuario de automatización ----------------------
  if ($SshPubKey) {
    Log "Habilitando OpenSSH Server..."
    Add-WindowsCapability -Online -Name OpenSSH.Server~~~~0.0.1.0 | Out-Null
    Set-Service -Name sshd -StartupType Automatic
    Start-Service sshd

    Log "Creando usuario de automatización '$AutoUser'..."
    $pw = -join ((48..57)+(65..90)+(97..122) | Get-Random -Count 20 | ForEach-Object {[char]$_})
    $sec = ConvertTo-SecureString $pw -AsPlainText -Force
    if (-not (Get-LocalUser -Name $AutoUser -ErrorAction SilentlyContinue)) {
      New-LocalUser -Name $AutoUser -Password $sec -PasswordNeverExpires -AccountNeverExpires | Out-Null
    } else {
      Set-LocalUser -Name $AutoUser -Password $sec
    }
    Add-LocalGroupMember -Group "Administrators" -Member $AutoUser -ErrorAction SilentlyContinue

    # Para usuarios admin, OpenSSH usa este archivo especial con ACL estricta.
    $authFile = "$env:ProgramData\ssh\administrators_authorized_keys"
    if (-not (Test-Path $authFile)) { New-Item -ItemType File -Path $authFile -Force | Out-Null }
    if (-not (Select-String -Path $authFile -SimpleMatch $SshPubKey -Quiet)) {
      Add-Content -Path $authFile -Value $SshPubKey
    }
    icacls $authFile /inheritance:r /grant "Administrators:F" /grant "SYSTEM:F" | Out-Null
  }

  # --- 3) RustDesk (control gráfico, opcional) ----------------------------
  $rustdeskPw = ""
  if ($RustdeskHost -and $RustdeskKey) {
    Log "Instalando RustDesk y apuntándolo a tu servidor..."
    $exe = "$env:TEMP\rustdesk.exe"
    Invoke-WebRequest "https://github.com/rustdesk/rustdesk/releases/latest/download/rustdesk-1.4.2-x86_64.exe" -OutFile $exe
    Start-Process $exe -ArgumentList "--silent-install" -Wait
    $rd = "$env:ProgramFiles\RustDesk\rustdesk.exe"
    Start-Sleep 3
    & $rd --config "host=$RustdeskHost,key=$RustdeskKey"
    $rustdeskPw = -join ((48..57)+(65..90)+(97..122) | Get-Random -Count 16 | ForEach-Object {[char]$_})
    & $rd --password $rustdeskPw
  }

  # --- 4) Reporte final ----------------------------------------------------
  Start-Sleep 2
  $meshIp = (& $tailscale ip -4 2>$null | Select-Object -First 1)
  $rdId = ""
  if ($RustdeskHost) { try { $rdId = (& "$env:ProgramFiles\RustDesk\rustdesk.exe" --get-id) } catch {} }

  Write-Host ""
  Write-Host "========================= MÁQUINA ENROLADA =========================" -ForegroundColor Green
  Write-Host " hostname      : $env:COMPUTERNAME"
  Write-Host " IP de malla   : $meshIp"
  if ($SshPubKey)     { Write-Host " usuario auto  : $AutoUser (SSH con clave, es Administrador)" }
  if ($RustdeskHost)  { Write-Host " rustdesk id   : $rdId" }
  if ($rustdeskPw)    { Write-Host " rustdesk pass : $rustdeskPw" }
  Write-Host "===================================================================" -ForegroundColor Green
  Write-Host " Ya aparece en el inventario del bot:  python fleet.py list"
}

# Si se invoca con parámetros (no por pipe), ejecutar directamente.
if ($PSBoundParameters.ContainsKey('Server')) {
  Install-Fleet -Server $Server -AuthKey $AuthKey -SshPubKey $SshPubKey `
    -RustdeskHost $RustdeskHost -RustdeskKey $RustdeskKey -AutoUser $AutoUser
}
