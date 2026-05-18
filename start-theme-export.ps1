# Theme-export launcher. ASCII-only so Windows PowerShell 5.1 parses reliably.
$ErrorActionPreference = "Stop"

function Find-DirWithFile($startDir, $fileName, $maxDepth) {
  $current = $startDir
  for ($i = 0; $i -le $maxDepth; $i++) {
    $p = Join-Path $current $fileName
    if (Test-Path $p) { return $current }
    $parent = Split-Path -Parent $current
    if ($parent -eq $current) { break }
    $current = $parent
  }
  return $null
}

try {
  $ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
  Set-Location $ScriptDir

  Write-Host "== Resume Manager theme export launcher ==" -ForegroundColor Cyan
  Write-Host "Script folder: $ScriptDir"

  # package.json may live here or a parent folder (mis-placed script)
  $ProjectRoot = Find-DirWithFile $ScriptDir "package.json" 5
  if (-not $ProjectRoot) {
    Write-Host ""
    Write-Host "ERROR: package.json not found (checked this folder and up to 5 parent levels)." -ForegroundColor Red
    Write-Host ""
    Write-Host "Theme PDF needs npm dependencies. Put these in ONE folder together:" -ForegroundColor Yellow
    Write-Host "  - package.json"
    Write-Host "  - package-lock.json   (recommended)"
    Write-Host "  - app-config.json"
    Write-Host "  - resume-manager.exe"
    Write-Host "  - start-theme-export.bat / start-theme-export.ps1"
    Write-Host ""
    Write-Host "Copy them from the full project zip from the developer, not only the exe."
    Write-Host ""
    exit 1
  }

  Set-Location $ProjectRoot
  Write-Host "Project root (has package.json): $ProjectRoot" -ForegroundColor Green

  $nodeDir = "C:\Program Files\nodejs"
  if (Test-Path $nodeDir) {
    $env:Path = "$nodeDir;$env:Path"
  }

  if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    throw "Node.js not found. Install Node.js LTS and re-run."
  }
  if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
    throw "npm not found. Reinstall Node.js LTS."
  }

  Write-Host "Node: $(node --version)"
  Write-Host "npm: $(npm --version)"

  $resumeCli = Join-Path $ProjectRoot 'node_modules\.bin\resume.cmd'
  if (-not (Test-Path $resumeCli)) {
    Write-Host "First run: npm install (theme PDF needs resume-cli)..." -ForegroundColor Yellow
    & npm.cmd install
    if ($LASTEXITCODE -ne 0) {
      throw "npm install failed (exit code: $LASTEXITCODE)"
    }
  }

  if (-not (Test-Path $resumeCli)) {
    throw "resume-cli missing: node_modules\.bin\resume.cmd. Theme PDF export unavailable."
  }

  Write-Host "resume-cli: $(& $resumeCli --version)" -ForegroundColor Green

  $exeCandidates = @(
    (Join-Path $ProjectRoot "resume-manager.exe"),
    (Join-Path $ProjectRoot 'src-tauri\target\release\resume-manager.exe')
  )
  $appExe = $exeCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1
  if (-not $appExe) {
    throw "resume-manager.exe not found under project root. Put exe next to package.json, or build first."
  }

  Write-Host "Launching: $appExe" -ForegroundColor Green
  Start-Process -FilePath $appExe -WorkingDirectory $ProjectRoot | Out-Null
  Write-Host "Started. Theme PDF export should work if resume-cli runs OK." -ForegroundColor Cyan
  exit 0
}
catch {
  Write-Host ""
  Write-Host "Failed: " -ForegroundColor Red -NoNewline
  Write-Host $_.Exception.Message -ForegroundColor Yellow
  exit 1
}
