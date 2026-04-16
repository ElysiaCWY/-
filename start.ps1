$ErrorActionPreference = "Stop"

try {
  $ProjectRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
  Set-Location $ProjectRoot

  Write-Host "== Resume Manager launcher ==" -ForegroundColor Cyan
  Write-Host "Project root: $ProjectRoot"

  # Ensure Node.js and Rust toolchain are available in this session.
  $nodeDir = "C:\Program Files\nodejs"
  $cargoDir = Join-Path $env:USERPROFILE ".cargo\bin"

  if (Test-Path $nodeDir) {
    $env:Path = "$nodeDir;$env:Path"
  }
  if (Test-Path $cargoDir) {
    $env:Path = "$cargoDir;$env:Path"
  }

  if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
    throw "node not found. Please install Node.js LTS."
  }
  if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "cargo not found. Please install Rustup toolchain."
  }

  Write-Host "Node: $(node --version)"
  Write-Host "Cargo: $(cargo --version)"

  if (-not (Test-Path (Join-Path $ProjectRoot "node_modules"))) {
    Write-Host "First run: installing npm dependencies..." -ForegroundColor Yellow
    & npm.cmd install
    if ($LASTEXITCODE -ne 0) {
      throw "npm install failed (exit code: $LASTEXITCODE)"
    }
  }

  $dashboardEntry = Join-Path $ProjectRoot "ui\dashboard.html"
  if (-not (Test-Path $dashboardEntry)) {
    throw "Entry page not found: $dashboardEntry"
  }

  Write-Host "Frontend entry: ui/dashboard.html" -ForegroundColor Cyan
  Write-Host "Starting Tauri dev..." -ForegroundColor Green
  & npm.cmd run tauri:dev
  exit $LASTEXITCODE
}
catch {
  Write-Host ""
  Write-Host "Launch failed: " -ForegroundColor Red -NoNewline
  Write-Host " $($_.Exception.Message)" -ForegroundColor Yellow
  Write-Host "Please share this error output for further debugging." -ForegroundColor Cyan
  exit 1
}

