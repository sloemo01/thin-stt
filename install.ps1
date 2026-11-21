# Thin STT installer — run via:
#   curl -L https://raw.githubusercontent.com/sloemo01/thin-stt/master/install.ps1 -o $env:TEMP\thinstt.ps1 ; powershell -ExecutionPolicy Bypass -File $env:TEMP\thinstt.ps1
$ErrorActionPreference = 'Stop'

$app = Join-Path $env:USERPROFILE 'thin-stt'
$zip = Join-Path $env:TEMP 'thinstt.zip'

Write-Host "Downloading thin-stt..."
Invoke-WebRequest 'https://github.com/sloemo01/thin-stt/archive/refs/heads/master.zip' -OutFile $zip

Write-Host "Installing to $app ..."
Expand-Archive $zip $env:TEMP -Force
if (Test-Path $app) { Remove-Item $app -Recurse -Force }
Move-Item "$env:TEMP\thin-stt-master" $app

# find python
$py = Get-Command python -ErrorAction SilentlyContinue
if (-not $py) { $py = Get-Command py -ErrorAction SilentlyContinue }
if (-not $py) { throw "Python not found. Install Python 3.10-3.12 from https://python.org first." }

Write-Host "Creating venv + installing deps (first run downloads ~300MB)..."
& $py.Source -m venv "$app\.venv"
& "$app\.venv\Scripts\pip.exe" install --quiet --upgrade pip
& "$app\.venv\Scripts\pip.exe" install --quiet -r "$app\requirements.txt"

# desktop + start-menu shortcut
$ws = New-Object -ComObject WScript.Shell
foreach ($lnk in @("$env:USERPROFILE\Desktop\Thin STT.lnk",
                   "$env:APPDATA\Microsoft\Windows\Start Menu\Programs\Thin STT.lnk")) {
    $sc = $ws.CreateShortcut($lnk)
    $sc.TargetPath = "$app\.venv\Scripts\pythonw.exe"
    $sc.Arguments = '-m thin_stt'
    $sc.WorkingDirectory = $app
    $sc.Save()
}

Write-Host "Done. Launching Thin STT (also on your Desktop as 'Thin STT')."
Start-Process "$app\.venv\Scripts\pythonw.exe" -ArgumentList '-m','thin_stt' -WorkingDirectory $app
