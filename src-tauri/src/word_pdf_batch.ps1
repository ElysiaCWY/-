# 由 resume-manager 调用：批量 Word -> PDF（需本机安装 Microsoft Word）
param(
  [Parameter(Mandatory = $true)]
  [string]$JsonPath
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

function Emit-Event($obj) {
  Write-Output (ConvertTo-Json $obj -Compress -Depth 8)
}

$raw = Get-Content -LiteralPath $JsonPath -Raw -Encoding UTF8
$items = $raw | ConvertFrom-Json
if ($null -eq $items) { $items = @() }
if ($items -isnot [System.Array]) { $items = @($items) }

# wdExportFormatPDF = 17（ExportAsFixedFormat）；wdFormatPDF = 17（SaveAs2 备用）
$wdExportFormatPDF = 17
$wdFormatPDF = 17

$word = $null
try {
  $word = New-Object -ComObject Word.Application
  $word.Visible = $false
  $word.DisplayAlerts = 0
} catch {
  Emit-Event @{ type = "error"; message = "无法启动 Word.Application：$($_.Exception.Message)" }
  exit 1
}

$total = $items.Count
$idx = 0
$converted = 0
$skipped = 0
$failed = 0

function Save-DocumentAsPdf {
  param($Document, [string]$PdfPath)
  # 优先 ExportAsFixedFormat（导出 PDF 的推荐 API）；失败再尝试 SaveAs2
  try {
    $null = $Document.ExportAsFixedFormat($PdfPath, $wdExportFormatPDF)
    return
  } catch {
    $null = $Document.SaveAs2($PdfPath, $wdFormatPDF)
  }
}

try {
  foreach ($p in $items) {
    $idx++
    $src = [string]$p.src
    $dst = [string]$p.dst
    $leaf = Split-Path -Path $src -Leaf

    if (-not (Test-Path -LiteralPath $src)) {
      Emit-Event @{ type = "fail"; index = $idx; total = $total; name = $leaf; error = "源文件不存在：$src" }
      $failed++
      continue
    }

    if (Test-Path -LiteralPath $dst) {
      Emit-Event @{ type = "skip"; index = $idx; total = $total; name = $leaf }
      $skipped++
      continue
    }

    $parent = Split-Path -Parent $dst
    if ($parent -and -not (Test-Path -LiteralPath $parent)) {
      $null = New-Item -ItemType Directory -Path $parent -Force
    }

    $doc = $null
    try {
      Emit-Event @{ type = "convert"; index = $idx; total = $total; name = $leaf }
      # ConfirmConversions:=False, ReadOnly:=True
      $doc = $word.Documents.Open($src, $false, $true)
      Save-DocumentAsPdf -Document $doc -PdfPath $dst
      $doc.Close($false) | Out-Null
      $doc = $null
      Emit-Event @{ type = "ok"; index = $idx; total = $total; name = $leaf }
      $converted++
    } catch {
      if ($null -ne $doc) {
        try { $doc.Close($false) } catch {}
        $doc = $null
      }
      $msg = $_.Exception.Message
      Emit-Event @{ type = "fail"; index = $idx; total = $total; name = $leaf; error = $msg }
      $failed++
    }
  }
} finally {
  if ($null -ne $word) {
    try { $word.Quit() } catch {}
    try {
      [void][System.Runtime.InteropServices.Marshal]::ReleaseComObject($word)
    } catch {}
    $word = $null
    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
  }
}

Emit-Event @{ type = "done"; converted = $converted; skipped = $skipped; failed = $failed; total = $total }
